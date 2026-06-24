#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! B2 + B3 audit-write integration tests.
//!
//! Each instrumented site gets two test cases:
//! 1. Happy path — mutation succeeds → exactly one audit row with correct fields.
//! 2. Rejection path — mutation is rejected (guard/invariant fires) → zero audit rows.
//!
//! The rejection tests prove the audit `append_in` sits inside the transaction and
//! rolls back together with the mutation.

mod support;

use atlas_api::dtos::{
    CreateGrantRequest, CreateProjectRequest, CreateUserRequest, GrantPrincipal,
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::MemberRole,
    entities::security_audit::AuditFilters,
    ids::{UserId, WorkspaceId},
};
use atlas_server::persistence::repos::{
    ActivationTokenRepo, ApiKeyRepo, MembershipRepo, NewActivationToken, NewApiKey, NewUser,
    NewWorkspace, PgSecurityAuditRepo, SecurityAuditRepo, UserRepo, WorkspaceRepo,
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

// ─── B3 platform helpers ──────────────────────────────────────────────────────

async fn platform_audit_rows(
    db: &TestDb,
) -> Vec<atlas_domain::entities::security_audit::SecurityAuditEvent> {
    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    repo.list_platform(&AuditFilters::default(), None, 200)
        .await
        .expect("list_platform")
}

async fn platform_audit_rows_for_action(
    db: &TestDb,
    action: &str,
) -> Vec<atlas_domain::entities::security_audit::SecurityAuditEvent> {
    platform_audit_rows(db)
        .await
        .into_iter()
        .filter(|r| r.action == action)
        .collect()
}

/// Creates an admin (non-root, is_system_admin=true) user and logs in as them.
async fn login_admin_user(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (
    atlas_client::AtlasClient,
    atlas_domain::entities::identity::User,
) {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: true,
        })
        .await
        .expect("create admin user");

    support::activate_user_in_db(db, user.id.0).await;

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login as admin");

    (client, user)
}

/// Creates a root user and logs in as them, returning the client and user record.
async fn login_root(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (
    atlas_client::AtlasClient,
    atlas_domain::entities::identity::User,
) {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: true,
            is_system_admin: false,
        })
        .await
        .expect("create root user");

    support::activate_user_in_db(db, user.id.0).await;

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login as root");

    (client, user)
}

/// Calls the raw `POST /v1/users/:id/system-admin` endpoint.
async fn set_system_admin_raw(
    client: &atlas_client::AtlasClient,
    user_id: uuid::Uuid,
    value: bool,
) -> Result<(), reqwest::StatusCode> {
    let resp = client
        .http_client()
        .post(format!(
            "{}/v1/users/{}/system-admin",
            client.base_url(),
            user_id
        ))
        .bearer_auth(client.token().unwrap_or(""))
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "is_system_admin": value }))
        .send()
        .await
        .expect("send");

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(resp.status())
    }
}

/// Calls `POST /v1/activate/:token` to activate an account.
async fn activate_account(server: &TestServer, token: &str) -> Result<(), reqwest::StatusCode> {
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/activate/{}", server.base_url(), token))
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "password": "ActivationPw1!" }))
        .send()
        .await
        .expect("send");

    if resp.status().is_success() {
        Ok(())
    } else {
        Err(resp.status())
    }
}

