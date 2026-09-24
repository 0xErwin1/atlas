//! Platform-admin administration of V2 custom roles, grants and deny rules
//! (`v2-e5-s4-authz-routes`): `/api/v2/custos/roles`, `/grants`, `/denies`.
//!
//! Every route requires a platform admin (root or `is_system_admin`)
//! session: a plain member session and an API key both answer 403. Deny
//! administration additionally requires `ATLAS_EXPLICIT_DENY_MODE` to be
//! `audit`; in `disabled` mode the rules stay readable but not writable.
//! Every mutation writes its security-audit row in the same transaction.
//!
//! Validation follows the registry catalog: Custos and Acta declare V2
//! resource kinds, so their targets validate through the catalog, while a
//! product without a published catalog (`platform` today) answers 422.
//! Custos actions are banned from custom roles (GRANT-5), so custom roles
//! exist for Acta; the role-in-use path is exercised on rows seeded through
//! the repositories.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::authorization::{
    AuthorityDto, CreateDenyRequest, CreateGrantV2Request, CreateRoleRequest, SubjectDto,
    TargetDto, UpdateRoleRequest,
};
use atlas_client::{AtlasClient, ClientError};
use atlas_core::ids::ActionId;
use atlas_custos::capability::Capability;
use atlas_custos::entities::authorization::{
    CustomRole, GrantAuthority, GrantId, GrantRecord, NewCustomRole, NewGrantRecord, RoleId,
    SubjectRecord, TargetRecord,
};
use atlas_custos::entities::identity::{ApiKeyType, NewApiKey};
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::authorization::{GrantV2Repo, RoleRepo};
use atlas_custos_postgres::repos::authorization::{PgGrantV2Repo, PgRoleRepo};
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo};
use atlas_server::auth::tokens::{generate_api_key, hash_token};
use atlas_server::config::DenyModeConfig;
use atlas_server::state::AppState;
use sea_orm::{DatabaseBackend, FromQueryResult, Statement};

/// Logs in a fresh platform admin (`is_system_admin`) and returns its client
/// with the user id the audit rows are attributed to.
async fn login_platform_admin(
    server: &support::TestServer,
    db: &support::TestDb,
) -> (AtlasClient, uuid::Uuid) {
    let (client, admin) = support::login_system_admin(server, db).await;

    (client, admin.id.0)
}

/// An all-scopes agent API key owned by a workspace owner, presented as a
/// bearer token: the credential kind the routes refuse regardless of scope.
async fn api_key_client(server: &support::TestServer, db: &support::TestDb) -> AtlasClient {
    let (ws, owner) = support::seed_workspace(db, "authz-key-owner").await;
    let raw_token = generate_api_key();
    let ctx = support::ctx(&ws, &owner);

    PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        atlas_custos::WorkspaceScope(ctx.workspace_id.0),
        &ctx.actor,
        NewApiKey {
            name: "authz-key".to_string(),
            token_hash: hash_token(&raw_token),
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes: Capability::ALL.to_vec(),
        },
    )
    .await
    .expect("create api key");

    AtlasClient::new(server.base_url().to_string()).with_token(raw_token)
}

async fn server_with_deny_mode(db: &support::TestDb, mode: DenyModeConfig) -> support::TestServer {
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_deny_mode(mode)
        .expect("rebuild the authorization service with the mode");
    support::TestServer::spawn_with_state(state).await
}

fn status_of(err: ClientError) -> u16 {
    match err {
        ClientError::Api(problem) => problem.status,
        other => panic!("expected an API problem, got {other:?}"),
    }
}

fn problem_type_of(err: ClientError) -> String {
    match err {
        ClientError::Api(problem) => problem.r#type,
        other => panic!("expected an API problem, got {other:?}"),
    }
}

fn detail_of(err: ClientError) -> String {
    match err {
        ClientError::Api(problem) => problem.detail.unwrap_or_default(),
        other => panic!("expected an API problem, got {other:?}"),
    }
}

/// A 422 naming the reserved membership-role prefix.
fn assert_reserved(err: ClientError) {
    match err {
        ClientError::Api(problem) => {
            assert_eq!(problem.status, 422, "{problem:?}");
            let detail = problem.detail.unwrap_or_default();
            assert!(
                detail.contains("reserved for workspace membership roles"),
                "{detail}"
            );
        }
        other => panic!("expected an API problem, got {other:?}"),
    }
}

