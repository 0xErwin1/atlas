//! Grant-create authority on the Custos authorization routes
//! (`v2-e5-s5b-grant-authority`, GRANT-3/GRANT-4): a non-platform-admin
//! actor may create a grant only with effective `custos::grant::create` on
//! the exact target, may grant only actions it holds there, may revoke only
//! with effective `custos::grant::delete` on the grant's target; credential
//! ceilings apply first (an API key never carries `custos::grant::create`);
//! every refusal writes a `grant.denied` audit row with a reason code; an
//! unavailable authorization service answers 503 without a cause.
//!
//! Targets are real `custos.groups` rows: the Custos provider answers
//! existence from storage, so a grant on a group that does not exist gives
//! the actor no effective authority there.

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
    AuthorityDto, CreateGrantV2Request, GrantV2Dto, SubjectDto, TargetDto,
};
use atlas_client::{AtlasClient, ClientError};
use atlas_core::capabilities::{CapabilityError, ProviderCatalog, ResourceFacts, ResourceProvider};
use atlas_core::ids::{ActionId, PrincipalSetId, ResourceRef};
use atlas_custos::WorkspaceScope;
use atlas_custos::authorize::{AuthorizationService, AuthorizationSettings, ProviderSet};
use atlas_custos::capability::Capability;
use atlas_custos::entities::authorization::{
    GrantAuthority, GrantId, GrantRecord, NewGrantRecord, SubjectRecord, SubjectSet, TargetRecord,
};
use atlas_custos::entities::groups::NewGroup;
use atlas_custos::entities::identity::{ApiKeyType, NewApiKey, User};
use atlas_custos::eval::DenyMode;
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::authorization::GrantV2Repo;
use atlas_custos::ports::group_repo::GroupRepo;
use atlas_custos_postgres::repos::authorization::PgGrantV2Repo;
use atlas_custos_postgres::repos::authorize::{PgAuthorizationFactsStore, PgGroupMembershipSource};
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo};
use atlas_custos_postgres::repos::permissions::PgGroupRepo;
use atlas_server::auth::tokens::{generate_api_key, hash_token};
use atlas_server::authz::v2_service::{TokioSleeper, validation_catalog};
use atlas_server::state::AppState;
use sea_orm::{DatabaseBackend, FromQueryResult, Statement};

const GROUP_READ: &str = "custos::group::read";
const GROUP_ADD_MEMBER: &str = "custos::group::add_member";
const GRANT_CREATE: &str = "custos::grant::create";
const GRANT_DELETE: &str = "custos::grant::delete";

fn action(raw: &str) -> ActionId {
    raw.parse().expect("valid action id")
}

fn group_target(group_id: uuid::Uuid) -> TargetRecord {
    TargetRecord::Ref(
        format!("custos::group::{group_id}")
            .parse()
            .expect("valid target ref"),
    )
}

fn group_target_dto(group_id: uuid::Uuid) -> TargetDto {
    TargetDto::Ref {
        value: format!("custos::group::{group_id}"),
    }
}

/// A real `custos.groups` row, the resource the Custos provider reports as
/// existing.
async fn seed_group(db: &support::TestDb, name: &str) -> uuid::Uuid {
    let (ws, owner) = support::seed_workspace(db, &format!("{name}-owner")).await;
    let group = PgGroupRepo {
        conn: db.conn().clone(),
    }
    .create(NewGroup {
        workspace_id: WorkspaceScope(ws.id.0),
        name: name.to_string(),
        created_by: owner.id,
    })
    .await
    .expect("seed group");

    group.id.0
}

