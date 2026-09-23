//! `POST /api/v2/custos/authorize` and `POST /api/v2/custos/authorize/batch`
//! (EVAL-1, EVAL-4, AVAIL-1): any authenticated principal asks about its
//! own authority and gets `allow`, `deny` or `not_found` per target, with no
//! reason or evidence. Every evaluation failure is an opaque 503 for a
//! single target; in a batch an unavailable target is `not_found`.
//!
//! Targets are real `custos.groups` and `custos.grants_v2` rows: the Custos
//! provider answers existence from storage.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_api::dtos::authorization::{
    AuthorizeBatchRequest, AuthorizeBatchResult, AuthorizeDecision, AuthorizeRequest,
};
use atlas_client::{AtlasClient, ClientError};
use atlas_core::capabilities::{CapabilityError, ProviderCatalog, ResourceFacts, ResourceProvider};
use atlas_core::ids::{ActionId, PrincipalSetId, ResourceRef};
use atlas_custos::WorkspaceScope;
use atlas_custos::authorize::{AuthorizationService, AuthorizationSettings, ProviderSet};
use atlas_custos::capability::Capability;
use atlas_custos::entities::authorization::{
    DenyRuleId, GrantAuthority, GrantId, NewDenyRecord, NewGrantRecord, SubjectRecord, TargetRecord,
};
use atlas_custos::entities::groups::NewGroup;
use atlas_custos::entities::identity::{ApiKeyType, NewApiKey};
use atlas_custos::eval::DenyMode;
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::authorization::{DenyRuleRepo, GrantV2Repo};
use atlas_custos::ports::group_repo::GroupRepo;
use atlas_custos_postgres::repos::authorization::{PgDenyRuleRepo, PgGrantV2Repo};
use atlas_custos_postgres::repos::authorize::{PgAuthorizationFactsStore, PgGroupMembershipSource};
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo};
use atlas_custos_postgres::repos::permissions::PgGroupRepo;
use atlas_server::auth::tokens::{generate_api_key, hash_token};
use atlas_server::authz::v2_service::{TokioSleeper, validation_catalog};
use atlas_server::config::DenyModeConfig;
use atlas_server::state::AppState;
use sea_orm::{DatabaseBackend, FromQueryResult, Statement};

const GROUP_READ: &str = "custos::group::read";
const GROUP_UPDATE: &str = "custos::group::update";
const GRANT_READ: &str = "custos::grant::read";
const GRANT_DELETE: &str = "custos::grant::delete";

fn action(raw: &str) -> ActionId {
    raw.parse().expect("valid action id")
}

fn group_ref(group_id: uuid::Uuid) -> String {
    format!("custos::group::{group_id}")
}

/// A real `custos.groups` row, the resource the Custos provider reports as
/// existing.
async fn seed_group(db: &support::TestDb, name: &str) -> uuid::Uuid {
    let (ws, owner) = support::seed_workspace(db, &format!("{name}-owner")).await;

    PgGroupRepo {
        conn: db.conn().clone(),
    }
    .create(NewGroup {
        workspace_id: WorkspaceScope(ws.id.0),
        name: name.to_string(),
        created_by: owner.id,
    })
    .await
    .expect("seed group")
    .id
    .0
}