fn principal_subject() -> SubjectDto {
    SubjectDto::Principal {
        id: uuid::Uuid::now_v7(),
    }
}

fn custos_ref(id: &str) -> TargetDto {
    TargetDto::Ref {
        value: format!("custos::group::{id}"),
    }
}

fn acta_ref(id: &str) -> TargetDto {
    TargetDto::Ref {
        value: format!("acta::document::{id}"),
    }
}

fn explicit_custos_actions() -> AuthorityDto {
    AuthorityDto::Actions {
        actions: vec!["custos::group::read".to_string()],
    }
}

fn action(raw: &str) -> ActionId {
    raw.parse().expect("valid action id")
}

/// Seeds an Acta custom role directly in storage: the API cannot create one
/// until Acta publishes its catalog, but the update/delete/role-in-use
/// paths must still be proven end to end.
async fn seed_acta_role(db: &support::TestDb, name: &str) -> CustomRole {
    PgRoleRepo {
        conn: db.conn().clone(),
    }
    .create(NewCustomRole {
        id: RoleId::new(),
        product: "acta".to_string(),
        name: name.to_string(),
        actions: vec![action("acta::document::read")],
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed role")
}

/// Seeds a grant that references `role` on an Acta target, the way E7 will
/// create them once Acta's catalog is published.
async fn seed_custom_role_grant(db: &support::TestDb, role: &CustomRole) -> GrantRecord {
    PgGrantV2Repo {
        conn: db.conn().clone(),
    }
    .create(NewGrantRecord {
        id: GrantId::new(),
        subject: SubjectRecord::Principal(PrincipalId::new()),
        target: TargetRecord::Ref("acta::document::d1".parse().unwrap()),
        authority: GrantAuthority::CustomRole(role.id),
        created_by: PrincipalId::new(),
    })
    .await
    .expect("seed grant")
}

#[derive(Debug, FromQueryResult)]
struct AuditRow {
    action: String,
    target_type: String,
    target_id: Option<uuid::Uuid>,
    metadata: serde_json::Value,
}

/// Every audit row attributed to `actor` whose action starts with `prefix`,
/// oldest first.
async fn audit_rows(db: &support::TestDb, actor: uuid::Uuid, prefix: &str) -> Vec<AuditRow> {
    AuditRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT action, target_type, target_id, metadata FROM custos.security_audit_log \
             WHERE actor_user_id = '{actor}' AND action LIKE '{prefix}%' \
             ORDER BY created_at ASC, id ASC"
        ),
    ))
    .all(db.conn())
    .await
    .expect("query audit rows")
}

// ---------------------------------------------------------------------------
// Gate: 401 / 403
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unauthenticated_requests_are_rejected_with_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let anonymous = server.client();

    let err = anonymous
        .custos()
        .list_roles("acta")
        .await
        .expect_err("no credential");
    assert_eq!(status_of(err), 401);

    let err = anonymous
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: principal_subject(),
            target: custos_ref("g1"),
            authority: explicit_custos_actions(),
        })
        .await
        .expect_err("no credential");
    assert_eq!(status_of(err), 401);

    db.teardown().await;
}

async fn assert_every_route_answers_403(client: &AtlasClient) {
    let id = uuid::Uuid::now_v7();

    let statuses = [
        status_of(client.custos().list_roles("acta").await.expect_err("403")),
        status_of(
            client
                .custos()
                .create_role(CreateRoleRequest {
                    product: "acta".to_string(),
                    name: "reviewer".to_string(),
                    actions: vec!["acta::document::read".to_string()],
                })
                .await
                .expect_err("403"),
        ),
        status_of(
            client
                .custos()
                .update_role(id, UpdateRoleRequest::default())
                .await
                .expect_err("403"),
        ),
        status_of(client.custos().delete_role(id).await.expect_err("403")),
        status_of(
            client
                .custos()
                .list_grants_v2("custos")
                .await
                .expect_err("403"),
        ),
        status_of(
            client
                .custos()
                .create_grant_v2(CreateGrantV2Request {
                    subject: principal_subject(),
                    target: custos_ref("g1"),
                    authority: explicit_custos_actions(),
                })
                .await
                .expect_err("403"),
        ),
        status_of(client.custos().delete_grant_v2(id).await.expect_err("403")),
        status_of(
            client
                .custos()
                .list_denies("custos")
                .await
                .expect_err("403"),
        ),
        status_of(
            client
                .custos()
                .create_deny(CreateDenyRequest {
                    subject: principal_subject(),
                    target: custos_ref("g1"),
                    actions: vec!["custos::group::read".to_string()],
                })
                .await
                .expect_err("403"),
        ),
        status_of(client.custos().delete_deny(id).await.expect_err("403")),
    ];

    // `DELETE /grants/{id}` resolves the grant before any authority check
    // (v2-e5-s5b): an unknown id is 404 for everyone, so a non-admin cannot
    // probe ids through the difference between 403 and 404.
    assert_eq!(
        statuses,
        [403, 403, 403, 403, 403, 403, 404, 403, 403, 403],
        "every route must refuse; the grant revocation answers 404 for an unknown id"
    );
}

