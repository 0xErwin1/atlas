#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::CreateApiKeyRequest;
use atlas_client::ClientError;
use serde_json::json;
use support::{TestDb, TestServer, login_user_with_workspace};

fn key_req(name: &str) -> CreateApiKeyRequest {
    CreateApiKeyRequest {
        name: name.to_string(),
        expires_at: None,
    }
}

#[tokio::test]
async fn get_ui_state_returns_empty_object_when_no_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _ws, _user) = login_user_with_workspace(&server, &db, "ui-empty").await;

    let state = client.get_ui_state().await.expect("get_ui_state");

    assert_eq!(state, json!({}), "a user with no row must get an empty object");
}

#[tokio::test]
async fn put_then_get_returns_state_verbatim() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _ws, _user) = login_user_with_workspace(&server, &db, "ui-roundtrip").await;

    let payload = json!({
        "collapsedFolders": ["a", "b"],
        "sidebarWidth": 280,
        "nested": { "flag": true, "n": null }
    });

    let echoed = client.set_ui_state(&payload).await.expect("set_ui_state");
    assert_eq!(echoed, payload, "PUT must echo the stored state back");

    let fetched = client.get_ui_state().await.expect("get_ui_state");
    assert_eq!(fetched, payload, "GET must return the stored state verbatim");
}

#[tokio::test]
async fn put_overwrites_previous_state() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _ws, _user) = login_user_with_workspace(&server, &db, "ui-upsert").await;

    client
        .set_ui_state(&json!({ "first": 1 }))
        .await
        .expect("first set_ui_state");

    let second = json!({ "second": 2, "extra": "x" });
    client.set_ui_state(&second).await.expect("second set_ui_state");

    let fetched = client.get_ui_state().await.expect("get_ui_state");

    assert_eq!(
        fetched, second,
        "a second PUT must replace the previous state (upsert)"
    );
}

#[tokio::test]
async fn api_key_principal_is_forbidden_on_both_endpoints() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _user) = login_user_with_workspace(&server, &db, "ui-agent-owner").await;

    let created_key = owner
        .create_api_key(&ws.slug, key_req("agent-key"))
        .await
        .expect("create_api_key");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(created_key.secret);

    let get_err = agent
        .get_ui_state()
        .await
        .expect_err("api key must not read ui-state");
    match get_err {
        ClientError::Api(p) => assert_eq!(p.status, 403, "expected 403 on GET, got {}", p.status),
        other => panic!("unexpected error on GET: {other:?}"),
    }

    let put_err = agent
        .set_ui_state(&json!({ "x": 1 }))
        .await
        .expect_err("api key must not write ui-state");
    match put_err {
        ClientError::Api(p) => assert_eq!(p.status, 403, "expected 403 on PUT, got {}", p.status),
        other => panic!("unexpected error on PUT: {other:?}"),
    }
}

#[tokio::test]
async fn unauthenticated_is_rejected_on_both_endpoints() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let get_err = server
        .client()
        .get_ui_state()
        .await
        .expect_err("unauthenticated GET must fail");
    match get_err {
        ClientError::Api(p) => assert_eq!(p.status, 401, "expected 401 on GET, got {}", p.status),
        other => panic!("unexpected error on GET: {other:?}"),
    }

    let put_err = server
        .client()
        .set_ui_state(&json!({ "x": 1 }))
        .await
        .expect_err("unauthenticated PUT must fail");
    match put_err {
        ClientError::Api(p) => assert_eq!(p.status, 401, "expected 401 on PUT, got {}", p.status),
        other => panic!("unexpected error on PUT: {other:?}"),
    }
}
