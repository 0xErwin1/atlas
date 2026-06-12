#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_client::AtlasClient;

#[tokio::test]
async fn no_credentials_on_probe_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let result = AtlasClient::new(server.base_url())
        .get_probe("ws-probe-no-creds")
        .await;

    assert!(result.is_err(), "unauthenticated probe must return 401");
    db.teardown().await;
}

#[tokio::test]
async fn wrong_workspace_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "probe-ws-404").await;

    let result = client.get_probe("nonexistent-workspace").await;

    assert!(result.is_err(), "wrong workspace must return 404");
    db.teardown().await;
}

#[tokio::test]
async fn valid_member_on_probe_returns_200() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "probe-valid-member").await;

    let result = client.get_probe(&ws.slug).await;

    assert!(
        result.is_ok(),
        "valid workspace member must get 200 on probe"
    );
    db.teardown().await;
}