#[tokio::test]
async fn a_plain_member_session_gets_403_on_every_route() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (member, _user) = support::login_user(&server, &db, "authz-member").await;

    assert_every_route_answers_403(&member).await;

    db.teardown().await;
}

#[tokio::test]
async fn an_api_key_gets_403_on_every_route_even_with_every_scope() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = server_with_deny_mode(&db, DenyModeConfig::Audit).await;
    let key = api_key_client(&server, &db).await;

    assert_every_route_answers_403(&key).await;

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

#[tokio::test]
async fn role_creation_accepts_a_published_product_and_rejects_the_rest_with_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (admin, admin_id) = login_platform_admin(&server, &db).await;

    let created = admin
        .custos()
        .create_role(CreateRoleRequest {
            product: "acta".to_string(),
            name: "reviewer".to_string(),
            actions: vec!["acta::document::read".to_string()],
        })
        .await
        .expect("acta has published its catalog");
    assert_eq!(created.product, "acta");
    assert_eq!(created.actions, vec!["acta::document::read"]);

    let unpublished = admin
        .custos()
        .create_role(CreateRoleRequest {
            product: "platform".to_string(),
            name: "operator".to_string(),
            actions: vec!["platform::thing::read".to_string()],
        })
        .await
        .expect_err("platform has not published a V2 catalog");
    let detail = detail_of(unpublished);
    assert!(
        detail.contains("platform") && detail.contains("published"),
        "got: {detail}"
    );

    let cases = [
        (
            "custos actions are never custom-role actions (GRANT-5)",
            CreateRoleRequest {
                product: "custos".to_string(),
                name: "auditor".to_string(),
                actions: vec!["custos::group::read".to_string()],
            },
        ),
        (
            "unknown product",
            CreateRoleRequest {
                product: "nope".to_string(),
                name: "r".to_string(),
                actions: vec!["nope::thing::read".to_string()],
            },
        ),
        (
            "empty action list",
            CreateRoleRequest {
                product: "acta".to_string(),
                name: "empty".to_string(),
                actions: vec![],
            },
        ),
        (
            "malformed action",
            CreateRoleRequest {
                product: "acta".to_string(),
                name: "malformed".to_string(),
                actions: vec!["not an action".to_string()],
            },
        ),
        (
            "V1 plural family is not a V2 action",
            CreateRoleRequest {
                product: "acta".to_string(),
                name: "plural".to_string(),
                actions: vec!["acta::docs::read".to_string()],
            },
        ),
        (
            "actions of another product than the role's",
            CreateRoleRequest {
                product: "acta".to_string(),
                name: "custos-actions".to_string(),
                actions: vec!["custos::group::read".to_string()],
            },
        ),
    ];
    for (label, case) in cases {
        let err = admin
            .custos()
            .create_role(case)
            .await
            .expect_err("invalid role must be rejected");
        assert_eq!(status_of(err), 422, "case: {label}");
    }

    assert_eq!(
        admin
            .custos()
            .list_roles("acta")
            .await
            .expect("list roles")
            .len(),
        1,
        "only the valid role was persisted"
    );
    assert!(
        admin
            .custos()
            .list_roles("custos")
            .await
            .expect("list roles")
            .is_empty()
    );
    assert_eq!(audit_rows(&db, admin_id, "role.").await.len(), 1);

    db.teardown().await;
}

