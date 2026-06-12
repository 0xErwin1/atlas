#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{CreateUserRequest, LoginRequest, UserDto};
use atlas_client::AtlasClient;
use support::{TestDb, TestServer, login_user_with_workspace};

#[tokio::test]
async fn create_user_requires_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (non_admin, _, _) = login_user_with_workspace(&server, &db, "non-admin").await;

    let status = non_admin
        .create_user(CreateUserRequest {
            username: "newuser".to_string(),
            display_name: "New User".to_string(),
            password: "Password1!".to_string(),
        })
        .await;

    assert!(
        matches!(status, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 but got {status:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn create_user_succeeds_for_root() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;

    let user: UserDto = root
        .create_user(CreateUserRequest {
            username: "brandnew".to_string(),
            display_name: "Brand New".to_string(),
            password: "Password1!".to_string(),
        })
        .await
        .expect("create_user");

    assert_eq!(user.username, "brandnew");
    assert_eq!(user.display_name, "Brand New");
    assert!(!user.is_root);
    assert!(user.disabled_at.is_none());

    db.teardown().await;
}

#[tokio::test]
async fn disable_user_revokes_sessions() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;

    let (victim_client, _, victim) = login_user_with_workspace(&server, &db, "victim").await;

    // Victim can call me before disable.
    victim_client.me().await.expect("me before disable");

    // Admin disables the victim.
    root.disable_user(victim.id.0).await.expect("disable_user");

    // Victim's session is now revoked — 401.
    let err = victim_client.me().await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 after disable, got {err:?}"
    );

    // Fresh login is also blocked.
    let mut fresh_client = AtlasClient::new(server.base_url().to_string());
    let login_err = fresh_client
        .login(LoginRequest {
            username: "victim".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await;
    assert!(
        matches!(login_err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 on login after disable, got {login_err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn enable_user_restores_access() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (_, _, victim) = login_user_with_workspace(&server, &db, "victim2").await;

    root.disable_user(victim.id.0).await.expect("disable");
    root.enable_user(victim.id.0).await.expect("enable");

    // Fresh login now succeeds.
    let mut restored = AtlasClient::new(server.base_url().to_string());
    restored
        .login(LoginRequest {
            username: "victim2".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login after re-enable");

    db.teardown().await;
}

#[tokio::test]
async fn disable_requires_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (actor, _, _) = login_user_with_workspace(&server, &db, "actor").await;
    let (_, _, target) = login_user_with_workspace(&server, &db, "target").await;

    let err = actor.disable_user(target.id.0).await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 but got {err:?}"
    );

    db.teardown().await;
}
