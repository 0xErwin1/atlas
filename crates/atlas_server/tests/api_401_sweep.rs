#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_core::registry::HttpMethod;
use atlas_server::router_audit::mounted_path;
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

#[tokio::test]
async fn all_non_public_routes_require_authentication() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (member, _user) = support::login_user(&server, &db, "sweep-member").await;
    let ws_slug = format!("ws-{}", "sweep-member");

    let http = reqwest::Client::new();

    for entry in route_matrix() {
        if entry.is_public {
            continue;
        }

        let path = mounted_path("/api", &entry.path_template.replace("{ws}", &ws_slug));
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
    }

    drop(member);
    db.teardown().await;
}

/// Every non-public route declared by the live registry must be wired in the
/// router.
///
/// For protected routes, an unauthenticated request returns 401, which
/// proves the router matched the path (the authn middleware fired). A 404
/// means the route is declared but missing from the router — the test turns
/// RED and forces the developer to wire the route.
///
/// Public routes are excluded from this check: they can legitimately return
/// 404 for sentinel inputs when the handler looks up a resource by path
/// parameter. Those routes are exercised by their own integration tests.
#[tokio::test]
async fn all_registry_entries_are_wired_in_router() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let ws_slug = "no-such-workspace-for-audit";
    let http = reqwest::Client::new();

    for entry in route_matrix() {
        if entry.is_public {
            continue;
        }

        let path = mounted_path("/api", &entry.path_template.replace("{ws}", ws_slug));
        let url = format!("{}{}", server.base_url(), path);

        let status = http
            .request(reqwest_method(entry.method), &url)
            .send()
            .await
            .expect("request must not error")
            .status()
            .as_u16();

        assert_ne!(
            status, 404,
            "route {} {} is declared but returned 404 — it is NOT wired in the router. \
             Add it to its owning component's router(state).",
            entry.method, path
        );
    }

    db.teardown().await;
}