/// Names under `acta:workspace-` belong to the workspace membership roles
/// the access dual-write maintains: an admin can neither create one, nor
/// rename a role into one, nor rename one out of it.
#[tokio::test]
async fn membership_role_names_are_reserved() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (admin, admin_id) = login_platform_admin(&server, &db).await;
    let reviewer = seed_acta_role(&db, "reviewer").await;
    let membership = seed_acta_role(&db, "acta:workspace-owner").await;

    for name in [
        "acta:workspace-owner",
        "acta:workspace-admin",
        "  acta:workspace-anything",
    ] {
        let err = admin
            .custos()
            .create_role(CreateRoleRequest {
                product: "acta".to_string(),
                name: name.to_string(),
                actions: vec!["acta::document::read".to_string()],
            })
            .await
            .expect_err("a reserved name cannot be created");
        assert_reserved(err);
    }

    let into = admin
        .custos()
        .update_role(
            reviewer.id.0,
            UpdateRoleRequest {
                name: Some("acta:workspace-admin".to_string()),
                actions: None,
            },
        )
        .await
        .expect_err("a role cannot be renamed into a reserved name");
    assert_reserved(into);

    let out_of = admin
        .custos()
        .update_role(
            membership.id.0,
            UpdateRoleRequest {
                name: Some("renamed".to_string()),
                actions: None,
            },
        )
        .await
        .expect_err("a membership role cannot be renamed");
    assert_reserved(out_of);

    let names: Vec<String> = admin
        .custos()
        .list_roles("acta")
        .await
        .expect("list roles")
        .into_iter()
        .map(|role| role.name)
        .collect();
    assert_eq!(names, vec!["reviewer", "acta:workspace-owner"]);
    assert!(audit_rows(&db, admin_id, "role.").await.is_empty());

    db.teardown().await;
}

#[tokio::test]
async fn roles_are_listed_renamed_and_deleted_with_an_audit_row_per_mutation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (admin, admin_id) = login_platform_admin(&server, &db).await;

    let reviewer = seed_acta_role(&db, "reviewer").await;
    let editor = seed_acta_role(&db, "editor").await;

    let listed = admin.custos().list_roles("acta").await.expect("list roles");
    assert_eq!(
        listed.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![reviewer.id.0, editor.id.0]
    );
    assert_eq!(listed[0].actions, vec!["acta::document::read"]);
    assert!(
        admin
            .custos()
            .list_roles("custos")
            .await
            .expect("list roles")
            .is_empty()
    );

    let renamed = admin
        .custos()
        .update_role(
            reviewer.id.0,
            UpdateRoleRequest {
                name: Some("auditor".to_string()),
                actions: None,
            },
        )
        .await
        .expect("rename role");
    assert_eq!(renamed.id, reviewer.id.0);
    assert_eq!(renamed.name, "auditor");
    assert_eq!(
        renamed.actions,
        vec!["acta::document::read"],
        "actions untouched"
    );
    assert!(renamed.updated_at > reviewer.updated_at);

    let blank = admin
        .custos()
        .update_role(
            reviewer.id.0,
            UpdateRoleRequest {
                name: Some("   ".to_string()),
                actions: None,
            },
        )
        .await
        .expect_err("a whitespace-only name is rejected");
    assert_eq!(status_of(blank), 422);

    let replaced = admin
        .custos()
        .update_role(
            reviewer.id.0,
            UpdateRoleRequest {
                name: None,
                actions: Some(vec!["acta::document::update".to_string()]),
            },
        )
        .await
        .expect("acta actions validate against the published catalog");
    assert_eq!(replaced.actions, vec!["acta::document::update"]);

    let collision = admin
        .custos()
        .update_role(
            reviewer.id.0,
            UpdateRoleRequest {
                name: Some(editor.name.clone()),
                actions: None,
            },
        )
        .await
        .expect_err("renaming onto an existing name");
    assert_eq!(status_of(collision), 409);

    admin
        .custos()
        .delete_role(reviewer.id.0)
        .await
        .expect("delete role");
    assert_eq!(
        admin
            .custos()
            .list_roles("acta")
            .await
            .expect("list roles")
            .len(),
        1
    );

    let missing = uuid::Uuid::now_v7();
    assert_eq!(
        status_of(
            admin
                .custos()
                .update_role(missing, UpdateRoleRequest::default())
                .await
                .expect_err("unknown role")
        ),
        404
    );
    assert_eq!(
        status_of(
            admin
                .custos()
                .delete_role(missing)
                .await
                .expect_err("unknown role")
        ),
        404
    );

    let rows = audit_rows(&db, admin_id, "role.").await;
    let actions: Vec<&str> = rows.iter().map(|row| row.action.as_str()).collect();
    assert_eq!(actions, ["role.updated", "role.updated", "role.deleted"]);
    for row in &rows {
        assert_eq!(row.target_type, "role");
        assert_eq!(row.target_id, Some(reviewer.id.0));
        assert_eq!(row.metadata["product"], "acta");
        assert_eq!(row.metadata["name"], "auditor");
    }
    assert_eq!(
        rows[0].metadata["previous_name"], "reviewer",
        "role.updated records the name it replaced"
    );
    assert!(
        rows[0].metadata.get("previous_actions").is_none(),
        "unchanged actions are not recorded as previous: {}",
        rows[0].metadata
    );
    assert_eq!(
        rows[1].metadata["previous_actions"],
        serde_json::json!(["acta::document::read"]),
        "role.updated records the actions it replaced"
    );
    assert!(rows[1].metadata.get("previous_name").is_none());
    assert!(rows[2].metadata.get("previous_name").is_none());

    db.teardown().await;
}

