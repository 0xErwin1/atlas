#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

/// R3 (`v2-e3-s2-pr2-platform-router`): proves the five routes moved out of
/// `lib.rs::app()` into `routes::platform::router()` keep their pre-refactor
/// mount and authentication posture against the ASSEMBLED router
/// (`atlas_server::app`), not just the declarative `declared_routes()` vs.
/// registry comparison in `platform.rs`'s own module tests.
///
/// `/health`, `/ready`, `/version` must answer without authentication
/// (mounted, never 401). `/api/me/ui-state` (GET and PUT) and `/api/meta`
/// must reject an unauthenticated request with exactly 401, proving
/// `require_authn` still sits in front of them. Both 404 and 405 count as
/// "not served": a 405 means the path is mounted but not for this method,
/// which is exactly the kind of drift a hand-reconstructed layer stack could
/// introduce silently.
#[tokio::test]
async fn platform_routes_keep_pre_refactor_mount_and_auth_posture() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    const PUBLIC_ROUTES: [(reqwest::Method, &str); 3] = [
        (reqwest::Method::GET, "/health"),
        (reqwest::Method::GET, "/ready"),
        (reqwest::Method::GET, "/version"),
    ];

    for (method, path) in PUBLIC_ROUTES {
        let status = send(&http, method.clone(), server.base_url(), path).await;

        assert!(
            !matches!(status, 404 | 405),
            "public route {method} {path} must be mounted in the assembled router, got {status}"
        );
        assert_ne!(
            status, 401,
            "public route {method} {path} must not require authentication, got 401"
        );
    }

    const PROTECTED_ROUTES: [(reqwest::Method, &str); 3] = [
        (reqwest::Method::GET, "/api/me/ui-state"),
        (reqwest::Method::PUT, "/api/me/ui-state"),
        (reqwest::Method::GET, "/api/meta"),
    ];

    for (method, path) in PROTECTED_ROUTES {
        let status = send(&http, method.clone(), server.base_url(), path).await;

        assert_eq!(
            status, 401,
            "protected route {method} {path} must reject an unauthenticated request, got {status}"
        );
    }

    db.teardown().await;
}

async fn send(http: &reqwest::Client, method: reqwest::Method, base_url: &str, path: &str) -> u16 {
    http.request(method, format!("{base_url}{path}"))
        .send()
        .await
        .expect("request must not error")
        .status()
        .as_u16()
}
