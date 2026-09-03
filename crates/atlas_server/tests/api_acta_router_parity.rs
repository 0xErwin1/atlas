#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_core::registry::HttpMethod;
use atlas_server::router_audit::{mounted_path, namespaces_for};

/// `v2-e3-s2-router-audit` PR4 verify gap: proves acta's 169 routes keep
/// their pre-refactor mount and authentication posture against the
/// ASSEMBLED router (`atlas_server::app`), not just the declarative
/// `declared_routes()` vs. registry comparisons in `acta.rs`'s own module
/// tests (mirrors PR2's `api_platform_router_parity.rs` and PR3's
/// `api_custos_router_parity.rs`).
///
/// This test is DATA-DRIVEN off `router_audit::acta_protected_route_paths()`
/// (`acta::protected_declared_routes()`: every route behind `require_authn`,
/// i.e. `declared_routes()` minus the one public route) rather than a
/// hand-curated sample, because a hand-picked "one representative route per
/// sub-family" sample — the shape PR3's custos test uses, appropriate for
/// custos's much smaller surface — cannot prove the OTHER 160+ acta routes
/// are still mounted and gated: acta is not layer-uniform in its
/// declaration style (`workspace_admin`/`boards_tasks`/`documents_folders`/
/// `search_family`/`webhooks_automations` via `component_routes!`, plus
/// seven hand-declared `layered` routes with a per-route `DefaultBodyLimit`
/// — see `acta.rs`'s own module doc, T4.1), so only an exhaustive sweep
/// proves every one of them still resolves through `require_authn` rather
/// than silently falling through to a 404 for an unmounted path or a
/// non-401 for a mis-wired layer stack.
///
/// For every protected route, this test sends an UNAUTHENTICATED request
/// using the route's own declared method (substituting a fixed placeholder
/// for every `{param}` path segment) and asserts `status == 401`. A 401
/// proves two things at once, which is exactly what makes it sufficient
/// here without a separate mount probe per route:
///   1. `require_authn` sits in front of the route (it rejected the
///      request), and
///   2. the path is actually MOUNTED in the assembled router: an unmounted
///      path never reaches `require_authn` at all, since the middleware is
///      layered onto each merged sub-router's `MethodRouter`
///      (`Router::layer` wraps the whole matched path, method-not-allowed
///      fallback included — confirmed against axum 0.8.9's
///      `path_router.rs::PathRouter::layer`, the same fact
///      `api_router_mount_assertion.rs` relies on for its own 405 probe) —
///      an unmounted path answers axum's own 404 before any layer runs,
///      never 401.
/// This is the proof T4.9's mount assertion (`api_router_mount_assertion.rs`)
/// cannot give for the protected halves of each component: T4.9
/// deliberately probes only each component's PUBLIC routes, because an
/// unauthenticated wrong-method probe against a protected route gets 401
/// from `require_authn` before axum's routing ever reaches method dispatch
/// — indistinguishable from "not mounted" without also standing up a real
/// session. This test supplies the missing protected-side mount proof using
/// the RIGHT method instead of a wrong one, where 401 (not 405) is the
/// expected, meaningful signal.
///
/// 336 unauthenticated requests (168 per namespace) in one test run cannot trip a rate limiter
/// or governor into a spurious 429: `acta::router()` layers `require_authn`
/// LAST (`acta.rs`'s `router()`, the `.layer(require_authn)` call sits below
/// `.layer(rate_limit)` and `.layer(csrf)` in source order), and axum layers
/// wrap outside-in in REVERSE of their `.layer()` call order, so the
/// last-added layer runs FIRST — `require_authn` is therefore the outermost
/// layer and executes before `require_rate_limit` ever sees the request.
/// `require_rate_limit` itself (`middleware/rate_limit.rs`'s
/// `require_rate_limit`) confirms this independently: it keys on the
/// `Principal` extension `require_authn` inserts, and no-ops (`next.run`,
/// no quota check) when that extension is absent — so an unauthenticated
/// request that never reaches past `require_authn` never reaches the
/// limiter's `check()` call at all, regardless of layer order. The one acta
/// route with its own per-route `GovernorLayer` (`ingest_github_event`) is
/// public, excluded from this sweep, and probed separately below with a
/// request count (two) far under its `burst_size(20)` quota
/// (`acta.rs::public::router`), which `lib.rs::app()`'s cloned router
/// shares across both mounts.
///
/// `v2-e3-s4` PR7 (D10): both proofs run once per namespace in
/// `namespaces_for("acta")` (`/api` and `/api/v2/acta`), offenders naming
/// the namespace.
#[tokio::test]
async fn acta_protected_routes_reject_unauthenticated_requests() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    let protected_routes = atlas_server::router_audit::acta_protected_route_paths();
    assert_eq!(
        protected_routes.len(),
        168,
        "acta owns 169 routes total, 1 of which (the GitHub ingest webhook) is public; \
         this sweep must cover all 168 protected routes, not a sample"
    );

    // The one public acta route (the GitHub ingest webhook) sits outside
    // `require_authn` but authenticates itself: the handler answers 401 for a
    // missing or malformed `x-hub-signature-256` header by its own contract
    // (`routes/integrations_ingest.rs`), so a 401 from an unsigned request is
    // the handler speaking, not the middleware, and says nothing about the
    // layer stack either way. Only the mount probe applies here (PATCH-405,
    // mirrors `api_custos_router_parity.rs`'s mount check for its public
    // routes).
    let public_routes = atlas_server::router_audit::acta_route_paths();
    assert_eq!(
        public_routes.len(),
        1,
        "acta has exactly one public route (the GitHub ingest webhook)"
    );
    let (_, public_path_template) = *public_routes
        .first()
        .expect("acta has exactly one public route");

    for namespace in namespaces_for("acta") {
        let namespace = namespace.as_str();
        for (method, path_template) in &protected_routes {
            let path = mounted_path(namespace, &substitute_placeholders(path_template));
            let status = send(&http, axum_method(*method), server.base_url(), &path).await;

            assert_eq!(
                status, 401,
                "namespace {namespace}: protected acta route {method:?} {path_template} must \
                 reject an unauthenticated request, got {status}"
            );
        }

        let public_path = mounted_path(namespace, &substitute_placeholders(public_path_template));

        let mount_status = send(
            &http,
            reqwest::Method::PATCH,
            server.base_url(),
            &public_path,
        )
        .await;
        assert_eq!(
            mount_status, 405,
            "namespace {namespace}: public route {public_path_template} must be mounted in the \
             assembled router (PATCH probe), got {mount_status}"
        );
    }

    db.teardown().await;
}

/// Replaces every `{...}` path-parameter segment with a fixed literal
/// placeholder, so a declared path template like
/// `/api/workspaces/{ws}/tasks/{readable_id}` becomes a concrete URL axum's
/// router can match against a real (if nonexistent) resource id.
fn substitute_placeholders(path_template: &str) -> String {
    let mut result = String::with_capacity(path_template.len());
    let mut chars = path_template.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
            }
            result.push_str("placeholder");
        } else {
            result.push(ch);
        }
    }

    result
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