#[tokio::test]
async fn deleting_a_role_referenced_by_a_grant_is_409_role_in_use() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (admin, admin_id) = login_platform_admin(&server, &db).await;

    let role = seed_acta_role(&db, "reviewer").await;
    let grant = seed_custom_role_grant(&db, &role).await;

    let blocked = admin
        .custos()
        .delete_role(role.id.0)
        .await
        .expect_err("a referenced role is not deletable");
    assert_eq!(problem_type_of(blocked), "urn:atlas:error:role-in-use");
    assert_eq!(
        admin
            .custos()
            .list_roles("acta")
            .await
            .expect("list roles")
            .len(),
        1,
        "the role survives"
    );
    assert!(
        audit_rows(&db, admin_id, "role.deleted").await.is_empty(),
        "a blocked delete writes no audit row"
    );

    admin
        .custos()
        .delete_grant_v2(grant.id.0)
        .await
        .expect("revoke grant");
    admin
        .custos()
        .delete_role(role.id.0)
        .await
        .expect("delete unreferenced role");

    let rows = audit_rows(&db, admin_id, "").await;
    let actions: Vec<&str> = rows.iter().map(|row| row.action.as_str()).collect();
    assert_eq!(actions, ["grant.revoked", "role.deleted"]);
    assert_eq!(rows[0].target_type, "grant_v2");
    assert_eq!(rows[0].target_id, Some(grant.id.0));
    assert_eq!(rows[0].metadata["authority_kind"], "custom");
    let SubjectRecord::Principal(seeded_principal) = &grant.subject else {
        panic!("the seeded grant is addressed to a principal");
    };
    assert_eq!(rows[0].metadata["subject"], seeded_principal.0.to_string());

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Grants
// ---------------------------------------------------------------------------

#[tokio::test]
async fn grants_are_created_listed_and_revoked_with_an_audit_row_per_mutation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (admin, admin_id) = login_platform_admin(&server, &db).await;
    let group_id = uuid::Uuid::now_v7();

    let principal_id = uuid::Uuid::now_v7();
    let by_ref = admin
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: SubjectDto::Principal { id: principal_id },
            target: custos_ref("g1"),
            authority: explicit_custos_actions(),
        })
        .await
        .expect("create ref grant");
    assert_eq!(by_ref.product, "custos");
    assert_eq!(by_ref.target, custos_ref("g1"));
    assert_eq!(by_ref.authority, explicit_custos_actions());
    assert_eq!(by_ref.created_by, admin_id);

    let by_selector = admin
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: SubjectDto::Group { id: group_id },
            target: TargetDto::Selector {
                value: "custos::**".to_string(),
            },
            authority: AuthorityDto::Actions {
                actions: vec![
                    "custos::group::read".to_string(),
                    "custos::user::read".to_string(),
                ],
            },
        })
        .await
        .expect("create selector grant");
    assert_eq!(by_selector.subject, SubjectDto::Group { id: group_id });
    assert_eq!(
        by_selector.target,
        TargetDto::Selector {
            value: "custos::**".to_string()
        }
    );

    let by_path = admin
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: principal_subject(),
            target: TargetDto::Path {
                value: "custos::user::u1".to_string(),
            },
            authority: AuthorityDto::Actions {
                actions: vec!["custos::user::reset_password".to_string()],
            },
        })
        .await
        .expect("create path grant");

    let listed = admin
        .custos()
        .list_grants_v2("custos")
        .await
        .expect("list grants");
    assert_eq!(
        listed.iter().map(|g| g.id).collect::<Vec<_>>(),
        vec![by_ref.id, by_selector.id, by_path.id]
    );
    assert!(
        admin
            .custos()
            .list_grants_v2("acta")
            .await
            .expect("list grants")
            .is_empty()
    );

    admin
        .custos()
        .delete_grant_v2(by_ref.id)
        .await
        .expect("revoke grant");
    assert_eq!(
        status_of(
            admin
                .custos()
                .delete_grant_v2(by_ref.id)
                .await
                .expect_err("already revoked")
        ),
        404
    );

    let rows = audit_rows(&db, admin_id, "grant.").await;
    let actions: Vec<&str> = rows.iter().map(|row| row.action.as_str()).collect();
    assert_eq!(
        actions,
        [
            "grant.created",
            "grant.created",
            "grant.created",
            "grant.revoked"
        ]
    );
    assert_eq!(rows[0].target_type, "grant_v2");
    assert_eq!(rows[0].target_id, Some(by_ref.id));
    assert_eq!(rows[0].metadata["target"], "custos::group::g1");
    assert_eq!(rows[0].metadata["authority_kind"], "actions");
    assert_eq!(rows[0].metadata["subject"], principal_id.to_string());
    assert_eq!(rows[1].metadata["subject_kind"], "group");
    assert_eq!(rows[1].metadata["subject"], group_id.to_string());
    assert_eq!(rows[3].metadata["subject"], principal_id.to_string());
    assert_eq!(rows[1].metadata["target_kind"], "selector");
    assert_eq!(rows[3].target_id, Some(by_ref.id));

    db.teardown().await;
}