// ─── user.created ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_user_created_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, admin_user) = login_admin_user(&server, &db, "audit-uc-admin").await;

    let ws_id = WorkspaceId::new();
    let ws = db
        .workspace_repo()
        .create(NewWorkspace {
            id: ws_id,
            name: "audit-uc-ws".to_string(),
            slug: "audit-uc-ws".to_string(),
        })
        .await
        .expect("create ws");

    let ws_ctx = WorkspaceCtx::new(ws_id, Actor::User(admin_user.id));
    db.membership_repo()
        .add(&ws_ctx, admin_user.id, MemberRole::Owner)
        .await
        .expect("membership");

    let created = admin_client
        .create_user(CreateUserRequest {
            username: "audit-uc-newbie".to_string(),
            display_name: "Newbie".to_string(),
            email: None,
            role: "member".to_string(),
            workspace: ws.slug.clone(),
        })
        .await
        .expect("create_user");

    let rows = platform_audit_rows_for_action(&db, "user.created").await;

    assert_eq!(rows.len(), 1, "exactly one user.created audit row");

    let row = &rows[0];
    assert_eq!(
        row.workspace_id, None,
        "platform event: workspace_id is NULL"
    );
    assert_eq!(
        row.actor,
        Actor::User(admin_user.id),
        "actor is the calling admin"
    );
    assert_eq!(row.target_type, "user");
    assert_eq!(
        row.target_id,
        Some(created.user.id),
        "target is the new user"
    );
    assert_eq!(
        row.metadata.get("workspace_id").and_then(|v| v.as_str()),
        Some(ws.id.0.to_string().as_str()),
        "metadata contains the assigned workspace_id"
    );
    assert!(
        row.metadata
            .get("initial_role")
            .and_then(|v| v.as_str())
            .is_some(),
        "metadata contains initial_role"
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_user_created_duplicate_username_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, admin_user) = login_admin_user(&server, &db, "audit-uc-dup-admin").await;

    let ws_id = WorkspaceId::new();
    let ws = db
        .workspace_repo()
        .create(NewWorkspace {
            id: ws_id,
            name: "audit-uc-dup-ws".to_string(),
            slug: "audit-uc-dup-ws".to_string(),
        })
        .await
        .expect("create ws");

    let ws_ctx = WorkspaceCtx::new(ws_id, Actor::User(admin_user.id));
    db.membership_repo()
        .add(&ws_ctx, admin_user.id, MemberRole::Owner)
        .await
        .expect("membership");

    admin_client
        .create_user(CreateUserRequest {
            username: "audit-uc-dup-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            role: "member".to_string(),
            workspace: ws.slug.clone(),
        })
        .await
        .expect("first create_user");

    let result = admin_client
        .create_user(CreateUserRequest {
            username: "audit-uc-dup-target".to_string(),
            display_name: "Target Dup".to_string(),
            email: None,
            role: "member".to_string(),
            workspace: ws.slug.clone(),
        })
        .await;

    assert!(result.is_err(), "duplicate username must fail");

    let rows = platform_audit_rows_for_action(&db, "user.created").await;
    assert_eq!(
        rows.len(),
        1,
        "only one user.created row (the first successful one), not two"
    );

    db.teardown().await;
}

// ─── user.disabled ────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_user_disabled_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, admin_user) = login_admin_user(&server, &db, "audit-ud-admin").await;

    let target = db
        .user_repo()
        .create(NewUser {
            username: "audit-ud-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create target");

    admin_client
        .disable_user(target.id.0)
        .await
        .expect("disable_user");

    let rows = platform_audit_rows_for_action(&db, "user.disabled").await;

    assert_eq!(rows.len(), 1, "exactly one user.disabled audit row");

    let row = &rows[0];
    assert_eq!(row.workspace_id, None);
    assert_eq!(
        row.actor,
        Actor::User(admin_user.id),
        "actor is the calling admin"
    );
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(target.id.0));

    db.teardown().await;
}

#[tokio::test]
async fn audit_user_disabled_not_found_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, _admin_user) = login_admin_user(&server, &db, "audit-ud-reject-admin").await;

    let nonexistent = uuid::Uuid::now_v7();
    let result = admin_client.disable_user(nonexistent).await;

    assert!(result.is_err(), "disable non-existent user must fail");

    let rows = platform_audit_rows_for_action(&db, "user.disabled").await;
    assert_eq!(rows.len(), 0, "zero audit rows on not-found");

    db.teardown().await;
}

