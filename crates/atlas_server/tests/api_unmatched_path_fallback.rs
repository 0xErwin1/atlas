#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use reqwest::StatusCode;

fn unmatched_paths() -> [String; 3] {
    let ws = uuid::Uuid::new_v4();
    let slug = uuid::Uuid::new_v4();
    let comment_id = uuid::Uuid::new_v4();
    let attachment_id = uuid::Uuid::new_v4();

    [
        "/api/does-not-exist".to_string(),
        "/does-not-exist".to_string(),
        format!(
            "/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}/content"
        ),
    ]
}

#[tokio::test]
async fn unmatched_paths_require_authentication_before_answering_not_found() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    for path in unmatched_paths() {
        let response = http
            .get(format!("{}{path}", server.base_url()))
            .send()
            .await
            .expect("send unauthenticated request");

        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "unauthenticated GET {path} must be rejected by require_authn before the 404 fallback"
        );
    }
}

#[tokio::test]
async fn unmatched_path_answers_not_found_once_authenticated() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _, _) =
        support::login_user_with_workspace(&server, &db, "unmatched-path-fallback").await;

    let path = "/api/does-not-exist";
    let response = client
        .http_client()
        .get(format!("{}{path}", server.base_url()))
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("send authenticated request");

    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "authenticated GET {path} must reach the 404 fallback"
    );
}

/// G1b (E11-S7 design D6): `/metrics` is not a registry entry and `app()`
/// never serves it (INV-METRICS-SECOND-LISTENER-ONLY), so a live request
/// against the public listener reaches the same protected fallback as any
/// other unmatched path — **401 unauthenticated, 404 authenticated**, never
/// a bare "404", which would fail the anonymous case. This is gate G1's
/// only container-backed half; the container-free half (G1a) lives in
/// `tests/metrics_exposition.rs`.
#[tokio::test]
async fn metrics_is_unreachable_on_the_public_listener() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    let unauthenticated = http
        .get(format!("{}/metrics", server.base_url()))
        .send()
        .await
        .expect("send unauthenticated request");

    assert_eq!(
        unauthenticated.status(),
        StatusCode::UNAUTHORIZED,
        "unauthenticated GET /metrics must be rejected by require_authn before the 404 fallback"
    );

    let (client, _, _) =
        support::login_user_with_workspace(&server, &db, "metrics-public-listener").await;

    let authenticated = client
        .http_client()
        .get(format!("{}/metrics", server.base_url()))
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("send authenticated request");

    assert_eq!(
        authenticated.status(),
        StatusCode::NOT_FOUND,
        "authenticated GET /metrics must reach the 404 fallback, never the exposition router"
    );
}