#[tokio::test]
async fn grant_creation_rejects_invalid_targets_and_authorities_with_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (admin, _) = login_platform_admin(&server, &db).await;
    let acta_role = seed_acta_role(&db, "reviewer").await;

    let on_acta = admin
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: principal_subject(),
            target: acta_ref("d1"),
            authority: AuthorityDto::Actions {
                actions: vec!["acta::document::read".to_string()],
            },
        })
        .await
        .expect("acta has published its catalog");
    assert_eq!(on_acta.product, "acta");

    let builtin_on_acta = admin
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: principal_subject(),
            target: acta_ref("d1"),
            authority: AuthorityDto::Builtin {
                name: "editor".to_string(),
                version: 1,
            },
        })
        .await
        .expect("acta declares editor@1");
    assert_eq!(
        builtin_on_acta.authority,
        AuthorityDto::Builtin {
            name: "editor".to_string(),
            version: 1,
        }
    );

    let unpublished = admin
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: principal_subject(),
            target: TargetDto::Ref {
                value: "platform::thing::t1".to_string(),
            },
            authority: AuthorityDto::Actions {
                actions: vec!["platform::thing::read".to_string()],
            },
        })
        .await
        .expect_err("platform has not published a V2 catalog");
    let detail = detail_of(unpublished);
    assert!(
        detail.contains("platform") && detail.contains("published"),
        "got: {detail}"
    );

    let base = CreateGrantV2Request {
        subject: principal_subject(),
        target: custos_ref("g1"),
        authority: explicit_custos_actions(),
    };
    let cases: Vec<(&str, CreateGrantV2Request)> = vec![
        (
            "unknown product",
            CreateGrantV2Request {
                target: TargetDto::Ref {
                    value: "nope::group::g1".to_string(),
                },
                ..base.clone()
            },
        ),
        (
            "unknown kind",
            CreateGrantV2Request {
                target: TargetDto::Ref {
                    value: "custos::widget::w1".to_string(),
                },
                ..base.clone()
            },
        ),
        (
            "malformed target",
            CreateGrantV2Request {
                target: TargetDto::Path {
                    value: "not a path".to_string(),
                },
                ..base.clone()
            },
        ),
        (
            "malformed principal set",
            CreateGrantV2Request {
                subject: SubjectDto::PrincipalSet {
                    id: "members".to_string(),
                },
                ..base.clone()
            },
        ),
        (
            "undeclared principal set (custos declares none)",
            CreateGrantV2Request {
                subject: SubjectDto::PrincipalSet {
                    id: "custos::group::g1::members".to_string(),
                },
                ..base.clone()
            },
        ),
        (
            "built-in role of a product that declares none (custos)",
            CreateGrantV2Request {
                authority: AuthorityDto::Builtin {
                    name: "admin".to_string(),
                    version: 1,
                },
                ..base.clone()
            },
        ),
        (
            "unknown custom role",
            CreateGrantV2Request {
                authority: AuthorityDto::Custom {
                    role_id: uuid::Uuid::now_v7(),
                },
                ..base.clone()
            },
        ),
        (
            "unknown built-in role version on acta",
            CreateGrantV2Request {
                target: acta_ref("d1"),
                authority: AuthorityDto::Builtin {
                    name: "editor".to_string(),
                    version: 2,
                },
                ..base.clone()
            },
        ),
        (
            "custom role of another product than the target",
            CreateGrantV2Request {
                authority: AuthorityDto::Custom {
                    role_id: acta_role.id.0,
                },
                ..base.clone()
            },
        ),
        (
            "action outside the target's product",
            CreateGrantV2Request {
                authority: AuthorityDto::Actions {
                    actions: vec!["acta::document::read".to_string()],
                },
                ..base.clone()
            },
        ),
        (
            "unknown action",
            CreateGrantV2Request {
                authority: AuthorityDto::Actions {
                    actions: vec!["custos::group::fly".to_string()],
                },
                ..base.clone()
            },
        ),
        (
            "V1 plural scope is not a V2 action",
            CreateGrantV2Request {
                authority: AuthorityDto::Actions {
                    actions: vec!["custos::grants::read".to_string()],
                },
                ..base.clone()
            },
        ),
        (
            "mixed products in one action set",
            CreateGrantV2Request {
                authority: AuthorityDto::Actions {
                    actions: vec![
                        "custos::group::read".to_string(),
                        "acta::document::read".to_string(),
                    ],
                },
                ..base.clone()
            },
        ),
        (
            "empty action list",
            CreateGrantV2Request {
                authority: AuthorityDto::Actions { actions: vec![] },
                ..base.clone()
            },
        ),
    ];

    for (label, case) in cases {
        let err = admin
            .custos()
            .create_grant_v2(case)
            .await
            .expect_err("invalid grant must be rejected");
        assert_eq!(status_of(err), 422, "case: {label}");
    }
    assert!(
        admin
            .custos()
            .list_grants_v2("custos")
            .await
            .expect("list grants")
            .is_empty(),
        "no invalid grant was persisted for custos"
    );
    assert_eq!(
        admin
            .custos()
            .list_grants_v2("acta")
            .await
            .expect("list grants")
            .len(),
        2,
        "only the two accepted acta grants were persisted"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Denies
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deny_administration_is_409_while_the_mode_is_disabled_but_reads_still_work() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (admin, admin_id) = login_platform_admin(&server, &db).await;

    let refused = admin
        .custos()
        .create_deny(CreateDenyRequest {
            subject: principal_subject(),
            target: custos_ref("g1"),
            actions: vec!["custos::group::read".to_string()],
        })
        .await
        .expect_err("disabled mode refuses deny creation");
    assert_eq!(
        problem_type_of(refused),
        "urn:atlas:error:deny-mode-disabled"
    );

    let refused = admin
        .custos()
        .delete_deny(uuid::Uuid::now_v7())
        .await
        .expect_err("disabled mode refuses deny deletion");
    assert_eq!(status_of(refused), 409);

    assert!(
        admin
            .custos()
            .list_denies("custos")
            .await
            .expect("reads work in every mode")
            .is_empty()
    );
    assert!(audit_rows(&db, admin_id, "deny.").await.is_empty());

    db.teardown().await;
}

