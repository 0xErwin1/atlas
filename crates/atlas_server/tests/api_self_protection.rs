//! B2 — Self-protection guards TDD integration tests.
//!
//! Guards fire unconditionally at the handler level, before any break-glass or
//! bypass logic. A caller must NOT modify their own privilege-defining
//! attributes. Another admin must do it.
//!
//! RED direction: proves that a caller targeting themselves is rejected 403.
//! GREEN/non-self direction: proves no regression — a different admin can still
//! perform the same operation on another user.
//! COMPOSITION direction: proves self-protection wins over the B1 global-admin
//! bypass — a system_admin (break-glass) is still blocked from self-role-change.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::LoginRequest;
use atlas_client::{AtlasClient, ClientError};
use atlas_domain::entities::security_audit::AuditFilters;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{
    MembershipRepo, NewUser, PgSecurityAuditRepo, SecurityAuditRepo, UserRepo,
};
use support::{TestDb, TestServer, login_user_with_workspace};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a fully activated system_admin user with a real password, logs in,
/// and returns the authenticated client together with the user record.
async fn create_and_login_system_admin(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (AtlasClient, atlas_domain::entities::identity::User) {
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
        .expect("create system admin");

    support::activate_user_in_db(db, user.id.0).await;

    let mut client = AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login system admin");

    (client, user)
}

/// Adds a user as a workspace member without login capability.
async fn add_member(
    db: &TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
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

fn assert_403(result: Result<impl std::fmt::Debug, ClientError>, context: &str) {
    match result {
        Err(ClientError::Api(p)) => assert_eq!(
            p.status, 403,
            "{context}: expected 403 but got {}: {}",
            p.status, p.title
        ),
        other => panic!("{context}: expected 403, got {other:?}"),
    }
}

/// Counts platform-scoped (workspace_id IS NULL) audit rows — used for
/// disable/enable events which record no workspace.
async fn count_platform_audit_rows(db: &TestDb) -> usize {
    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    repo.list_platform(&AuditFilters::default(), None, 100)
        .await
        .expect("list_platform")
        .len()
}

// ---------------------------------------------------------------------------
// B-1: disable_user self-guard
// ---------------------------------------------------------------------------

/// RED: an admin who tries to disable their own account gets 403.
///
/// The self-check fires before any privileged target check, so even a root
/// user (who can disable system-admins) cannot disable themselves.
#[tokio::test]
async fn self_disable_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let me = root.me().await.expect("me");
    let my_id = me.id.expect("me.id must be present for a user session");

    let result = root.disable_user(my_id).await;

    assert_403(result, "self-disable");

    db.teardown().await;
}

/// GREEN/non-self: a root user can still disable a DIFFERENT user — no regression.
#[tokio::test]
async fn root_disables_other_user_succeeds() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let (_, _, target) = login_user_with_workspace(&server, &db, "sp-dis-target").await;

    root.disable_user(target.id.0)
        .await
        .expect("root disabling another user must succeed");

    db.teardown().await;
}

/// AUDIT: self-disable is rejected before the transaction commits, so zero
/// audit rows are written to the platform log.
#[tokio::test]
async fn self_disable_writes_zero_audit_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let me = root.me().await.expect("me");
    let my_id = me.id.expect("me.id must be present");

    let _ = root.disable_user(my_id).await;

    assert_eq!(
        count_platform_audit_rows(&db).await,
        0,
        "self-disable must not write audit rows"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B-3: enable_user self-guard (symmetry)
// ---------------------------------------------------------------------------

/// RED: an admin who tries to enable their own account gets 403.
///
/// In practice a logged-in admin is never disabled, so the "enable self"
/// path is normally unreachable — but the guard must exist for symmetry and
/// to prevent a race condition (e.g. enable issued while another admin
/// concurrently disables).
#[tokio::test]
async fn self_enable_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let me = root.me().await.expect("me");
    let my_id = me.id.expect("me.id must be present");

    let result = root.enable_user(my_id).await;

    assert_403(result, "self-enable");

    db.teardown().await;
}

/// GREEN/non-self: a root user can still enable a DIFFERENT disabled user.
#[tokio::test]
async fn root_enables_other_user_succeeds() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let (_, _, target) = login_user_with_workspace(&server, &db, "sp-en-target").await;

    root.disable_user(target.id.0).await.expect("disable first");
    root.enable_user(target.id.0)
        .await
        .expect("root enabling another user must succeed");

    db.teardown().await;
}

/// AUDIT: self-enable is rejected before the transaction, so zero audit rows.
#[tokio::test]
async fn self_enable_writes_zero_audit_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let me = root.me().await.expect("me");
    let my_id = me.id.expect("me.id must be present");

    let _ = root.enable_user(my_id).await;

    assert_eq!(
        count_platform_audit_rows(&db).await,
        0,
        "self-enable must not write audit rows"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B-2: update_member_role self-guard
// ---------------------------------------------------------------------------

/// RED: a workspace owner who tries to change their OWN role gets 403.
#[tokio::test]
async fn self_role_change_owner_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) =
        login_user_with_workspace(&server, &db, "sp-rc-self-owner").await;

    // Add a second owner so the last-owner lockout doesn't mask the self-guard.
    add_member(&db, ws.id, "sp-rc-self-owner2", MemberRole::Owner).await;

    let result = owner_client
        .update_member_role(&ws.slug, owner_user.id.0, "admin")
        .await;

    assert_403(result, "owner self-role-change");

    db.teardown().await;
}

