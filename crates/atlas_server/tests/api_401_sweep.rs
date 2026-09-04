#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Unauthenticated sweep over every protected route the registry declares,
//! at its own `/api/v2/<component>` mount: each must answer 401.
//!
//! This file proves only the authentication invariant. Mount presence (a
//! declared route actually resolving through the assembled `app()`) is
//! proven by `every_declared_route_resolves_at_its_component_mount` in
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

/// T5.9/T5.10/T7.12 (`v2-e3-s4` PR5/PR7, D1/D10), collapsed to one mount by
/// `v2-e3-s7` (D1/U2): each `route_matrix()` entry's own `mounted()` — from
/// one data source (`route_matrix()`), not a hand-copied test file — asserts
/// 401 at its own `/api/v2/<component>` mount. `INV-NONVACUOUS` (gate 7):
/// asserts a non-zero number of protected routes were actually probed, so a
/// degenerate collapse that examined zero routes would fail loudly instead
/// of passing by vacuity.
#[tokio::test]
async fn all_non_public_routes_require_authentication() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (member, _user) = support::login_user(&server, &db, "sweep-member").await;
    let ws_slug = format!("ws-{}", "sweep-member");

    let http = reqwest::Client::new();
    let matrix = route_matrix();
    let mut probed_routes = 0usize;

    for entry in &matrix {
        if entry.is_public {
            continue;
        }

        let relative = entry.path_template.replace("{ws}", &ws_slug);
        let path = atlas_server::router_audit::mounted_path(
            &atlas_server::router_audit::v2_namespace(&entry.component),
            &relative,
        );
        let url = format!("{}{}", server.base_url(), path);

        let status = http
            .request(reqwest_method(entry.method), &url)
            .send()
            .await
            .map(|response| response.status().as_u16())
            .unwrap_or(0);

        assert_eq!(
            status, 401,
            "expected 401 for {} {} but got {}",
            entry.method, path, status
        );
        probed_routes += 1;
    }

    assert!(
        probed_routes > 0,
        "the sweep must probe at least one protected route, or its assertions pass vacuously"
    );

    drop(member);
    db.teardown().await;
}
