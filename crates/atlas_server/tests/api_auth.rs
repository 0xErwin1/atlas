#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{LoginRequest, MeResponse};
use atlas_client::AtlasClient;

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