// ─── user.enabled ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_user_enabled_happy_path_writes_one_row_with_real_actor() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, admin_user) = login_admin_user(&server, &db, "audit-ue-admin").await;

    let target = db
        .user_repo()
        .create(NewUser {
            username: "audit-ue-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create target");

    // Disable then enable.
    admin_client
        .disable_user(target.id.0)
        .await
        .expect("disable first");
    admin_client
        .enable_user(target.id.0)
        .await
        .expect("enable_user");

    let rows = platform_audit_rows_for_action(&db, "user.enabled").await;

    assert_eq!(rows.len(), 1, "exactly one user.enabled audit row");

    let row = &rows[0];
    assert_eq!(row.workspace_id, None);
    // The extractor fix (rename _admin -> admin) ensures the real actor is captured, not a placeholder.
    assert_eq!(
        row.actor,
        Actor::User(admin_user.id),
        "actor must be the real calling admin (extractor-fix regression guard)"
    );
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(target.id.0));

    db.teardown().await;
}

#[tokio::test]
async fn audit_user_enabled_not_found_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, _admin_user) = login_admin_user(&server, &db, "audit-ue-reject-admin").await;

    let nonexistent = uuid::Uuid::now_v7();
    let result = admin_client.enable_user(nonexistent).await;

    assert!(result.is_err(), "enable non-existent user must fail");

    let rows = platform_audit_rows_for_action(&db, "user.enabled").await;
    assert_eq!(rows.len(), 0, "zero audit rows on not-found");

    db.teardown().await;
}

// ─── user.system_admin_set ────────────────────────────────────────────────────

#[tokio::test]
async fn audit_user_system_admin_set_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (root_client, root_user) = login_root(&server, &db, "audit-sa-root").await;

    let target = db
        .user_repo()
        .create(NewUser {
            username: "audit-sa-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create target");

    set_system_admin_raw(&root_client, target.id.0, true)
        .await
        .expect("set_system_admin");

    let rows = platform_audit_rows_for_action(&db, "user.system_admin_set").await;

    assert_eq!(rows.len(), 1, "exactly one user.system_admin_set audit row");

    let row = &rows[0];
    assert_eq!(row.workspace_id, None);
    assert_eq!(row.actor, Actor::User(root_user.id));
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(target.id.0));
    assert_eq!(
        row.metadata
            .get("is_system_admin")
            .and_then(|v| v.as_bool()),
        Some(true),
        "metadata records the new is_system_admin value"
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_user_system_admin_set_non_root_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, _admin_user) = login_admin_user(&server, &db, "audit-sa-reject-admin").await;

    let target = db
        .user_repo()
        .create(NewUser {
            username: "audit-sa-reject-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create target");

    let result = set_system_admin_raw(&admin_client, target.id.0, true).await;

    assert!(result.is_err(), "non-root must be rejected with 403");

    let rows = platform_audit_rows_for_action(&db, "user.system_admin_set").await;
    assert_eq!(rows.len(), 0, "zero audit rows when guard rejects");

    db.teardown().await;
}

// ─── user.password_reset ──────────────────────────────────────────────────────

#[tokio::test]
async fn audit_user_password_reset_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, admin_user) = login_admin_user(&server, &db, "audit-pr-admin").await;

    let target = db
        .user_repo()
        .create(NewUser {
            username: "audit-pr-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create target");

    admin_client
        .reset_user_password(target.id.0, "NewPassword1!")
        .await
        .expect("reset_user_password");

    let rows = platform_audit_rows_for_action(&db, "user.password_reset").await;

    assert_eq!(rows.len(), 1, "exactly one user.password_reset audit row");

    let row = &rows[0];
    assert_eq!(row.workspace_id, None);
    assert_eq!(row.actor, Actor::User(admin_user.id));
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(target.id.0));

    db.teardown().await;
}

#[tokio::test]
async fn audit_user_password_reset_not_found_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, _admin_user) = login_admin_user(&server, &db, "audit-pr-reject-admin").await;

    let nonexistent = uuid::Uuid::now_v7();
    let result = admin_client
        .reset_user_password(nonexistent, "NewPassword1!")
        .await;

    assert!(result.is_err(), "reset non-existent user must fail");

    let rows = platform_audit_rows_for_action(&db, "user.password_reset").await;
    assert_eq!(rows.len(), 0, "zero audit rows on not-found");

    db.teardown().await;
}

// ─── user.activation_regenerated ─────────────────────────────────────────────

