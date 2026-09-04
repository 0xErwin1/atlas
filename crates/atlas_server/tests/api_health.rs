#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use sea_orm::ConnectionTrait;

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

/// SHELL-CFG-2, design D4.2/R11: `/api/meta`'s response body must never
/// carry a secret field, regardless of how the endpoint's shape evolves.
/// This is a permanent negative assertion (no secret field name appears),
/// not a shape assertion — it survives E11-S3a's `/api/meta` rewrite.
///
/// The harness cannot inject real `ATLAS_ROOT_PASSWORD`/
/// `ATLAS_WEBHOOK_ENC_KEY`/S3-credential values into a running test server
/// (edition 2024 forbids `std::env::set_var`, and `TestServer` spawns
/// `AppState::for_test`, not `AtlasConfig::from_registry`), so this asserts
/// structurally: the serialized response never carries any of the five
/// secret field names `AtlasConfig`'s component structs define.
#[tokio::test]
async fn meta_response_carries_no_secret_field_names() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "meta-secret-fields").await;

    let meta = client
        .server_meta()
        .await
        .expect("server_meta request must succeed");
    let body = serde_json::to_string(&meta).expect("meta serializes to JSON");

    for forbidden in [
        "root_password",
        "webhook_enc_key",
        "access_key_id",
        "secret_access_key",
        "api_key",
    ] {
        assert!(
            !body.contains(forbidden),
            "/api/v2/platform/meta response must never carry a `{forbidden}` field: {body}"
        );
    }

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
    assert_eq!(meta.max_attachment_bytes, Some(20 * 1024 * 1024));
    assert_eq!(meta.semantic_search_enabled, Some(true));

    db.teardown().await;
}

#[tokio::test]
async fn meta_reports_semantic_search_disabled_when_no_embedding_provider_exists() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let mut state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("test state");
    state.embedding_provider = None;
    let server = support::TestServer::spawn_with_state(state).await;
    let (client, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "meta-semantic-disabled").await;

    let meta = client.server_meta().await.expect("server_meta request");
    assert_eq!(meta.semantic_search_enabled, Some(false));

    db.teardown().await;
}

#[tokio::test]
async fn meta_reports_semantic_search_disabled_when_schema_is_absent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    db.conn()
        .execute_unprepared("DROP TABLE acta.search_embeddings")
        .await
        .expect("drop semantic search table");
    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("test state");
    let server = support::TestServer::spawn_with_state(state).await;
    let (client, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "meta-semantic-schema-absent").await;

    let meta = client.server_meta().await.expect("server_meta request");
    assert_eq!(meta.semantic_search_enabled, Some(false));

    db.teardown().await;
}

#[tokio::test]
async fn meta_reports_semantic_search_disabled_when_schema_disappears_after_startup() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("test state");
    db.conn()
        .execute_unprepared("DROP TABLE acta.search_embeddings")
        .await
        .expect("drop semantic search table after startup readiness");
    let server = support::TestServer::spawn_with_state(state).await;
    let (client, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "meta-semantic-schema-drift").await;

    let meta = client.server_meta().await.expect("server_meta request");
    assert_eq!(meta.semantic_search_enabled, Some(false));

    db.teardown().await;
}

#[tokio::test]
async fn meta_exposes_the_configured_attachment_limit() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("test state")
        .with_max_attachment_bytes(123_456);
    let server = support::TestServer::spawn_with_state(state).await;
    let (client, _ws, _user) = support::login_user_with_workspace(&server, &db, "meta-limit").await;

    let meta = client.server_meta().await.expect("server_meta request");
    assert_eq!(meta.max_attachment_bytes, Some(123_456));

    db.teardown().await;
}
