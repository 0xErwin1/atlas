#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! B2 audit-write integration tests.
//!
//! Each instrumented site gets two test cases:
//! 1. Happy path — mutation succeeds → exactly one audit row with correct fields.
//! 2. Rejection path — mutation is rejected (guard/invariant fires) → zero audit rows.
//!
//! The rejection tests prove the audit `append_in` sits inside the transaction and
//! rolls back together with the mutation.

mod support;

use atlas_api::dtos::{CreateGrantRequest, CreateProjectRequest, GrantPrincipal};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::MemberRole,
    entities::security_audit::AuditFilters,
    ids::{UserId, WorkspaceId},
};
use atlas_server::persistence::repos::{
    ApiKeyRepo, MembershipRepo, NewApiKey, NewUser, PgSecurityAuditRepo, SecurityAuditRepo,
    UserRepo,
};
use support::{TestDb, TestServer, login_user_with_workspace};

// ─── helpers ─────────────────────────────────────────────────────────────────

async fn count_workspace_audit_rows(db: &TestDb, ws_id: WorkspaceId) -> usize {
    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    repo.list_for_workspace(ws_id, &AuditFilters::default(), None, 100)
        .await
        .expect("list_for_workspace")
        .len()
}

async fn audit_rows_for_workspace(
    db: &TestDb,
    ws_id: WorkspaceId,
) -> Vec<atlas_domain::entities::security_audit::SecurityAuditEvent> {
    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    repo.list_for_workspace(ws_id, &AuditFilters::default(), None, 100)
        .await
        .expect("list_for_workspace")
}

async fn add_member_to_ws(
    db: &TestDb,
    ws_id: WorkspaceId,
    username: &str,
    role: MemberRole,
) -> atlas_domain::entities::identity::User {
    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(user.id));
    db.membership_repo()
        .add(&ctx, user.id, role)
        .await
        .expect("add membership");

    user
}

async fn login_owner(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (
    atlas_client::AtlasClient,
    atlas_server::persistence::repos::Workspace,
    atlas_domain::entities::identity::User,
) {
    login_user_with_workspace(server, db, username).await
}

fn user_grant_req(user_id: uuid::Uuid, role: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "user".to_string(),
            id: user_id,
        },
        role: role.to_string(),
    }
}

fn agent_grant_req(key_id: uuid::Uuid, role: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "api_key".to_string(),
            id: key_id,
        },
        role: role.to_string(),
    }
}

async fn create_agent_key(
    db: &TestDb,
    ws_id: WorkspaceId,
    creator: UserId,
    name: &str,
) -> atlas_domain::entities::identity::ApiKey {
    let ctx = WorkspaceCtx::new(ws_id, Actor::User(creator));
    db.api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: name.to_string(),
                token_hash: format!("hash-{name}"),
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
            },
        )
        .await
        .expect("create api key")
}

// ─── membership.role_changed ──────────────────────────────────────────────────

#[tokio::test]
async fn audit_membership_role_changed_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) = login_owner(&server, &db, "audit-rc-happy").await;

    let target = add_member_to_ws(&db, ws.id, "audit-rc-happy-target", MemberRole::Member).await;

    owner_client
        .update_member_role(&ws.slug, target.id.0, "admin")
        .await
        .expect("update_member_role");

    let rows = audit_rows_for_workspace(&db, ws.id).await;

    assert_eq!(rows.len(), 1, "exactly one audit row after role change");

    let row = &rows[0];
    assert_eq!(row.action, "membership.role_changed");
    assert_eq!(row.workspace_id, Some(ws.id));
    assert_eq!(row.actor, Actor::User(owner_user.id));
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(target.id.0));
    assert_eq!(
        row.metadata.get("old_role").and_then(|v| v.as_str()),
        Some("member"),
        "old_role must be the role before the change"
    );
    assert_eq!(
        row.metadata.get("new_role").and_then(|v| v.as_str()),
        Some("admin"),
        "new_role must be the role after the change"
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_membership_role_changed_rejected_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) = login_owner(&server, &db, "audit-rc-reject").await;

    let nonexistent_user_id = uuid::Uuid::now_v7();
    let result = owner_client
        .update_member_role(&ws.slug, nonexistent_user_id, "admin")
        .await;

    // Target is not a member → 404.
    assert!(result.is_err(), "must fail for non-member target");

    assert_eq!(
        count_workspace_audit_rows(&db, ws.id).await,
        0,
        "zero audit rows on rejection"
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_membership_role_changed_last_owner_lockout_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) = login_owner(&server, &db, "audit-rc-last-owner").await;

    let result = owner_client
        .update_member_role(&ws.slug, owner_user.id.0, "member")
        .await;

    // Last-owner lockout → 409.
    assert!(result.is_err(), "last-owner lockout must fail");

    assert_eq!(
        count_workspace_audit_rows(&db, ws.id).await,
        0,
        "zero audit rows on last-owner lockout"
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_membership_role_changed_admin_on_owner_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner_client, ws, owner_user) =
        login_owner(&server, &db, "audit-rc-admin-owner-403").await;

    let (admin_client, _admin_user) = {
        use atlas_api::dtos::LoginRequest;
        use atlas_server::auth::password;
        let hash = password::hash("TestPassword1!".to_string())
            .await
            .expect("hash");
        let user = db
            .user_repo()
            .create(NewUser {
                username: "audit-rc-admin-403-caller".to_string(),
                display_name: "admin".to_string(),
                email: None,
                password_hash: Some(hash),
                is_root: false,
                is_system_admin: false,
            })
            .await
            .expect("create admin user");
        support::activate_user_in_db(&db, user.id.0).await;
        let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
        db.membership_repo()
            .add(&ctx, user.id, MemberRole::Admin)
            .await
            .expect("add admin membership");
        let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
        client
            .login(LoginRequest {
                username: "audit-rc-admin-403-caller".to_string(),
                password: "TestPassword1!".to_string(),
            })
            .await
            .expect("login");
        (client, user)
    };

    let result = admin_client
        .update_member_role(&ws.slug, owner_user.id.0, "member")
        .await;

    // Admin cannot modify owner → 403.
    assert!(result.is_err(), "must fail 403");

    assert_eq!(
        count_workspace_audit_rows(&db, ws.id).await,
        0,
        "zero audit rows when admin is blocked from touching an owner"
    );

    db.teardown().await;
}