/// Seeds a grant of explicit `actions` on the group for `actor`, the way a
/// platform admin would have created it through `POST /grants`.
async fn seed_actor_grant(
    db: &support::TestDb,
    actor: &User,
    group_id: uuid::Uuid,
    actions: &[&str],
) -> GrantRecord {
    PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .create(NewGrantRecord {
        id: GrantId::new(),
        subject: SubjectRecord::Principal(PrincipalId::from(actor.id)),
        target: group_target(group_id),
        authority: GrantAuthority::Actions(actions.iter().map(|raw| action(raw)).collect()),
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed actor grant")
}

/// An agent API key with `scopes`, owned by a workspace owner and presented
/// as a bearer token.
async fn api_key_client(
    server: &support::TestServer,
    db: &support::TestDb,
    scopes: Vec<Capability>,
) -> (AtlasClient, uuid::Uuid) {
    let (ws, owner) = support::seed_workspace(db, "grant-authority-key-owner").await;
    let raw_token = generate_api_key();
    let ctx = support::ctx(&ws, &owner);

    let key = PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        atlas_custos::WorkspaceScope(ctx.workspace_id.0),
        &ctx.actor,
        NewApiKey {
            name: "grant-authority-key".to_string(),
            token_hash: hash_token(&raw_token),
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes,
        },
    )
    .await
    .expect("create api key");

    (
        AtlasClient::new(server.base_url().to_string()).with_token(raw_token),
        key.id.0,
    )
}

fn status_of(err: ClientError) -> u16 {
    match err {
        ClientError::Api(problem) => problem.status,
        other => panic!("expected an API problem, got {other:?}"),
    }
}

fn problem_of(err: ClientError) -> atlas_api::problem::ProblemDetails {
    match err {
        ClientError::Api(problem) => problem,
        other => panic!("expected an API problem, got {other:?}"),
    }
}

fn grant_request(group_id: uuid::Uuid, actions: &[&str]) -> CreateGrantV2Request {
    CreateGrantV2Request {
        subject: SubjectDto::Principal {
            id: uuid::Uuid::now_v7(),
        },
        target: group_target_dto(group_id),
        authority: AuthorityDto::Actions {
            actions: actions.iter().map(|raw| (*raw).to_string()).collect(),
        },
    }
}

#[derive(Debug, FromQueryResult)]
struct AuditRow {
    action: String,
    target_type: String,
    target_id: Option<uuid::Uuid>,
    metadata: serde_json::Value,
}

/// Every `grant.*` audit row attributed to the user `actor`, oldest first.
async fn user_audit_rows(db: &support::TestDb, actor: uuid::Uuid) -> Vec<AuditRow> {
    audit_rows_where(db, &format!("actor_user_id = '{actor}'")).await
}

/// Every `grant.*` audit row attributed to the api key `key`, oldest first.
async fn key_audit_rows(db: &support::TestDb, key: uuid::Uuid) -> Vec<AuditRow> {
    audit_rows_where(db, &format!("actor_api_key_id = '{key}'")).await
}

async fn audit_rows_where(db: &support::TestDb, actor_clause: &str) -> Vec<AuditRow> {
    AuditRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT action, target_type, target_id, metadata FROM custos.security_audit_log \
             WHERE {actor_clause} AND action LIKE 'grant.%' \
             ORDER BY created_at ASC, id ASC"
        ),
    ))
    .all(db.conn())
    .await
    .expect("query audit rows")
}

fn assert_denied_row(row: &AuditRow, group_id: uuid::Uuid, reason: &str) {
    assert_eq!(row.action, "grant.denied");
    assert_eq!(row.target_type, "grant_v2");
    assert_eq!(row.target_id, None);
    assert_eq!(row.metadata["target"], format!("custos::group::{group_id}"));
    assert_eq!(row.metadata["reason"], reason);
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The fixture the non-admin scenarios build on: an editor-like actor whose
/// only authority is `[custos::group::read, custos::grant::create]` on the
/// group, visible through the storage the facts loader reads.
#[tokio::test]
async fn the_seeded_actor_holds_exactly_its_grant_on_the_target() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, actor) = support::login_user(&server, &db, "grant-authority-actor").await;
    let group_id = seed_group(&db, "fixture-group").await;

    let seeded = seed_actor_grant(&db, &actor, group_id, &[GROUP_READ, GRANT_CREATE]).await;

    let subjects = SubjectSet {
        principal: PrincipalId::from(actor.id),
        groups: vec![],
        principal_sets: vec![],
    };
    let facts = PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .list_for_subjects("custos", &subjects)
    .await
    .expect("list the actor's grants");

    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].id, seeded.id);
    assert_eq!(facts[0].target, group_target(group_id));
    assert_eq!(
        facts[0].authority,
        GrantAuthority::Actions(vec![action(GROUP_READ), action(GRANT_CREATE)])
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Grant creation by a delegate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_actor_with_grant_create_grants_within_its_authority_and_the_row_names_it() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (actor_client, actor) = support::login_user(&server, &db, "delegate-ok").await;
    let group_id = seed_group(&db, "ok-group").await;
    seed_actor_grant(&db, &actor, group_id, &[GROUP_READ, GRANT_CREATE]).await;
    let grantee = uuid::Uuid::now_v7();

    let created: GrantV2Dto = actor_client
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: SubjectDto::Principal { id: grantee },
            ..grant_request(group_id, &[GROUP_READ])
        })
        .await
        .expect("the actor may grant what it holds");
    assert_eq!(created.created_by, actor.id.0);
    assert_eq!(created.subject, SubjectDto::Principal { id: grantee });
    assert_eq!(created.target, group_target_dto(group_id));

    let rows = user_audit_rows(&db, actor.id.0).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "grant.created");
    assert_eq!(rows[0].target_id, Some(created.id));
    assert_eq!(rows[0].metadata["subject"], grantee.to_string());

    db.teardown().await;
}