/// Seeds a grant of explicit `actions` on `target` for `principal`, the way
/// a platform admin would have created it; returns the grant row's id.
async fn seed_grant(
    db: &support::TestDb,
    principal: PrincipalId,
    target: &str,
    actions: &[&str],
) -> uuid::Uuid {
    PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .create(NewGrantRecord {
        id: GrantId::new(),
        subject: SubjectRecord::Principal(principal),
        target: TargetRecord::Ref(target.parse().expect("valid target ref")),
        authority: GrantAuthority::Actions(actions.iter().map(|raw| action(raw)).collect()),
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed grant")
    .id
    .0
}

async fn seed_deny(db: &support::TestDb, principal: PrincipalId, target: &str, actions: &[&str]) {
    PgDenyRuleRepo {
        conn: db.conn().clone(),
    }
    .create(NewDenyRecord {
        id: DenyRuleId::new(),
        subject: SubjectRecord::Principal(principal),
        target: TargetRecord::Ref(target.parse().expect("valid target ref")),
        actions: actions.iter().map(|raw| action(raw)).collect(),
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed deny");
}

#[derive(Debug, FromQueryResult)]
struct KeyPrincipal {
    principal_id: uuid::Uuid,
}

/// An agent API key with `scopes`, presented as a bearer token, and the
/// agent principal it acts as.
async fn api_key_client(
    server: &support::TestServer,
    db: &support::TestDb,
    scopes: Vec<Capability>,
) -> (AtlasClient, PrincipalId) {
    let (ws, owner) = support::seed_workspace(db, "authorize-key-owner").await;
    let raw_token = generate_api_key();
    let ctx = support::ctx(&ws, &owner);

    let key = PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        WorkspaceScope(ctx.workspace_id.0),
        &ctx.actor,
        NewApiKey {
            name: "authorize-key".to_string(),
            token_hash: hash_token(&raw_token),
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes,
        },
    )
    .await
    .expect("create api key");

    let principal = KeyPrincipal::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT principal_id FROM custos.api_keys WHERE id = '{}'",
            key.id.0
        ),
    ))
    .one(db.conn())
    .await
    .expect("query key principal")
    .expect("key row")
    .principal_id;

    (
        AtlasClient::new(server.base_url().to_string()).with_token(raw_token),
        PrincipalId(principal),
    )
}

fn problem_of(err: ClientError) -> atlas_api::problem::ProblemDetails {
    match err {
        ClientError::Api(problem) => problem,
        other => panic!("expected an API problem, got {other:?}"),
    }
}

fn question(action_raw: &str, target: &str) -> AuthorizeRequest {
    AuthorizeRequest {
        action: action_raw.to_string(),
        target: target.to_string(),
    }
}

async fn decide(client: &AtlasClient, action_raw: &str, target: &str) -> AuthorizeDecision {
    client
        .custos()
        .authorize(&question(action_raw, target))
        .await
        .expect("authorize answers")
        .decision
}

#[tokio::test]
async fn an_unauthenticated_caller_is_401_on_both_routes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let anonymous = server.client();
    let target = group_ref(uuid::Uuid::now_v7());

    let single = anonymous
        .custos()
        .authorize(&question(GROUP_READ, &target))
        .await
        .expect_err("unauthenticated");
    let batch = anonymous
        .custos()
        .authorize_batch(&AuthorizeBatchRequest {
            action: GROUP_READ.to_string(),
            targets: vec![target],
        })
        .await
        .expect_err("unauthenticated");

    assert_eq!(problem_of(single).status, 401);
    assert_eq!(problem_of(batch).status, 401);

    db.teardown().await;
}

#[tokio::test]
async fn a_principal_gets_allow_deny_or_not_found_for_its_own_authority() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "authorize-delegate").await;
    let granted = seed_group(&db, "authorize-granted").await;
    let other = seed_group(&db, "authorize-other").await;
    seed_grant(
        &db,
        PrincipalId::from(user.id),
        &group_ref(granted),
        &[GROUP_READ],
    )
    .await;

    assert_eq!(
        decide(&client, GROUP_READ, &group_ref(granted)).await,
        AuthorizeDecision::Allow
    );
    assert_eq!(
        decide(&client, GROUP_UPDATE, &group_ref(granted)).await,
        AuthorizeDecision::Deny
    );
    assert_eq!(
        decide(&client, GROUP_READ, &group_ref(other)).await,
        AuthorizeDecision::NotFound
    );
    assert_eq!(
        decide(&client, GROUP_READ, &group_ref(uuid::Uuid::now_v7())).await,
        AuthorizeDecision::NotFound
    );

    db.teardown().await;
}

#[tokio::test]
async fn root_is_allowed_only_on_a_target_that_exists() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let group = seed_group(&db, "authorize-root").await;

    assert_eq!(
        decide(&root, GROUP_UPDATE, &group_ref(group)).await,
        AuthorizeDecision::Allow
    );
    assert_eq!(
        decide(&root, GROUP_UPDATE, &group_ref(uuid::Uuid::now_v7())).await,
        AuthorizeDecision::NotFound
    );

    db.teardown().await;
}

