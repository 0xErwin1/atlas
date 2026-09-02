#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_core::registry::HttpMethod;

/// T4.9 (`v2-e3-s2-router-audit` PR4, added after PR2's verify — the "MOUNT
/// GAP" note): proves each of the three component routers is actually
/// `.merge()`d into the real `atlas_server::app()`.
///
/// Every other audit in this slice (D2's bidirectional set-equality test,
/// D5's declare-and-verify test) compares two in-process data structures —
/// `declared_routes()` against the registry's `ComponentEntry.api.routes` —
/// and never touches a live `axum::Router`, let alone the assembled `app()`.
/// Deleting `.merge(routes::<component>::router(state))` from `app()` would
/// leave every one of those audits green, because none of them ever calls
/// `router()` or spawns a server. This is the only test in the suite that
/// spawns the real assembled router and proves, per component, that one of
/// its declared routes actually answers through it.
///
/// The probed path for each component is picked FROM that component's own
/// declared route set (`router_audit::{platform,custos,acta}_route_paths()`),
/// never a hand-typed literal, so a future route addition or removal cannot
/// silently stop being represented here. Those helpers deliberately expose
/// only each component's `public` (no-`require_authn`-layer) routes, NOT the
/// full `declared_routes()` union: `Router::layer` wraps each mounted path's
/// entire `MethodRouter`, including its own method-not-allowed fallback
/// (confirmed against axum 0.8.9's `path_router.rs::PathRouter::layer`), so
/// an unauthenticated wrong-method probe against an authenticated route
/// would get 401 from `require_authn` before axum's routing ever produces a
/// 405 — indistinguishable from "not mounted" without also standing up a
/// real session. Every component's public set is non-empty today
/// (`platform`: health/ready/version; `custos`: login/activate; `acta`:
/// the GitHub ingest webhook), so this is not a hypothetical fallback.
///
/// Mount proof never uses "status != 404": several real handlers legitimately
/// answer 404 for their own business reason (e.g. `activate`'s unknown-token
/// 404, per `api_custos_router_parity.rs`'s own doc). Instead, each probed
/// path is hit with one HTTP method that path does NOT register. A mounted
/// axum path answers a foreign method with 405 Method Not Allowed regardless
/// of what its real methods return; an unmounted path answers 404 for every
/// method. The per-route layers on `custos`'s governed `login`/`activate`
/// (the only per-route-layered routes among the three components' PUBLIC
/// subsets — acta's `DefaultBodyLimit` routes and its own governed ingest
/// route sit in its `protected`/`layered` modules, not `public`) still wrap
/// their method-not-allowed fallback the same way, so a per-route layer
/// never defeats this probe — the exact mechanism `api_custos_router_parity.rs`
/// already exercises for its own PATCH-probe mount check. One request per
/// component (three total) keeps every governed route's request count far
/// under any rate limiter's burst size.
///
/// Adversarial proof (see PR4's verification checklist): temporarily
/// removing `.merge(routes::acta::router(state))` from `app()` does not stop
/// the crate from compiling — `app()`'s return type is unchanged — but it
/// does stop `/api/workspaces/{ws}/integrations/{integration}/events` (this
/// test's acta probe path, `acta::public`'s only declared route) from being
/// served at all: the PATCH probe below would then receive 404 (no route
/// registered for that path under any method), not 405, and
/// `assert_eq!(status, 405, ...)` would fail, naming `acta` in its message.
/// This was verified by editing `lib.rs`, confirming the change compiles as
/// expected via `cargo check -p atlas_server --all-targets`, then
/// reverting — the test itself cannot run in this sandbox (rootless podman
/// UID/GID failure blocks every container-backed test here; CI is the
/// gate), so the failure mode is reasoned from source rather than observed.
/// The same reasoning applies symmetrically to `platform` (probe path
/// `/health`) and `custos` (probe path `/api/activate/{token}`).
#[tokio::test]
async fn every_component_router_is_merged_into_app() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    let components: [(&str, Vec<(HttpMethod, &'static str)>); 3] = [
        (
            "platform",
            atlas_server::router_audit::platform_route_paths(),
        ),
        ("custos", atlas_server::router_audit::custos_route_paths()),
        ("acta", atlas_server::router_audit::acta_route_paths()),
    ];

    for (component, routes) in components {
        assert!(
            !routes.is_empty(),
            "{component} must declare at least one route to probe"
        );

        let (path, probe_method) = pick_probe(component, &routes);
        let mounted = atlas_server::router_audit::mounted_path("/api", path);

        let status = send(&http, probe_method.clone(), server.base_url(), &mounted).await;

        assert_eq!(
            status, 405,
            "{component}'s route {path} must be mounted in the assembled app() \
             (probed with {probe_method}, a method that path does not register), got {status}"
        );
    }

    db.teardown().await;
}

/// Deterministically picks one `(path, probe_method)` pair from a
/// component's declared routes: the lexicographically first path, and the
/// first HTTP method (in a fixed candidate order) that path does NOT
/// register for any of its declared methods. Deterministic so the test's
/// outcome does not depend on `Vec`/`HashMap` iteration order across runs.
fn pick_probe(
    component: &str,
    routes: &[(HttpMethod, &'static str)],
) -> (&'static str, reqwest::Method) {
    let mut paths: Vec<&'static str> = routes.iter().map(|(_, path)| *path).collect();
    paths.sort_unstable();
    paths.dedup();

    const CANDIDATE_ORDER: [HttpMethod; 7] = [
        HttpMethod::Patch,
        HttpMethod::Delete,
        HttpMethod::Put,
        HttpMethod::Post,
        HttpMethod::Get,
        HttpMethod::Head,
        HttpMethod::Options,
    ];

    for path in paths {
        let declared_methods: Vec<HttpMethod> = routes
            .iter()
            .filter(|(_, candidate_path)| *candidate_path == path)
            .map(|(method, _)| *method)
            .collect();

        if let Some(probe_method) = CANDIDATE_ORDER
            .into_iter()
            .find(|candidate| !declared_methods.contains(candidate))
        {
            return (path, axum_method(probe_method));
        }
        // This path registers every candidate method; try the next path
        // rather than fail outright (T4.9's mandate: "if a path registers
        // ALL methods you care about, say so and handle it explicitly").
    }

    panic!(
        "{component}: every declared path registers all {} candidate probe methods; \
         pick_probe needs a wider candidate set",
        CANDIDATE_ORDER.len()
    );
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
