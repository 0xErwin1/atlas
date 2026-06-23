#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{CreateUserRequest, UserDto};
use atlas_client::AtlasClient;
use atlas_server::persistence::repos::{NewUser, UserRepo};
use support::TestDb;

// ── helpers ───────────────────────────────────────────────────────────────────

async fn create_user_with_flags(
    db: &TestDb,
    username: &str,
    is_root: bool,
    is_system_admin: bool,
) -> atlas_domain::entities::identity::User {
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    db.user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: hash,
            is_root,
            is_system_admin,
        })
        .await
        .expect("create user")
}

async fn login_as(server: &support::TestServer, username: &str) -> AtlasClient {
    let mut client = server.client();
    client
        .login(atlas_api::dtos::LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");
    client
}

async fn set_system_admin(
    client: &AtlasClient,
    user_id: uuid::Uuid,
    value: bool,
) -> Result<UserDto, atlas_client::ClientError> {
    let response = client
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

    if response.status().is_success() {
        Ok(response.json::<UserDto>().await.expect("decode dto"))
    } else {
        let problem: atlas_api::problem::ProblemDetails =
            response.json().await.unwrap_or_else(|_| {
                atlas_api::problem::ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0)
            });
        Err(atlas_client::ClientError::Api(problem))
    }
}

async fn reset_password_as(
    client: &AtlasClient,
    user_id: uuid::Uuid,
) -> Result<(), atlas_client::ClientError> {
    client.reset_user_password(user_id, "NewPassword1!").await
}

// ── boundary matrix tests ──────────────────────────────────────────────────────

/// A system-admin can list users.
#[tokio::test]
async fn system_admin_can_list_users() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-list-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-list-admin").await;

    let result = sysadmin.list_users().await;
    assert!(
        result.is_ok(),
        "system-admin should be able to list users, got {result:?}"
    );

    db.teardown().await;
}

/// A system-admin can create a user.
#[tokio::test]
async fn system_admin_can_create_user() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-create-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-create-admin").await;

    let result = sysadmin
        .create_user(CreateUserRequest {
            username: "sa-created-plain".to_string(),
            display_name: "Created by SA".to_string(),
            email: None,
            password: "Password1!".to_string(),
        })
        .await;
    assert!(
        result.is_ok(),
        "system-admin should be able to create users, got {result:?}"
    );

    db.teardown().await;
}

/// A system-admin can enable a plain user.
#[tokio::test]
async fn system_admin_can_enable_plain_user() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-enable-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-enable-admin").await;

    let plain = create_user_with_flags(&db, "sa-enable-plain", false, false).await;

    let result = sysadmin.enable_user(plain.id.0).await;
    assert!(
        result.is_ok(),
        "system-admin should be able to enable plain user, got {result:?}"
    );

    db.teardown().await;
}

/// A system-admin can disable a plain user.
#[tokio::test]
async fn system_admin_can_disable_plain_user() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-disable-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-disable-admin").await;

    let plain = create_user_with_flags(&db, "sa-disable-plain", false, false).await;

    let result = sysadmin.disable_user(plain.id.0).await;
    assert!(
        result.is_ok(),
        "system-admin should be able to disable plain user, got {result:?}"
    );

    db.teardown().await;
}

/// A system-admin CANNOT disable a root user (target-protection).
#[tokio::test]
async fn system_admin_cannot_disable_root() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-disr-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-disr-admin").await;

    let root = create_user_with_flags(&db, "sa-disr-root", true, false).await;

    let result = sysadmin.disable_user(root.id.0).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "system-admin should get 403 when disabling root, got {result:?}"
    );

    db.teardown().await;
}

/// A system-admin CANNOT disable another system-admin (target-protection).
#[tokio::test]
async fn system_admin_cannot_disable_peer_system_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-dispeer-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-dispeer-admin").await;

    let peer = create_user_with_flags(&db, "sa-dispeer-peer", false, true).await;

    let result = sysadmin.disable_user(peer.id.0).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "system-admin should get 403 when disabling peer system-admin, got {result:?}"
    );

    db.teardown().await;
}

/// A system-admin CANNOT reset password of a root user (target-protection).
#[tokio::test]
async fn system_admin_cannot_reset_password_of_root() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-rpwr-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-rpwr-admin").await;

    let root = create_user_with_flags(&db, "sa-rpwr-root", true, false).await;

    let result = reset_password_as(&sysadmin, root.id.0).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "system-admin should get 403 resetting root password, got {result:?}"
    );

    db.teardown().await;
}

/// A system-admin CANNOT reset password of another system-admin (target-protection).
#[tokio::test]
async fn system_admin_cannot_reset_password_of_peer_system_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-rppeer-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-rppeer-admin").await;

    let peer = create_user_with_flags(&db, "sa-rppeer-peer", false, true).await;

    let result = reset_password_as(&sysadmin, peer.id.0).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "system-admin should get 403 resetting peer system-admin password, got {result:?}"
    );

    db.teardown().await;
}