// ─── membership.removed ──────────────────────────────────────────────────────

#[tokio::test]
async fn audit_membership_removed_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) = login_owner(&server, &db, "audit-mr-happy").await;

    let target = add_member_to_ws(&db, ws.id, "audit-mr-happy-target", MemberRole::Member).await;

    owner_client
        .remove_member(&ws.slug, target.id.0)
        .await
        .expect("remove_member");

    let rows = audit_rows_for_workspace(&db, ws.id).await;

    assert_eq!(rows.len(), 1, "exactly one audit row after member removal");

    let row = &rows[0];
    assert_eq!(row.action, "membership.removed");
    assert_eq!(row.workspace_id, Some(ws.id));
    assert_eq!(row.actor, Actor::User(owner_user.id));
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(target.id.0));
    assert_eq!(
        row.metadata.get("role").and_then(|v| v.as_str()),
        Some("member"),
        "metadata must record the role they held before removal"
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_membership_removed_last_owner_lockout_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) = login_owner(&server, &db, "audit-mr-last-owner").await;

    let result = owner_client.remove_member(&ws.slug, owner_user.id.0).await;

    // Last-owner lockout → 409.
    assert!(result.is_err(), "last-owner lockout must fail");

    assert_eq!(
        count_workspace_audit_rows(&db, ws.id).await,
        0,
        "zero audit rows on last-owner removal lockout"
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_membership_removed_not_member_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) = login_owner(&server, &db, "audit-mr-not-member").await;

    let stranger = db
        .user_repo()
        .create(NewUser {
            username: "audit-mr-not-member-stranger".into(),
            display_name: "stranger".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create stranger");

    let result = owner_client.remove_member(&ws.slug, stranger.id.0).await;
    assert!(result.is_err(), "non-member target must fail");

    assert_eq!(
        count_workspace_audit_rows(&db, ws.id).await,
        0,
        "zero audit rows when target is not a member"
    );

    db.teardown().await;
}

// ─── grant.created (project scope) ──────────────────────────────────────────

#[tokio::test]
async fn audit_project_grant_created_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) = login_owner(&server, &db, "audit-pgc-happy").await;

    let project = owner_client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "audit-pgc-proj".to_string(),
                slug: "audit-pgc-proj".to_string(),
                task_prefix: "APC".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let grantee = add_member_to_ws(&db, ws.id, "audit-pgc-happy-grantee", MemberRole::Member).await;

    owner_client
        .create_project_grant(
            &ws.slug,
            &project.slug,
            user_grant_req(grantee.id.0, "viewer"),
        )
        .await
        .expect("create_project_grant");

    let rows = audit_rows_for_workspace(&db, ws.id).await;

    let grant_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "grant.created")
        .collect();

    assert_eq!(grant_rows.len(), 1, "exactly one grant.created audit row");

    let row = grant_rows[0];
    assert_eq!(row.workspace_id, Some(ws.id));
    assert_eq!(row.actor, Actor::User(owner_user.id));
    assert_eq!(row.target_type, "grant");
    assert!(row.target_id.is_some(), "target_id must be the grant UUID");
    assert_eq!(
        row.metadata.get("resource_type").and_then(|v| v.as_str()),
        Some("project"),
    );
    assert_eq!(
        row.metadata.get("role").and_then(|v| v.as_str()),
        Some("viewer"),
    );
    assert_eq!(
        row.metadata.get("grantee_type").and_then(|v| v.as_str()),
        Some("user"),
    );
    assert_eq!(
        row.metadata
            .get("grantee_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok()),
        Some(grantee.id.0),
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_project_grant_created_non_member_grantee_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) = login_owner(&server, &db, "audit-pgc-reject").await;

    let project = owner_client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "audit-pgc-rej-proj".to_string(),
                slug: "audit-pgc-rej-proj".to_string(),
                task_prefix: "APR".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let stranger_id = uuid::Uuid::now_v7();

    let result = owner_client
        .create_project_grant(
            &ws.slug,
            &project.slug,
            user_grant_req(stranger_id, "viewer"),
        )
        .await;

    assert!(result.is_err(), "non-member grantee must be rejected");

    let rows = audit_rows_for_workspace(&db, ws.id).await;
    let grant_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "grant.created")
        .collect();
    assert_eq!(grant_rows.len(), 0, "zero grant.created rows on rejection");

    db.teardown().await;
}

