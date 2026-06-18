#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{ChangePasswordRequest, LoginRequest, UserDto};
use atlas_client::AtlasClient;
use atlas_domain::{Actor, WorkspaceCtx};
use atlas_server::persistence::repos::{ApiKeyRepo, NewApiKey};
use support::{TestDb, TestServer, login_root_user, login_user_with_workspace};

#[tokio::test]
async fn list_users_returns_all_users_for_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = login_root_user(&server, &db).await;
    let (_member, _, member_user) =
        login_user_with_workspace(&server, &db, "settings-member").await;

    let users: Vec<UserDto> = root.list_users().await.expect("list_users");

    assert!(
        users.iter().any(|u| u.id == member_user.id.0),
        "the created member should appear in the user list"
    );
    assert!(
        users.iter().any(|u| u.is_root),
        "the calling root user should appear in the user list"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_users_includes_disabled_users() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = login_root_user(&server, &db).await;
    let (_victim, _, victim) = login_user_with_workspace(&server, &db, "settings-disabled").await;

    root.disable_user(victim.id.0).await.expect("disable_user");

    let users: Vec<UserDto> = root.list_users().await.expect("list_users");

    let listed = users
        .iter()
        .find(|u| u.id == victim.id.0)
        .expect("disabled user must still be listed");
    assert!(
        listed.disabled_at.is_some(),
        "disabled user should carry a disabled_at timestamp"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_users_rejects_non_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (member, _, _) = login_user_with_workspace(&server, &db, "settings-nonadmin").await;

    let err = member.list_users().await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 for non-admin, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_users_rejects_unauthenticated() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let anon = AtlasClient::new(server.base_url().to_string());

    let err = anon.list_users().await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 for unauthenticated, got {err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn change_password_succeeds_and_rotates_credentials() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "settings-rotate").await;

    client
        .change_password(ChangePasswordRequest {
            current_password: "TestPassword1!".to_string(),
            new_password: "BrandNewPass2@".to_string(),
        })
        .await
        .expect("change_password");

    // Old password no longer works.
    let mut old_login = AtlasClient::new(server.base_url().to_string());
    let old_err = old_login
        .login(LoginRequest {
            username: "settings-rotate".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await;
    assert!(
        matches!(old_err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "old password should be rejected, got {old_err:?}"
    );

    // New password works.
    let mut new_login = AtlasClient::new(server.base_url().to_string());
    new_login
        .login(LoginRequest {
            username: "settings-rotate".to_string(),
            password: "BrandNewPass2@".to_string(),
        })
        .await
        .expect("login with new password");

    db.teardown().await;
}

#[tokio::test]
async fn change_password_rejects_wrong_current_password() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "settings-wrongpw").await;

    let err = client
        .change_password(ChangePasswordRequest {
            current_password: "NotMyPassword9!".to_string(),
            new_password: "BrandNewPass2@".to_string(),
        })
        .await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 for wrong current password, got {err:?}"
    );

    // Password unchanged: original still works.
    let mut relogin = AtlasClient::new(server.base_url().to_string());
    relogin
        .login(LoginRequest {
            username: "settings-wrongpw".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("original password must still work");

    db.teardown().await;
}

#[tokio::test]
async fn change_password_rejects_api_key_principal() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) = login_user_with_workspace(&server, &db, "settings-agent").await;

    let raw_secret = "atlas_settings_agent_secret_token";
    let token_hash = atlas_server::auth::tokens::hash_token(raw_secret);
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    db.api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "settings-bot".to_string(),
                token_hash,
                expires_at: None,
            },
        )
        .await
        .expect("create api key");

    let agent = AtlasClient::new(server.base_url().to_string()).with_token(raw_secret);

    let err = agent
        .change_password(ChangePasswordRequest {
            current_password: "irrelevant".to_string(),
            new_password: "irrelevant2".to_string(),
        })
        .await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 for api-key principal, got {err:?}"
    );

    db.teardown().await;
}