/// A system-admin CANNOT promote/demote system-admin (RequireRoot blocks it).
#[tokio::test]
async fn system_admin_cannot_promote_demote() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-promote-admin", false, true).await;
    let sysadmin = login_as(&server, "sa-promote-admin").await;

    let plain = create_user_with_flags(&db, "sa-promote-plain", false, false).await;

    let result = set_system_admin(&sysadmin, plain.id.0, true).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "system-admin should get 403 when promoting, got {result:?}"
    );

    db.teardown().await;
}

/// Root CAN promote a plain user to system-admin.
#[tokio::test]
async fn root_can_promote_to_system_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let plain = create_user_with_flags(&db, "sa-rootprm-plain", false, false).await;

    let result = set_system_admin(&root, plain.id.0, true).await;
    assert!(
        result.is_ok(),
        "root should be able to promote, got {result:?}"
    );
    assert!(
        result.unwrap().is_system_admin,
        "promoted user should have is_system_admin=true"
    );

    db.teardown().await;
}

/// Root CAN demote a system-admin.
#[tokio::test]
async fn root_can_demote_system_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let sysadmin = create_user_with_flags(&db, "sa-rootdem-sa", false, true).await;

    let result = set_system_admin(&root, sysadmin.id.0, false).await;
    assert!(
        result.is_ok(),
        "root should be able to demote, got {result:?}"
    );
    assert!(
        !result.unwrap().is_system_admin,
        "demoted user should have is_system_admin=false"
    );

    db.teardown().await;
}

/// Root CANNOT promote/demote itself (self-target rejected).
#[tokio::test]
async fn root_cannot_target_self_for_promote() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let root_user = create_user_with_flags(&db, "sa-selfpr-root", true, false).await;
    let root = login_as(&server, "sa-selfpr-root").await;

    let result = set_system_admin(&root, root_user.id.0, true).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 400),
        "root self-target should get 400, got {result:?}"
    );

    db.teardown().await;
}

/// Root CANNOT set system-admin on itself (self-target = root-target coincide for the single root).
/// The DB constraint allows only one root, so "another root" cannot exist; the self-target 400
/// already exercises the root-target rejection path.
#[tokio::test]
async fn root_cannot_target_self_for_system_admin_change() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let root_user = create_user_with_flags(&db, "sa-self2-root", true, false).await;
    let root = login_as(&server, "sa-self2-root").await;

    // Self-target is rejected as 400 — both the self-check and the root-target check apply.
    let result = set_system_admin(&root, root_user.id.0, true).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 400),
        "root self-target should get 400, got {result:?}"
    );

    db.teardown().await;
}

/// A plain user gets 403 on all admin routes.
#[tokio::test]
async fn plain_user_gets_403_on_admin_routes() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-plain-actor", false, false).await;
    let plain = login_as(&server, "sa-plain-actor").await;
    let target = create_user_with_flags(&db, "sa-plain-target", false, false).await;

    let list_err = plain.list_users().await;
    assert!(
        matches!(list_err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "plain user should get 403 on list_users, got {list_err:?}"
    );

    let disable_err = plain.disable_user(target.id.0).await;
    assert!(
        matches!(disable_err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "plain user should get 403 on disable, got {disable_err:?}"
    );

    let promote_err = set_system_admin(&plain, target.id.0, true).await;
    assert!(
        matches!(promote_err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "plain user should get 403 on promote, got {promote_err:?}"
    );

    db.teardown().await;
}

/// Root CAN disable a plain user.
#[tokio::test]
async fn root_can_disable_plain_user() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    let plain = create_user_with_flags(&db, "sa-rootdis-plain", false, false).await;

    let result = root.disable_user(plain.id.0).await;
    assert!(
        result.is_ok(),
        "root should be able to disable plain user, got {result:?}"
    );

    db.teardown().await;
}

/// is_system_admin is present in UserDto.
#[tokio::test]
async fn user_dto_includes_is_system_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;
    create_user_with_flags(&db, "sa-dto-check-sa", false, true).await;

    let users = root.list_users().await.expect("list users");
    let sa = users.iter().find(|u| u.username == "sa-dto-check-sa");
    assert!(sa.is_some(), "seeded system-admin not found in list");
    assert!(
        sa.unwrap().is_system_admin,
        "is_system_admin should be true in UserDto"
    );

    db.teardown().await;
}

/// is_system_admin is present in MeResponse.
#[tokio::test]
async fn me_response_includes_is_system_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    create_user_with_flags(&db, "sa-me-check", false, true).await;
    let sysadmin = login_as(&server, "sa-me-check").await;

    let me = sysadmin.me().await.expect("me");
    assert!(
        me.is_system_admin,
        "is_system_admin should be true in MeResponse"
    );

    db.teardown().await;
}