#[tokio::test]
async fn audit_user_activation_regenerated_happy_path_writes_one_row_with_real_actor() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, admin_user) = login_admin_user(&server, &db, "audit-ar-admin").await;

    let target = db
        .user_repo()
        .create(NewUser {
            username: "audit-ar-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create target");

    // Seed an activation token so regenerate doesn't fail because the user
    // has no unconsumed tokens to invalidate.
    let token_repo = db.activation_token_repo();
    token_repo
        .create(NewActivationToken {
            user_id: target.id,
            token_hash: "initial-hash".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::days(7),
        })
        .await
        .expect("seed activation token");

    admin_client
        .regenerate_activation_link(target.id.0)
        .await
        .expect("regenerate_activation_link");

    let rows = platform_audit_rows_for_action(&db, "user.activation_regenerated").await;

    assert_eq!(
        rows.len(),
        1,
        "exactly one user.activation_regenerated audit row"
    );

    let row = &rows[0];
    assert_eq!(row.workspace_id, None);
    // The extractor fix (rename _admin -> admin) ensures the real actor is captured.
    assert_eq!(
        row.actor,
        Actor::User(admin_user.id),
        "actor must be the real calling admin (extractor-fix regression guard)"
    );
    assert_eq!(row.target_type, "user");
    assert_eq!(row.target_id, Some(target.id.0));

    db.teardown().await;
}

#[tokio::test]
async fn audit_user_activation_regenerated_already_activated_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, _admin_user) = login_admin_user(&server, &db, "audit-ar-reject-admin").await;

    let target = db
        .user_repo()
        .create(NewUser {
            username: "audit-ar-reject-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create target");

    // Mark the user as already activated.
    support::activate_user_in_db(&db, target.id.0).await;

    let result = admin_client.regenerate_activation_link(target.id.0).await;

    assert!(result.is_err(), "already-activated user must fail with 409");

    let rows = platform_audit_rows_for_action(&db, "user.activation_regenerated").await;
    assert_eq!(
        rows.len(),
        0,
        "zero audit rows when user is already activated"
    );

    db.teardown().await;
}

// ─── api_key.created ──────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_api_key_created_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user_client, user) = login_admin_user(&server, &db, "audit-akc-user").await;

    let created = user_client
        .create_user_api_key(atlas_api::dtos::CreateUserApiKeyRequest {
            name: "audit-akc-key".to_string(),
            r#type: Some("agent".to_string()),
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create_user_api_key");

    let rows = platform_audit_rows_for_action(&db, "api_key.created").await;

    assert_eq!(rows.len(), 1, "exactly one api_key.created audit row");

    let row = &rows[0];
    assert_eq!(row.workspace_id, None, "api_key events are platform-scoped");
    assert_eq!(row.actor, Actor::User(user.id));
    assert_eq!(row.target_type, "api_key");
    assert_eq!(row.target_id, Some(created.id), "target is the new key id");
    assert_eq!(
        row.metadata.get("key_type").and_then(|v| v.as_str()),
        Some("agent")
    );
    assert!(
        row.metadata
            .get("key_name")
            .and_then(|v| v.as_str())
            .is_some(),
        "metadata contains key_name"
    );

    db.teardown().await;
}

#[tokio::test]
async fn audit_api_key_created_by_api_key_principal_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user_client, user) = login_admin_user(&server, &db, "audit-akc-reject-user").await;

    // Create a key first, then use it to try creating another key (not allowed).
    let key = user_client
        .create_user_api_key(atlas_api::dtos::CreateUserApiKeyRequest {
            name: "audit-akc-reject-key".to_string(),
            r#type: Some("agent".to_string()),
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create initial api key");

    // Use the API key to try creating another key — should fail (API keys
    // cannot create other API keys).
    let key_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key.secret.clone());

    let result = key_client
        .create_user_api_key(atlas_api::dtos::CreateUserApiKeyRequest {
            name: "should-fail".to_string(),
            r#type: Some("agent".to_string()),
            expires_at: None,
            initial_grant: None,
        })
        .await;

    assert!(result.is_err(), "api key principal cannot create api keys");

    // Only the first successful creation should have produced an audit row.
    let rows = platform_audit_rows_for_action(&db, "api_key.created").await;
    assert_eq!(
        rows.len(),
        1,
        "only the first successful creation is audited"
    );
    assert_eq!(rows[0].actor, Actor::User(user.id));

    db.teardown().await;
}

