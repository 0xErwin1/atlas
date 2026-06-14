#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{LoginRequest, MeResponse};
use atlas_client::AtlasClient;
use atlas_server::persistence::repos::UserRepo;

#[tokio::test]
async fn login_returns_body_token_and_set_cookie() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "auth-login-user").await;

    assert!(
        client.token().is_some(),
        "client must store the session token after login"
    );

    db.teardown().await;
}

#[tokio::test]
async fn login_invalid_credentials_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let result = AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: "nobody".into(),
            password: "wrong".into(),
        })
        .await;

    assert!(result.is_err(), "wrong credentials must fail");

    db.teardown().await;
}

#[tokio::test]
async fn bearer_token_authenticates_me_endpoint() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "auth-me-user").await;

    let me: MeResponse = client.me().await.expect("GET /v1/auth/me must succeed");

    assert_eq!(me.username, user.username);

    db.teardown().await;
}

#[tokio::test]
async fn unauthenticated_me_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let result = AtlasClient::new(server.base_url()).me().await;

    assert!(result.is_err(), "unauthenticated /me must fail with 401");

    db.teardown().await;
}

#[tokio::test]
async fn logout_revokes_session() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "auth-logout-user").await;

    client.logout().await.expect("logout must succeed");

    let result = client.me().await;
    assert!(result.is_err(), "after logout the token must be invalid");

    db.teardown().await;
}

#[tokio::test]
async fn expired_session_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "auth-expiry-user").await;

    support::expire_all_sessions(&db).await;

    let result = client.me().await;
    assert!(result.is_err(), "expired session must be rejected with 401");

    db.teardown().await;
}

#[tokio::test]
async fn nonexistent_user_login_returns_401() {
    // Behavioral test for timing-oracle fix: both "user not found" and "wrong password"
    // paths must return 401 with the same shape. Timing itself is not unit-asserted.
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let result = atlas_client::AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: "does-not-exist-at-all".into(),
            password: "anypassword".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "nonexistent user login must return 401, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn wrong_password_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, user) = support::login_user(&server, &db, "auth-wrongpw-user").await;

    let result = atlas_client::AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: user.username.clone(),
            password: "definitelywrong".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "wrong password must return 401, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn disabled_user_with_correct_password_returns_401() {
    // Behavioral test for the disabled-account timing fix: a disabled user must
    // still receive 401 even when the correct password is supplied.
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, user) = support::login_user(&server, &db, "auth-disabled-user").await;

    db.user_repo().disable(user.id).await.expect("disable user");

    let result = atlas_client::AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: user.username.clone(),
            password: "TestPassword1!".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "disabled user with correct password must return 401, got: {result:?}"
    );

    db.teardown().await;
}