#[tokio::test]
async fn denies_are_created_listed_and_deleted_in_audit_mode_with_an_audit_row_per_mutation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = server_with_deny_mode(&db, DenyModeConfig::Audit).await;
    let (admin, admin_id) = login_platform_admin(&server, &db).await;
    let group_id = uuid::Uuid::now_v7();

    let created = admin
        .custos()
        .create_deny(CreateDenyRequest {
            subject: SubjectDto::Group { id: group_id },
            target: TargetDto::Selector {
                value: "custos::**".to_string(),
            },
            actions: vec!["custos::group::delete".to_string()],
        })
        .await
        .expect("create deny rule");
    assert_eq!(created.product, "custos");
    assert_eq!(created.subject, SubjectDto::Group { id: group_id });
    assert_eq!(created.actions, vec!["custos::group::delete"]);
    assert_eq!(created.created_by, admin_id);

    let listed = admin
        .custos()
        .list_denies("custos")
        .await
        .expect("list denies");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);

    let acta_principal = uuid::Uuid::now_v7();
    let on_acta = admin
        .custos()
        .create_deny(CreateDenyRequest {
            subject: SubjectDto::Principal { id: acta_principal },
            target: acta_ref("d1"),
            actions: vec!["acta::document::read".to_string()],
        })
        .await
        .expect("acta has published its catalog");
    assert_eq!(on_acta.product, "acta");

    for (label, invalid) in [
        (
            "empty actions",
            CreateDenyRequest {
                subject: principal_subject(),
                target: custos_ref("g1"),
                actions: vec![],
            },
        ),
        (
            "action outside the target's product",
            CreateDenyRequest {
                subject: principal_subject(),
                target: custos_ref("g1"),
                actions: vec!["acta::document::read".to_string()],
            },
        ),
        (
            "platform target (catalog not published)",
            CreateDenyRequest {
                subject: principal_subject(),
                target: TargetDto::Ref {
                    value: "platform::thing::t1".to_string(),
                },
                actions: vec!["platform::thing::read".to_string()],
            },
        ),
        (
            "unknown product",
            CreateDenyRequest {
                subject: principal_subject(),
                target: TargetDto::Ref {
                    value: "nope::group::g1".to_string(),
                },
                actions: vec!["nope::group::read".to_string()],
            },
        ),
    ] {
        let err = admin
            .custos()
            .create_deny(invalid)
            .await
            .expect_err("invalid deny must be rejected");
        assert_eq!(status_of(err), 422, "case: {label}");
    }

    admin
        .custos()
        .delete_deny(created.id)
        .await
        .expect("delete deny");
    assert_eq!(
        status_of(
            admin
                .custos()
                .delete_deny(created.id)
                .await
                .expect_err("already deleted")
        ),
        404
    );
    assert!(
        admin
            .custos()
            .list_denies("custos")
            .await
            .expect("list denies")
            .is_empty()
    );

    let rows = audit_rows(&db, admin_id, "deny.").await;
    let actions: Vec<&str> = rows.iter().map(|row| row.action.as_str()).collect();
    assert_eq!(actions, ["deny.created", "deny.created", "deny.deleted"]);
    for row in &rows {
        assert_eq!(row.target_type, "deny_rule");
    }
    for row in [&rows[0], &rows[2]] {
        assert_eq!(row.target_id, Some(created.id));
        assert_eq!(row.metadata["product"], "custos");
        assert_eq!(row.metadata["target"], "custos::**");
        assert_eq!(row.metadata["subject_kind"], "group");
        assert_eq!(row.metadata["subject"], group_id.to_string());
    }
    assert_eq!(rows[1].target_id, Some(on_acta.id));
    assert_eq!(rows[1].metadata["product"], "acta");
    assert_eq!(rows[1].metadata["target"], "acta::document::d1");
    assert_eq!(rows[1].metadata["subject_kind"], "principal");
    assert_eq!(rows[1].metadata["subject"], acta_principal.to_string());
    assert_eq!(
        rows[1].metadata["actions"],
        serde_json::json!(["acta::document::read"])
    );

    db.teardown().await;
}

