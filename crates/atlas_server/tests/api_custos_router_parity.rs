#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_server::router_audit::{mounted_path, v2_namespace};

/// `v2-e3-s2-router-audit` PR3: proves the 35 routes moved out of
/// `lib.rs::app()` into `routes::custos::router()` keep their pre-refactor
/// mount and authentication posture against the ASSEMBLED router
/// (`atlas_server::app`), not just the declarative `declared_routes()` vs.
/// registry comparison in `custos.rs`'s own module tests (mirrors PR2's
/// `api_platform_router_parity.rs`).
///
/// `login` and `activate` (both methods) must answer without authentication
/// (never 401) and must be mounted in the assembled router. Every sampled
/// protected route must reject an unauthenticated request with exactly 401,
/// proving `require_authn` still sits in front of every one of them
/// post-move.
///
/// The mount check cannot use "status != 404" on the route's own method:
/// `activate`'s real handlers legitimately answer 404 for an unknown token
/// (see `routes/activate.rs`'s `ApiError::NotFound` path), so that status is
/// indistinguishable from "path not mounted" for that route. Instead each
/// route is probed with PATCH, a method none of `login`/`activate` register.
/// Axum answers a foreign method on a matched path with 405 Method Not
/// Allowed, and answers an unmatched path with 404 regardless of method — so
/// a 405 on the PATCH probe proves the path is mounted, independent of
/// whatever the real method's handler legitimately returns.
///
/// `v2-e3-s4` PR7 (D10), collapsed to one mount by `v2-e3-s7` (D1/U2): the
/// whole proof runs once, at custos's own `/api/v2/custos` mount, offenders
/// naming the namespace.
#[tokio::test]
async fn custos_routes_keep_pre_refactor_mount_and_auth_posture() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    const PUBLIC_ROUTES: [(reqwest::Method, &str); 3] = [
        (reqwest::Method::POST, "/auth/login"),
        (reqwest::Method::GET, "/activate/some-token"),
        (reqwest::Method::POST, "/activate/some-token"),
    ];

    // One representative route per sub-family behind `require_authn` (D6):
    // auth self-service, users admin, api-keys, grants (the two `Some(_)`
    // capability routes), groups, and the security audit log.
    const PROTECTED_ROUTES: [(reqwest::Method, &str); 8] = [
        (reqwest::Method::GET, "/auth/me"),
        (reqwest::Method::GET, "/users"),
        (reqwest::Method::GET, "/api-keys"),
        (reqwest::Method::GET, "/workspaces/some-ws/grants"),
        (
            reqwest::Method::GET,
            "/workspaces/some-ws/projects/some-project/grants",
        ),
        (reqwest::Method::GET, "/workspaces/some-ws/groups"),
        (reqwest::Method::GET, "/admin/audit"),
        (reqwest::Method::GET, "/workspaces/some-ws/audit"),
    ];

    let namespace = v2_namespace("custos");
    let namespace = namespace.as_str();
    let mut examined_public_routes = 0usize;

    for (method, relative) in PUBLIC_ROUTES {
        let path = mounted_path(namespace, relative);
        let status = send(&http, method.clone(), server.base_url(), &path).await;

        assert_ne!(
            status, 401,
            "namespace {namespace}: public route {method} {path} must not require \
             authentication, got 401"
        );
        examined_public_routes += 1;
    }

    assert!(
        examined_public_routes > 0,
        "custos_routes_keep_pre_refactor_mount_and_auth_posture must examine at least one \
         public route at {namespace}, or its assertions pass vacuously"
    );

    // The mount probe is per path, not per (method, path): activate registers
    // two methods on one path, and probing it once keeps the request count
    // per governed route well under the limiter's burst size.
    let mut probed_paths: Vec<&str> = PUBLIC_ROUTES.iter().map(|(_, path)| *path).collect();
    probed_paths.dedup();

    assert!(
        !probed_paths.is_empty(),
        "custos_routes_keep_pre_refactor_mount_and_auth_posture must probe at least one \
         public path for its mount at {namespace}, or its assertions pass vacuously"
    );

    for relative in probed_paths {
        let path = mounted_path(namespace, relative);
        let mount_status = send(&http, reqwest::Method::PATCH, server.base_url(), &path).await;
        assert_eq!(
            mount_status, 405,
            "namespace {namespace}: public route {path} must be mounted in the assembled \
             router (PATCH probe), got {mount_status}"
        );
    }

    let mut examined_protected_routes = 0usize;

    for (method, relative) in PROTECTED_ROUTES {
        let path = mounted_path(namespace, relative);
        let status = send(&http, method.clone(), server.base_url(), &path).await;

        assert_eq!(
            status, 401,
            "namespace {namespace}: protected route {method} {path} must reject an \
             unauthenticated request, got {status}"
        );
        examined_protected_routes += 1;
    }

    assert!(
        examined_protected_routes > 0,
        "custos_routes_keep_pre_refactor_mount_and_auth_posture must examine at least one \
         protected route at {namespace}, or its assertions pass vacuously"
    );

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
