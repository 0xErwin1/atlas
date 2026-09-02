#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::{HashMap, HashSet};

use atlas_core::registry::HttpMethod;
use atlas_server::router_audit::{
    acta_route_paths, custos_route_paths, mounted_path, platform_route_paths,
};

/// D9 (`v2-e3-s4` PR1): a frozen, pre-rewrite snapshot of every `(method,
/// path)` pair `platform`/`custos`/`acta` declared BEFORE any of this
/// slice's literal moves — 212 pairs (210 pre-`v2-e3-s4` entries plus
/// `/openapi.json`/`/scalar`, both now representable and registered as of
/// this PR's D3 widening). Generated programmatically from
/// `Registry::entries().flat_map(|e| e.api.routes)`, never hand-typed, and
/// committed once at the start of this slice — an independent expectation
/// captured before the risky step, not a derivation from the current
/// registry that could pass trivially against itself.
///
/// This guard is a carry-forward gate, not a one-time PR1 check: it stays
/// green through every PR boundary of this slice (D9), asserting the V1
/// mount (`/api`, single-namespace) keeps serving every one of these paths
/// exactly as before, regardless of what PR4's literal rewrite or PR5's dual
/// mount change underneath it. It is not deleted at the end of `v2-e3-s4`;
/// S7 retires the V1 mount itself and changes this guard's assertion target
/// then, not its existence.
const V1_ROUTE_PRESENCE_FIXTURE: &str = include_str!("fixtures/v1_route_presence.json");

/// Foreign-method probe order, widest-first, mirroring
/// `api_router_mount_assertion.rs::pick_probe` and
/// `api_route_exclusion_list.rs::foreign_method_for` — reused, not
/// reimplemented, since it is the same "pick a method this path does not
/// register" problem.
const CANDIDATE_ORDER: [HttpMethod; 7] = [
    HttpMethod::Patch,
    HttpMethod::Delete,
    HttpMethod::Put,
    HttpMethod::Post,
    HttpMethod::Get,
    HttpMethod::Head,
    HttpMethod::Options,
];

fn parse_fixture() -> Vec<(HttpMethod, String)> {
    let raw: Vec<(String, String)> =
        serde_json::from_str(V1_ROUTE_PRESENCE_FIXTURE).expect("fixture must be valid JSON");

    raw.into_iter()
        .map(|(method, path)| {
            let method: HttpMethod = method.parse().expect("fixture method must be valid");
            (method, path)
        })
        .collect()
}

fn axum_method(method: HttpMethod) -> reqwest::Method {
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

async fn send(http: &reqwest::Client, method: reqwest::Method, base_url: &str, path: &str) -> u16 {
    http.request(method, format!("{base_url}{path}"))
        .send()
        .await
        .expect("request must not error")
        .status()
        .as_u16()
}

/// T1.15: every fixture entry 404s-or-405s-correctly against the live,
/// single-mount `/api` server — a public route 405s on a foreign method
/// (never probed with its own, business-logic-bearing method, matching
/// `api_route_exclusion_list.rs`'s own reasoning for why "not 404" is never
/// the pass condition); a protected route 401s unauthenticated, proving
/// `require_authn` still sits in front of it.
#[tokio::test]
async fn every_v1_fixture_route_is_reachable_at_its_pre_s4_mount() {
    let fixture = parse_fixture();
    assert_eq!(
        fixture.len(),
        212,
        "the D9 fixture must hold exactly 212 pairs; a different count means the fixture \
         itself has drifted from what it froze"
    );

    let public_paths: HashSet<String> = platform_route_paths()
        .into_iter()
        .chain(custos_route_paths())
        .chain(acta_route_paths())
        .map(|(_, path)| mounted_path("/api", path))
        .collect();

    let mut methods_by_path: HashMap<&str, Vec<HttpMethod>> = HashMap::new();
    for (method, path) in &fixture {
        methods_by_path
            .entry(path.as_str())
            .or_default()
            .push(*method);
    }

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    for (method, path) in &fixture {
        if public_paths.contains(path.as_str()) {
            let declared_methods = methods_by_path
                .get(path.as_str())
                .expect("path must have at least its own method");

            let foreign_method = CANDIDATE_ORDER
                .into_iter()
                .find(|candidate| !declared_methods.contains(candidate))
                .unwrap_or_else(|| {
                    panic!("{path}: every candidate probe method is already declared")
                });

            let status = send(&http, axum_method(foreign_method), server.base_url(), path).await;

            assert_eq!(
                status, 405,
                "public V1 route {method:?} {path} must still be mounted (probed with a \
                 foreign method {foreign_method:?}), got {status}"
            );
        } else {
            let status = send(&http, axum_method(*method), server.base_url(), path).await;

            assert_eq!(
                status, 401,
                "protected V1 route {method:?} {path} must reject an unauthenticated request, \
                 got {status}"
            );
        }
    }

    db.teardown().await;
}