#[tokio::test]
async fn an_actor_cannot_grant_actions_beyond_its_authority() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (actor_client, actor) = support::login_user(&server, &db, "delegate-beyond").await;
    let group_id = seed_group(&db, "beyond-group").await;
    seed_actor_grant(&db, &actor, group_id, &[GROUP_READ, GRANT_CREATE]).await;

    let refused = actor_client
        .custos()
        .create_grant_v2(grant_request(group_id, &[GROUP_READ, GROUP_ADD_MEMBER]))
        .await
        .expect_err("add_member is beyond the actor's authority");
    assert_eq!(status_of(refused), 403);

    let rows = user_audit_rows(&db, actor.id.0).await;
    assert_eq!(rows.len(), 1);
    assert_denied_row(&rows[0], group_id, "beyond_authority");
    assert_eq!(
        rows[0].metadata["beyond"],
        serde_json::json!([GROUP_ADD_MEMBER])
    );
    assert_eq!(
        rows[0].metadata["granted"],
        serde_json::json!([GROUP_ADD_MEMBER, GROUP_READ]),
        "the granted list is recorded in ascending order"
    );
    assert!(
        actor_client
            .custos()
            .list_grants_v2("custos")
            .await
            .is_err(),
        "listing stays platform-admin only"
    );

    db.teardown().await;
}

#[tokio::test]
async fn an_actor_cannot_grant_on_a_target_where_it_lacks_grant_create() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (actor_client, actor) = support::login_user(&server, &db, "delegate-elsewhere").await;
    let g1 = seed_group(&db, "g1").await;
    let g2 = seed_group(&db, "g2").await;
    seed_actor_grant(&db, &actor, g1, &[GROUP_READ, GRANT_CREATE]).await;

    let refused = actor_client
        .custos()
        .create_grant_v2(grant_request(g2, &[GROUP_READ]))
        .await
        .expect_err("no grant::create on g2");
    assert_eq!(status_of(refused), 403);

    let rows = user_audit_rows(&db, actor.id.0).await;
    assert_eq!(rows.len(), 1);
    assert_denied_row(&rows[0], g2, "missing_grant_create");

    db.teardown().await;
}

#[tokio::test]
async fn an_actor_without_grant_create_is_refused_even_when_it_holds_the_granted_action() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (actor_client, actor) = support::login_user(&server, &db, "delegate-reader").await;
    let group_id = seed_group(&db, "reader-group").await;
    seed_actor_grant(&db, &actor, group_id, &[GROUP_READ]).await;

    let refused = actor_client
        .custos()
        .create_grant_v2(grant_request(group_id, &[GROUP_READ]))
        .await
        .expect_err("no grant::create at all");
    assert_eq!(status_of(refused), 403);

    let rows = user_audit_rows(&db, actor.id.0).await;
    assert_eq!(rows.len(), 1);
    assert_denied_row(&rows[0], group_id, "missing_grant_create");

    db.teardown().await;
}

#[tokio::test]
async fn a_delegate_may_only_grant_on_an_exact_ref_target() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (actor_client, actor) = support::login_user(&server, &db, "delegate-selector").await;
    let group_id = seed_group(&db, "selector-group").await;
    seed_actor_grant(&db, &actor, group_id, &[GROUP_READ, GRANT_CREATE]).await;

    let refused = actor_client
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            target: TargetDto::Selector {
                value: "custos::**".to_string(),
            },
            ..grant_request(group_id, &[GROUP_READ])
        })
        .await
        .expect_err("delegation needs an exact target");
    assert_eq!(status_of(refused), 403);

    let rows = user_audit_rows(&db, actor.id.0).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "grant.denied");
    assert_eq!(rows[0].metadata["reason"], "not_exact_target");
    assert_eq!(rows[0].metadata["target"], "custos::**");

    db.teardown().await;
}

