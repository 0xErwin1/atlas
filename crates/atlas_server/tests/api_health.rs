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

/// SHELL-CFG-2, design D4.2/R11: `/api/v2/platform/meta`'s response body
/// must never carry a secret field, regardless of how the endpoint's shape
/// evolves. This is a permanent negative assertion (no secret field name
/// appears), not a shape assertion — it survives E11-S3a's `/api/v2/platform/meta`
/// rewrite.
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

    let token = client.token().expect("authenticated token");
    let body = reqwest::Client::new()
        .get(support::path::api_url(
            server.base_url(),
            "platform",
            "/meta",
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("meta request must succeed")
        .text()
        .await
        .expect("meta body is text");
    assert!(
        body.contains("\"version\""),
        "the raw meta body must be the JSON payload, got: {body}"
    );

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

/// E11-S3a design D4: `/api/v2/platform/meta` is identity-only, no config
/// value, and lists the registry's present components (non-vacuously).
#[tokio::test]
async fn meta_exposes_version_url_and_components() {
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
    assert!(
        !meta.components.is_empty(),
        "components must walk a non-zero component count"
    );
    assert!(
        meta.components.iter().any(|c| c.stable_id == "platform"),
        "platform must be among the listed components"
    );

    db.teardown().await;
}