// ─── api_key.revoked ──────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_api_key_revoked_happy_path_writes_one_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user_client, user) = login_admin_user(&server, &db, "audit-akr-user").await;

    let key = user_client
        .create_user_api_key(atlas_api::dtos::CreateUserApiKeyRequest {
            name: "audit-akr-key".to_string(),
            r#type: Some("agent".to_string()),
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create_user_api_key");

    user_client
        .revoke_user_api_key(key.id)
        .await
        .expect("revoke_user_api_key");

    let rows = platform_audit_rows_for_action(&db, "api_key.revoked").await;

    assert_eq!(rows.len(), 1, "exactly one api_key.revoked audit row");

    let row = &rows[0];
    assert_eq!(row.workspace_id, None);
    assert_eq!(row.actor, Actor::User(user.id));
    assert_eq!(row.target_type, "api_key");
    assert_eq!(row.target_id, Some(key.id));

    db.teardown().await;
}

#[tokio::test]
async fn audit_api_key_revoked_not_found_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user_client, _user) = login_admin_user(&server, &db, "audit-akr-reject-user").await;

    let nonexistent = uuid::Uuid::now_v7();
    let result = user_client.revoke_user_api_key(nonexistent).await;

    assert!(result.is_err(), "revoke non-existent key must fail");

    let rows = platform_audit_rows_for_action(&db, "api_key.revoked").await;
    assert_eq!(rows.len(), 0, "zero audit rows on not-found");

    db.teardown().await;
}

// ─── account.activated ────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_account_activated_happy_path_writes_one_row_actor_is_self() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (admin_client, admin_user) = login_admin_user(&server, &db, "audit-aa-admin").await;

    let ws_id = WorkspaceId::new();
    let ws = db
        .workspace_repo()
        .create(NewWorkspace {
            id: ws_id,
            name: "audit-aa-ws".to_string(),
            slug: "audit-aa-ws".to_string(),
        })
        .await
        .expect("create ws");

    let ws_ctx = WorkspaceCtx::new(ws_id, Actor::User(admin_user.id));
    db.membership_repo()
        .add(&ws_ctx, admin_user.id, MemberRole::Owner)
        .await
        .expect("membership");

    let created = admin_client
        .create_user(CreateUserRequest {
            username: "audit-aa-newbie".to_string(),
            display_name: "Newbie".to_string(),
            email: None,
            role: "member".to_string(),
            workspace: ws.slug.clone(),
        })
        .await
        .expect("create_user");

    // Extract the token from the activation link (format: ".../activate/<token>").
    let token = created
        .activation_link
        .rsplit('/')
        .next()
        .expect("token from link")
        .to_string();

    activate_account(&server, &token).await.expect("activate");

    let rows = platform_audit_rows_for_action(&db, "account.activated").await;

    assert_eq!(rows.len(), 1, "exactly one account.activated audit row");

    let row = &rows[0];
    assert_eq!(row.workspace_id, None);
    // Actor is the activating user themselves (self-service activation).
    assert_eq!(
        row.target_id,
        Some(created.user.id),
        "target is the user who activated"
    );
    assert_eq!(
        row.actor,
        Actor::User(UserId(created.user.id)),
        "actor == target: self-activation"
    );
    assert_eq!(row.target_type, "user");

    db.teardown().await;
}

#[tokio::test]
async fn audit_account_activated_bad_token_writes_zero_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let result = activate_account(&server, "invalid-token-xyz").await;

    assert!(result.is_err(), "bad token must fail with 404");

    let rows = platform_audit_rows_for_action(&db, "account.activated").await;
    assert_eq!(rows.len(), 0, "zero audit rows on bad token");

    db.teardown().await;
}
