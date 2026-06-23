#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_client::AtlasClient;
use support::route_matrix::{ROUTE_MATRIX, RouteKind};

#[tokio::test]
async fn all_non_public_routes_require_authentication() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (member, _user) = support::login_user(&server, &db, "sweep-member").await;
    let ws_slug = format!("ws-{}", "sweep-member");

    let anon = AtlasClient::new(server.base_url());

    for entry in ROUTE_MATRIX {
        if matches!(entry.kind, RouteKind::Public) {
            continue;
        }

        let path = entry.path_template.replace("{ws}", &ws_slug);

        let response = reqwest_call_raw(
            &anon,
            entry.method,
            &format!("{}{}", server.base_url(), path),
        )
        .await;

        assert_eq!(
            response, 401,
            "expected 401 for {} {} but got {}",
            entry.method, path, response
        );
    }

    drop(member);
    db.teardown().await;
}

/// Every non-public entry in ROUTE_REGISTRY must be wired in the router.
///
/// For protected routes, an unauthenticated request returns 401, which proves
/// the router matched the path (the authn middleware fired). A 404 means the
/// route is in the registry but missing from the router — the test turns RED
/// and forces the developer to wire the route.
///
/// Public routes are excluded from this check: they can legitimately return 404
/// for sentinel inputs when the handler looks up a resource by path parameter
/// (e.g. activation token routes). Those routes are exercised by their own
/// integration tests.
#[tokio::test]
async fn all_registry_entries_are_wired_in_router() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let ws_slug = "no-such-workspace-for-audit";
    let http = reqwest::Client::new();

    for entry in ROUTE_MATRIX {
        if matches!(entry.kind, RouteKind::Public) {
            continue;
        }

        let path = entry.path_template.replace("{ws}", ws_slug);
        let url = format!("{}{}", server.base_url(), path);

        let req = match entry.method {
            "GET" => http.get(&url),
            "POST" => http.post(&url),
            "PATCH" => http.patch(&url),
            "DELETE" => http.delete(&url),
            _ => http.get(&url),
        };
        let status = req
            .send()
            .await
            .expect("request must not error")
            .status()
            .as_u16();

        assert_ne!(
            status, 404,
            "route {} {} is in ROUTE_REGISTRY but returned 404 — it is NOT wired in the router. \
             Add it to lib.rs.",
            entry.method, path
        );
    }

    db.teardown().await;
}

async fn reqwest_call_raw(_client: &AtlasClient, method: &str, url: &str) -> u16 {
    let http = reqwest::Client::new();
    let req = match method {
        "GET" => http.get(url),
        "POST" => http.post(url),
        "PATCH" => http.patch(url),
        "DELETE" => http.delete(url),
        _ => http.get(url),
    };
    req.send().await.map(|r| r.status().as_u16()).unwrap_or(0)
}
