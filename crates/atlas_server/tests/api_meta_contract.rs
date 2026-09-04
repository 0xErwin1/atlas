#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! E11-S3a PR2, design D4: `/api/v2/platform/meta` and `/version` expose
//! registry identity only, asserted by name — extending the S1 redaction
//! test to the reshaped payload — and the two payloads' shared fields never
//! drift apart on the same process.

mod support;

/// A closed list of `AtlasConfig` field/env names that must never appear in
/// `/api/v2/platform/meta`'s body, by name (spec Scenario "`/meta` never
/// carries a config value, asserted by name").
const FORBIDDEN_NAMES: &[&str] = &[
    "max_attachment_bytes",
    "semantic_search_enabled",
    "ATLAS_ROOT_PASSWORD",
    "ATLAS_WEBHOOK_ENC_KEY",
    "ATLAS_S3_ACCESS_KEY_ID",
    "ATLAS_S3_SECRET_ACCESS_KEY",
    "postgres://",
    "://",
];

#[tokio::test]
async fn meta_body_never_carries_a_config_field_by_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "meta-contract-names").await;

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

    for forbidden in FORBIDDEN_NAMES {
        assert!(
            !body.contains(forbidden),
            "/api/v2/platform/meta must never carry `{forbidden}`: {body}"
        );
    }

    db.teardown().await;
}

/// Design D4's drift guard: `/version` and `/api/v2/platform/meta` are built
/// through one shared constructor (`ops::meta::shared_identity`), so their
/// common fields (`version`, `build`, `components`) must be byte-identical
/// on the same process.
#[tokio::test]
async fn version_and_meta_share_identical_version_build_and_components() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "meta-contract-drift").await;

    let version_body: serde_json::Value = reqwest::Client::new()
        .get(format!("{}/version", server.base_url()))
        .send()
        .await
        .expect("version request must succeed")
        .json()
        .await
        .expect("version body decodes");

    let meta = client
        .server_meta()
        .await
        .expect("server_meta request must succeed");

    assert_eq!(version_body["version"], serde_json::json!(meta.version));
    assert_eq!(version_body["build"], serde_json::json!(meta.build));
    assert_eq!(
        version_body["components"],
        serde_json::to_value(&meta.components).expect("components serialize"),
    );
    assert!(
        !meta.components.is_empty(),
        "the walked component count must be non-zero"
    );

    db.teardown().await;
}