// ─── grant.revoked (project scope) ──────────────────────────────────────────

#[tokio::test]
async fn audit_project_grant_revoked_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) = login_owner(&server, &db, "audit-pgr-happy").await;

    let project = owner_client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "audit-pgr-proj".to_string(),
                slug: "audit-pgr-proj".to_string(),
                task_prefix: "PGR".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let grantee = add_member_to_ws(&db, ws.id, "audit-pgr-happy-grantee", MemberRole::Member).await;

    let grants_before = owner_client
        .list_project_grants(&ws.slug, &project.slug, None, None)
        .await
        .expect("list_project_grants");

    owner_client
        .create_project_grant(
            &ws.slug,
            &project.slug,
            user_grant_req(grantee.id.0, "viewer"),
        )
        .await
        .expect("create grant");

    let grants_after = owner_client
        .list_project_grants(&ws.slug, &project.slug, None, None)
        .await
        .expect("list_project_grants after create");

    let new_grant = grants_after
        .items
        .into_iter()
        .find(|g| !grants_before.items.iter().any(|b| b.id == g.id))
        .expect("new grant not found");

    // Clear audit rows from create before testing revoke.
    let rows_before_revoke = audit_rows_for_workspace(&db, ws.id).await;
    let create_row_count = rows_before_revoke.len();

    owner_client
        .delete_project_grant(&ws.slug, &project.slug, new_grant.id)
        .await
        .expect("delete_project_grant");

    let rows = audit_rows_for_workspace(&db, ws.id).await;
    let revoke_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "grant.revoked")
        .collect();

    assert_eq!(
        revoke_rows.len(),
        1,
        "exactly one grant.revoked audit row; total rows = {}, create rows = {create_row_count}",
        rows.len()
    );

    let row = revoke_rows[0];
    assert_eq!(row.workspace_id, Some(ws.id));
    assert_eq!(row.actor, Actor::User(owner_user.id));
    assert_eq!(row.target_type, "grant");
    assert_eq!(row.target_id, Some(new_grant.id));
    assert_eq!(
        row.metadata.get("resource_type").and_then(|v| v.as_str()),
        Some("project"),
    );
    assert_eq!(
        row.metadata.get("grantee_type").and_then(|v| v.as_str()),
        Some("user"),
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_project_grant_revoked_not_found_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) = login_owner(&server, &db, "audit-pgr-reject").await;

    let project = owner_client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "audit-pgr-rej-proj".to_string(),
                slug: "audit-pgr-rej-proj".to_string(),
                task_prefix: "PRR".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let nonexistent_grant_id = uuid::Uuid::now_v7();

    let result = owner_client
        .delete_project_grant(&ws.slug, &project.slug, nonexistent_grant_id)
        .await;

    assert!(result.is_err(), "non-existent grant delete must fail");

    let rows = audit_rows_for_workspace(&db, ws.id).await;
    let revoke_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "grant.revoked")
        .collect();
    assert_eq!(
        revoke_rows.len(),
        0,
        "zero grant.revoked rows on not-found rejection"
    );

    db.teardown().await;
}

// ─── grant.created (workspace scope) ─────────────────────────────────────────