#[tokio::test]
async fn a_batch_answers_every_target_in_request_order() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "authorize-batch").await;
    let granted = seed_group(&db, "authorize-batch-granted").await;
    let other = seed_group(&db, "authorize-batch-other").await;
    seed_grant(
        &db,
        PrincipalId::from(user.id),
        &group_ref(granted),
        &[GROUP_READ],
    )
    .await;
    let alias = format!(
        "custos::group::{}",
        granted.hyphenated().to_string().to_uppercase()
    );
    let targets = vec![
        group_ref(uuid::Uuid::now_v7()),
        group_ref(granted),
        alias,
        group_ref(other),
    ];

    let response = client
        .custos()
        .authorize_batch(&AuthorizeBatchRequest {
            action: GROUP_READ.to_string(),
            targets: targets.clone(),
        })
        .await
        .expect("batch answers");

    assert_eq!(
        response.results,
        targets
            .into_iter()
            .zip([
                AuthorizeDecision::NotFound,
                AuthorizeDecision::Allow,
                AuthorizeDecision::NotFound,
                AuthorizeDecision::NotFound,
            ])
            .map(|(target, decision)| AuthorizeBatchResult { target, decision })
            .collect::<Vec<_>>()
    );

    db.teardown().await;
}

#[tokio::test]
async fn an_alias_spelling_of_an_existing_id_is_not_found() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "authorize-alias").await;
    let group = seed_group(&db, "authorize-alias").await;
    seed_grant(
        &db,
        PrincipalId::from(user.id),
        &group_ref(group),
        &[GROUP_READ],
    )
    .await;

    for alias in [
        group.hyphenated().to_string().to_uppercase(),
        group.simple().to_string(),
    ] {
        assert_eq!(
            decide(&client, GROUP_READ, &format!("custos::group::{alias}")).await,
            AuthorizeDecision::NotFound,
            "{alias}"
        );
    }

    db.teardown().await;
}

#[tokio::test]
async fn an_api_key_is_bounded_by_its_credential_ceiling() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (key_client, key_principal) = api_key_client(&server, &db, Capability::ALL.to_vec()).await;
    let (session_client, user) = support::login_user(&server, &db, "authorize-ceiling").await;
    let group = seed_group(&db, "authorize-ceiling").await;
    let grant_row = seed_grant(
        &db,
        PrincipalId::from(user.id),
        &group_ref(group),
        &[GROUP_READ],
    )
    .await;
    let target = format!("custos::grant::{grant_row}");

    for principal in [key_principal, PrincipalId::from(user.id)] {
        seed_grant(&db, principal, &target, &[GRANT_READ, GRANT_DELETE]).await;
    }

    assert_eq!(
        decide(&key_client, GRANT_READ, &target).await,
        AuthorizeDecision::Allow
    );
    assert_eq!(
        decide(&key_client, GRANT_DELETE, &target).await,
        AuthorizeDecision::Deny,
        "no key scope translates to custos::grant::delete"
    );
    assert_eq!(
        decide(&session_client, GRANT_DELETE, &target).await,
        AuthorizeDecision::Allow,
        "a session holding the same grant is not bounded"
    );

    db.teardown().await;
}

#[tokio::test]
async fn malformed_questions_are_400_and_unpublished_products_are_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "authorize-invalid").await;
    let group = group_ref(uuid::Uuid::now_v7());

    for (action_raw, target) in [
        ("custos::group", group.as_str()),
        (GROUP_READ, "custos::group"),
        (GROUP_READ, "custos::workspace::w1/group::g1"),
    ] {
        let problem = problem_of(
            client
                .custos()
                .authorize(&question(action_raw, target))
                .await
                .expect_err("malformed"),
        );

        assert_eq!(problem.status, 400, "{action_raw} on {target}");
    }

    // `platform` declares no V2 resource kinds, so it is the product without
    // a published catalog (Acta publishes one since v2-e7-s1a).
    for (action_raw, target) in [
        ("platform::thing::read", "platform::thing::t1"),
        (GROUP_READ, "platform::thing::t1"),
    ] {
        let problem = problem_of(
            client
                .custos()
                .authorize(&question(action_raw, target))
                .await
                .expect_err("unpublished product"),
        );

        assert_eq!(problem.status, 422, "{action_raw} on {target}");
    }

    let batch = problem_of(
        client
            .custos()
            .authorize_batch(&AuthorizeBatchRequest {
                action: GROUP_READ.to_string(),
                targets: vec![group, "not a ref".to_string()],
            })
            .await
            .expect_err("one malformed target"),
    );
    assert_eq!(batch.status, 400);

    db.teardown().await;
}

