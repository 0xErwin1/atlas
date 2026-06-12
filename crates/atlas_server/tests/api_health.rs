#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

#[tokio::test]
async fn health_endpoint_returns_200_via_atlas_client() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let response = server
        .client()
        .health()
        .await
        .expect("health request must succeed");

    assert_eq!(response.status, "ok");

    db.teardown().await;
}