#[tokio::test]
async fn a_grant_on_a_group_that_does_not_exist_gives_no_authority_and_does_not_leak() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (actor_client, actor) = support::login_user(&server, &db, "delegate-ghost").await;
    let ghost = uuid::Uuid::now_v7();
    seed_actor_grant(&db, &actor, ghost, &[GROUP_READ, GRANT_CREATE]).await;

    let refused = actor_client
        .custos()
        .create_grant_v2(grant_request(ghost, &[GROUP_READ]))
        .await
        .expect_err("a missing target carries no authority");
    let problem = problem_of(refused);
    assert_eq!(problem.status, 403);
    assert!(
        !problem
            .detail
            .unwrap_or_default()
            .to_lowercase()
            .contains("exist"),
        "the refusal must not reveal whether the target exists"
    );

    let rows = user_audit_rows(&db, actor.id.0).await;
    assert_eq!(rows.len(), 1);
    assert_denied_row(&rows[0], ghost, "missing_grant_create");

    db.teardown().await;
}

/// GRANT-4 / S7: no key scope translates to `custos::grant::create`, so a
/// key is refused on its ceiling before any grant of its own is consulted.
#[tokio::test]
async fn an_api_key_with_every_scope_is_403_on_grant_creation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let group_id = seed_group(&db, "key-group").await;
    let (key, key_id) = api_key_client(&server, &db, Capability::ALL.to_vec()).await;

    let refused = key
        .custos()
        .create_grant_v2(grant_request(group_id, &[GROUP_READ]))
        .await
        .expect_err("a key cannot delegate");
    assert_eq!(status_of(refused), 403);

    let rows = key_audit_rows(&db, key_id).await;
    assert_eq!(rows.len(), 1);
    assert_denied_row(&rows[0], group_id, "ceiling_lacks_grant_create");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Revocation by a delegate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoking_needs_effective_grant_delete_on_the_grants_target() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (creator_client, creator) = support::login_user(&server, &db, "delegate-creator").await;
    let (revoker_client, revoker) = support::login_user(&server, &db, "delegate-revoker").await;
    let group_id = seed_group(&db, "revoke-group").await;
    seed_actor_grant(&db, &creator, group_id, &[GROUP_READ, GRANT_CREATE]).await;
    seed_actor_grant(&db, &revoker, group_id, &[GRANT_DELETE]).await;

    let grant = creator_client
        .custos()
        .create_grant_v2(grant_request(group_id, &[GROUP_READ]))
        .await
        .expect("create grant");

    let refused = creator_client
        .custos()
        .delete_grant_v2(grant.id)
        .await
        .expect_err("the creator holds no grant::delete");
    assert_eq!(status_of(refused), 403);
    let rows = user_audit_rows(&db, creator.id.0).await;
    assert_eq!(rows.len(), 2);
    assert_denied_row(&rows[1], group_id, "missing_grant_delete");
    assert_eq!(rows[1].metadata["grant_id"], grant.id.to_string());

    assert_eq!(
        status_of(
            revoker_client
                .custos()
                .delete_grant_v2(uuid::Uuid::now_v7())
                .await
                .expect_err("an unknown id is 404 before any evaluation")
        ),
        404
    );

    revoker_client
        .custos()
        .delete_grant_v2(grant.id)
        .await
        .expect("grant::delete on the target allows the revocation");
    let rows = user_audit_rows(&db, revoker.id.0).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].action, "grant.revoked");
    assert_eq!(rows[0].target_id, Some(grant.id));

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Unavailable authorization facts
// ---------------------------------------------------------------------------

/// A Custos provider that never answers: the request must end on the
/// configured timeout with a 503, never hang and never allow.
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
async fn a_hanging_provider_answers_503_within_the_configured_timeout() {
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
    let (actor_client, actor) = support::login_user(&server, &db, "delegate-timeout").await;
    let group_id = seed_group(&db, "timeout-group").await;
    seed_actor_grant(&db, &actor, group_id, &[GROUP_READ, GRANT_CREATE]).await;

    let started = std::time::Instant::now();
    let failed = actor_client
        .custos()
        .create_grant_v2(grant_request(group_id, &[GROUP_READ]))
        .await
        .expect_err("facts are unavailable");
    let elapsed = started.elapsed();
    let problem = problem_of(failed);

    assert_eq!(problem.status, 503);
    assert_eq!(problem.r#type, "urn:atlas:error:authorization-unavailable");
    assert!(
        problem.detail.is_none(),
        "no cause leaves the server: {problem:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "the request must end on the timeout, took {elapsed:?}"
    );
    assert!(
        user_audit_rows(&db, actor.id.0).await.is_empty(),
        "an unavailable evaluation is neither a grant nor a refusal"
    );

    db.teardown().await;
}