/// A Custos provider that never answers: the request must end on the
/// configured timeout, never hang and never allow.
struct HangingProvider;

#[async_trait]
impl ResourceProvider for HangingProvider {
    async fn validate_ref(&self, _resource: &ResourceRef) -> Result<bool, CapabilityError> {
        std::future::pending().await
    }

    async fn path_of(&self, _resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        std::future::pending().await
    }

    async fn ancestors(
        &self,
        _resource: &ResourceRef,
    ) -> Result<Vec<ResourceRef>, CapabilityError> {
        std::future::pending().await
    }

    async fn members_of(
        &self,
        _set: &PrincipalSetId,
    ) -> Result<Vec<atlas_core::ids::PrincipalId>, CapabilityError> {
        std::future::pending().await
    }

    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
        std::future::pending().await
    }

    async fn resource_facts(
        &self,
        _resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn a_hanging_provider_is_503_for_one_target_and_not_found_in_a_batch() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let settings = AuthorizationSettings {
        catalog: validation_catalog(&state.registry).expect("catalog"),
        deny_mode: DenyMode::Disabled,
        provider_timeout: Duration::from_millis(200),
    };
    let hanging = AuthorizationService::new(
        ProviderSet::new().with("custos", Arc::new(HangingProvider)),
        PgAuthorizationFactsStore {
            conn: db.conn().clone(),
        },
        PgGroupMembershipSource {
            conn: db.conn().clone(),
        },
        Arc::new(TokioSleeper),
        settings,
    );
    let server =
        support::TestServer::spawn_with_state(state.with_authorization_service(Arc::new(hanging)))
            .await;
    let (client, user) = support::login_user(&server, &db, "authorize-timeout").await;
    let group = seed_group(&db, "authorize-timeout").await;
    seed_grant(
        &db,
        PrincipalId::from(user.id),
        &group_ref(group),
        &[GROUP_READ],
    )
    .await;

    let started = std::time::Instant::now();
    let single = problem_of(
        client
            .custos()
            .authorize(&question(GROUP_READ, &group_ref(group)))
            .await
            .expect_err("facts are unavailable"),
    );
    let batch = client
        .custos()
        .authorize_batch(&AuthorizeBatchRequest {
            action: GROUP_READ.to_string(),
            targets: vec![group_ref(group)],
        })
        .await
        .expect("a batch answers per target");
    let elapsed = started.elapsed();

    assert_eq!(single.status, 503);
    assert_eq!(single.r#type, "urn:atlas:error:authorization-unavailable");
    assert!(single.detail.is_none(), "no cause leaves the server");
    assert_eq!(
        batch.results,
        vec![AuthorizeBatchResult {
            target: group_ref(group),
            decision: AuthorizeDecision::NotFound,
        }]
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "both requests must end on the timeout, took {elapsed:?}"
    );

    db.teardown().await;
}

/// Lowercase deny-mode name usable inside a workspace slug.
fn mode_slug(mode: DenyModeConfig) -> &'static str {
    match mode {
        DenyModeConfig::Disabled => "disabled",
        DenyModeConfig::Audit => "audit",
        DenyModeConfig::Enforced => "enforced",
    }
}

#[tokio::test]
async fn stored_denies_never_block_in_audit_or_disabled_mode() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    for mode in [DenyModeConfig::Disabled, DenyModeConfig::Audit] {
        let state = AppState::for_test(db.conn().clone())
            .await
            .expect("AppState::for_test")
            .with_deny_mode(mode)
            .expect("deny mode");
        let server = support::TestServer::spawn_with_state(state).await;
        let (client, user) =
            support::login_user(&server, &db, &format!("authorize-{}", mode_slug(mode))).await;
        let group = seed_group(&db, &format!("authorize-deny-{}", mode_slug(mode))).await;
        let principal = PrincipalId::from(user.id);
        seed_grant(
            &db,
            principal,
            &group_ref(group),
            &[GROUP_READ, GROUP_UPDATE],
        )
        .await;
        seed_deny(&db, principal, &group_ref(group), &[GROUP_READ]).await;

        assert_eq!(
            decide(&client, GROUP_READ, &group_ref(group)).await,
            AuthorizeDecision::Allow,
            "{mode:?}"
        );
    }

    db.teardown().await;
}
