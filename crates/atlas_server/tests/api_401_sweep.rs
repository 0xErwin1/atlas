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
