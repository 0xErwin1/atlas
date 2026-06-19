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

#[tokio::test]
async fn meta_exposes_version_and_optional_url() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _ws, _user) = support::login_user_with_workspace(&server, &db, "meta-url-1").await;

    let meta = client
        .server_meta()
        .await
        .expect("server_meta request must succeed");

    assert!(!meta.version.is_empty(), "version must be present");
    // ATLAS_SERVER_URL is unset under test, so the optional url resolves to None.
    // This proves the field is wired through the response without mutating env
    // (which is forbidden under edition 2024 + unsafe_code = forbid).
    assert!(
        meta.url.is_none(),
        "url must be absent when ATLAS_SERVER_URL is unset"
    );

    db.teardown().await;
}