/// A principal-set subject the catalog does not declare can never be stored,
/// as a grant or as a deny rule: the facts loader resolves only declared
/// sets, so an undeclared one would be a row nothing ever evaluates.
#[tokio::test]
async fn an_undeclared_principal_set_subject_is_422_on_grants_and_denies() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = server_with_deny_mode(&db, DenyModeConfig::Audit).await;
    let (admin, admin_id) = login_platform_admin(&server, &db).await;
    let undeclared = SubjectDto::PrincipalSet {
        id: "custos::group::g1::members".to_string(),
    };

    let grant = admin
        .custos()
        .create_grant_v2(CreateGrantV2Request {
            subject: undeclared.clone(),
            target: custos_ref("g1"),
            authority: explicit_custos_actions(),
        })
        .await
        .expect_err("custos declares no principal sets");
    assert_eq!(status_of(grant), 422);

    let deny = admin
        .custos()
        .create_deny(CreateDenyRequest {
            subject: undeclared,
            target: custos_ref("g1"),
            actions: vec!["custos::group::read".to_string()],
        })
        .await
        .expect_err("custos declares no principal sets");
    assert_eq!(status_of(deny), 422);

    assert!(
        admin
            .custos()
            .list_grants_v2("custos")
            .await
            .expect("list grants")
            .is_empty()
    );
    assert!(
        admin
            .custos()
            .list_denies("custos")
            .await
            .expect("list denies")
            .is_empty()
    );
    assert!(audit_rows(&db, admin_id, "").await.is_empty());

    db.teardown().await;
}