#[tokio::test]
async fn audit_workspace_grant_created_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) = login_owner(&server, &db, "audit-wgc-happy").await;

    let agent_key = create_agent_key(&db, ws.id, owner_user.id, "audit-wgc-key").await;

    owner_client
        .create_workspace_grant(&ws.slug, agent_grant_req(agent_key.id.0, "editor"))
        .await
        .expect("create_workspace_grant");

    let rows = audit_rows_for_workspace(&db, ws.id).await;
    let grant_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "grant.created")
        .collect();

    assert_eq!(grant_rows.len(), 1, "exactly one grant.created audit row");

    let row = grant_rows[0];
    assert_eq!(row.workspace_id, Some(ws.id));
    assert_eq!(row.actor, Actor::User(owner_user.id));
    assert_eq!(row.target_type, "grant");
    assert!(row.target_id.is_some(), "target_id must be the grant UUID");
    assert_eq!(
        row.metadata.get("resource_type").and_then(|v| v.as_str()),
        Some("workspace"),
    );
    assert_eq!(
        row.metadata.get("role").and_then(|v| v.as_str()),
        Some("editor"),
    );
    assert_eq!(
        row.metadata.get("grantee_type").and_then(|v| v.as_str()),
        Some("api_key"),
    );
    assert_eq!(
        row.metadata
            .get("grantee_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok()),
        Some(agent_key.id.0),
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_workspace_grant_created_non_member_user_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) = login_owner(&server, &db, "audit-wgc-reject").await;

    let stranger_id = uuid::Uuid::now_v7();

    let result = owner_client
        .create_workspace_grant(&ws.slug, user_grant_req(stranger_id, "editor"))
        .await;

    assert!(result.is_err(), "non-member user grant must fail");

    let rows = audit_rows_for_workspace(&db, ws.id).await;
    let grant_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "grant.created")
        .collect();
    assert_eq!(grant_rows.len(), 0, "zero grant.created rows on rejection");

    db.teardown().await;
}

// ─── grant.revoked (workspace scope) ─────────────────────────────────────────

#[tokio::test]
async fn audit_workspace_grant_revoked_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) = login_owner(&server, &db, "audit-wgr-happy").await;

    let agent_key = create_agent_key(&db, ws.id, owner_user.id, "audit-wgr-key").await;

    let grants_before = owner_client
        .list_workspace_grants(&ws.slug, None, None)
        .await
        .expect("list_workspace_grants before");

    owner_client
        .create_workspace_grant(&ws.slug, agent_grant_req(agent_key.id.0, "editor"))
        .await
        .expect("create_workspace_grant");

    let grants_after = owner_client
        .list_workspace_grants(&ws.slug, None, None)
        .await
        .expect("list_workspace_grants after create");

    let new_grant = grants_after
        .items
        .into_iter()
        .find(|g| !grants_before.items.iter().any(|b| b.id == g.id))
        .expect("new grant not found");

    let rows_before_revoke = audit_rows_for_workspace(&db, ws.id).await;
    let create_row_count = rows_before_revoke
        .iter()
        .filter(|r| r.action == "grant.created")
        .count();

    owner_client
        .delete_workspace_grant(&ws.slug, new_grant.id)
        .await
        .expect("delete_workspace_grant");

    let rows = audit_rows_for_workspace(&db, ws.id).await;
    let revoke_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "grant.revoked")
        .collect();

    assert_eq!(
        revoke_rows.len(),
        1,
        "exactly one grant.revoked audit row; create rows = {create_row_count}"
    );

    let row = revoke_rows[0];
    assert_eq!(row.workspace_id, Some(ws.id));
    assert_eq!(row.actor, Actor::User(owner_user.id));
    assert_eq!(row.target_type, "grant");
    assert_eq!(row.target_id, Some(new_grant.id));
    assert_eq!(
        row.metadata.get("resource_type").and_then(|v| v.as_str()),
        Some("workspace"),
    );
    assert_eq!(
        row.metadata.get("grantee_type").and_then(|v| v.as_str()),
        Some("api_key"),
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_workspace_grant_revoked_not_found_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) = login_owner(&server, &db, "audit-wgr-reject").await;

    let nonexistent_grant_id = uuid::Uuid::now_v7();

    let result = owner_client
        .delete_workspace_grant(&ws.slug, nonexistent_grant_id)
        .await;

    assert!(result.is_err(), "non-existent grant delete must fail");

    let rows = audit_rows_for_workspace(&db, ws.id).await;
    let revoke_rows: Vec<_> = rows
        .iter()
        .filter(|r| r.action == "grant.revoked")
        .collect();
    assert_eq!(
        revoke_rows.len(),
        0,
        "zero grant.revoked rows on not-found rejection"
    );

    db.teardown().await;
}