/// RED: a workspace admin who tries to change their OWN role gets 403.
#[tokio::test]
async fn self_role_change_admin_returns_403() {
    use atlas_server::auth::password;

    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "sp-rc-self-admin-owner").await;

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    let admin_user = db
        .user_repo()
        .create(NewUser {
            username: "sp-rc-self-admin".to_string(),
            display_name: "sp-rc-self-admin".to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create admin user");

    support::activate_user_in_db(&db, admin_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(admin_user.id));
    db.membership_repo()
        .add(&ctx, admin_user.id, MemberRole::Admin)
        .await
        .expect("add admin membership");

    let mut admin_client = AtlasClient::new(server.base_url().to_string());
    admin_client
        .login(LoginRequest {
            username: "sp-rc-self-admin".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    let result = admin_client
        .update_member_role(&ws.slug, admin_user.id.0, "member")
        .await;

    assert_403(result, "admin self-role-change");

    db.teardown().await;
}

/// GREEN/non-self: a workspace owner can still change ANOTHER member's role.
#[tokio::test]
async fn owner_changes_other_member_role_succeeds() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "sp-rc-other-owner").await;

    let target = add_member(&db, ws.id, "sp-rc-other-target", MemberRole::Member).await;

    owner_client
        .update_member_role(&ws.slug, target.id.0, "admin")
        .await
        .expect("owner changing another member's role must succeed");

    db.teardown().await;
}

/// AUDIT: self-role-change is rejected before the transaction, so zero workspace
/// audit rows.
#[tokio::test]
async fn self_role_change_writes_zero_audit_rows() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) =
        login_user_with_workspace(&server, &db, "sp-rc-audit-owner").await;

    // A second owner prevents last-owner lockout from masking the self-guard.
    add_member(&db, ws.id, "sp-rc-audit-owner2", MemberRole::Owner).await;

    let result = owner_client
        .update_member_role(&ws.slug, owner_user.id.0, "admin")
        .await;

    assert!(result.is_err(), "self-role-change must fail");

    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    let rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 100)
        .await
        .expect("list_for_workspace");

    assert_eq!(rows.len(), 0, "self-role-change must not write audit rows");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// COMPOSITION: self-protection wins over the B1 global-admin bypass
// ---------------------------------------------------------------------------

/// A system_admin (break-glass, non-member) trying to change THEIR OWN
/// workspace role gets 403. The self-guard fires at the handler, before any
/// WorkspaceOwnerOrAdmin break-glass bypass takes effect.
#[tokio::test]
async fn system_admin_self_role_change_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "sp-comp-sysadmin-owner").await;

    // The system_admin is a MEMBER of this workspace so that the route finds
    // their membership record (otherwise it would hit 404 on target lookup,
    // which would mask the 403 we expect from the self-guard).
    let (sysadmin_client, sysadmin_user) =
        create_and_login_system_admin(&server, &db, "sp-comp-sysadmin").await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(sysadmin_user.id));
    db.membership_repo()
        .add(&ctx, sysadmin_user.id, MemberRole::Admin)
        .await
        .expect("add sysadmin membership");

    let result = sysadmin_client
        .update_member_role(&ws.slug, sysadmin_user.id.0, "member")
        .await;

    assert_403(result, "system_admin self-role-change (composition)");

    db.teardown().await;
}

/// A system_admin (break-glass, non-member) CAN change ANOTHER member's role
/// in any workspace — the B1 global-admin bypass is not affected.
///
/// This is the non-self regression direction for the composition test.
#[tokio::test]
async fn system_admin_changes_other_member_role_succeeds() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "sp-comp-bypass-owner").await;

    let target = add_member(&db, ws.id, "sp-comp-bypass-target", MemberRole::Member).await;

    // Another owner so demoting the target doesn't cause a last-owner lockout.
    add_member(&db, ws.id, "sp-comp-bypass-owner2", MemberRole::Owner).await;

    let (sysadmin_client, _) =
        create_and_login_system_admin(&server, &db, "sp-comp-bypass-sysadmin").await;

    sysadmin_client
        .update_member_role(&ws.slug, target.id.0, "admin")
        .await
        .expect("system_admin changing another member's role must succeed via B1 bypass");

    db.teardown().await;
}

/// B-4 regression lock: set_system_admin already blocks self-modification.
/// Root attempting to set_system_admin on themselves must fail (not 200).
///
/// The existing guard returns 400 BadRequest, not 403 — that is the
/// pre-existing behaviour and must not regress.
#[tokio::test]
async fn set_system_admin_self_is_blocked() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let me = root.me().await.expect("me");
    let my_id = me.id.expect("me.id must be present");

    let response = root
        .http_client()
        .post(format!(
            "{}/api/users/{}/system-admin",
            root.base_url(),
            my_id,
        ))
        .bearer_auth(root.token().unwrap_or(""))
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "is_system_admin": true }))
        .send()
        .await
        .expect("send");

    assert_ne!(
        response.status().as_u16(),
        200,
        "set_system_admin self must not succeed"
    );

    db.teardown().await;
}
