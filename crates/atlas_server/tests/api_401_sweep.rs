#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Unauthenticated sweep over every protected route the registry declares,
//! at both `/api` and `/api/v2`: each must answer 401.
//!
//! This file proves only the authentication invariant. Mount presence at
//! both namespaces (a declared route actually resolving through the
//! assembled `app()`) is proven by
//! `every_declared_route_resolves_at_both_mounts` in
//! `api_router_mount_assertion.rs`. A "not 404" check on an unauthenticated
//! request cannot prove that: `app()` mounts a protected root fallback, so
//! an unmatched path also answers 401, and such an assertion never fires.

mod support;

use atlas_core::registry::HttpMethod;
use support::route_matrix::route_matrix;

/// Maps every `HttpMethod` variant to its `reqwest::Method`. Exhaustive by
/// construction (no `_` arm): a method added to `HttpMethod` fails to
/// compile here instead of silently falling back to GET, which is exactly
/// the bug this migration fixes (the old raw-request helpers below matched
/// on `&str` with a `_ => http.get(...)` fallback that turned every
/// unhandled method, including PUT, into a GET).
fn reqwest_method(method: HttpMethod) -> reqwest::Method {
    match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
        HttpMethod::Head => reqwest::Method::HEAD,
        HttpMethod::Options => reqwest::Method::OPTIONS,
    }
}

/// T5.9/T5.10/T7.12 (`v2-e3-s4` PR5/PR7, D1/D10): namespace-parametrized off
/// each `route_matrix()` entry's own `namespaces()` — not a flat
/// `NAMESPACES` pair — from one data source (`route_matrix()`), not two
/// hand-copied test files — asserts the same 401 behavior at both `/api`
/// and the route's own `/api/v2/<component>`.
#[tokio::test]
async fn all_non_public_routes_require_authentication() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (member, _user) = support::login_user(&server, &db, "sweep-member").await;
    let ws_slug = format!("ws-{}", "sweep-member");

    let http = reqwest::Client::new();
    let matrix = route_matrix();

    for entry in &matrix {
        if entry.is_public {
            continue;
        }

        let relative = entry.path_template.replace("{ws}", &ws_slug);

        for namespace in entry.namespaces() {
            let path = atlas_server::router_audit::mounted_path(&namespace, &relative);
            let url = format!("{}{}", server.base_url(), path);

            let status = http
                .request(reqwest_method(entry.method), &url)
                .send()
                .await
                .map(|response| response.status().as_u16())
                .unwrap_or(0);

            assert_eq!(
                status, 401,
                "namespace {namespace}: expected 401 for {} {} but got {}",
                entry.method, path, status
            );
        }
    }

    drop(member);
    db.teardown().await;
}
