#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::collections::{BTreeSet, HashMap};

use atlas_core::registry::HttpMethod;
use atlas_server::router_audit::{ROOT_LEVEL_PATHS, mounted_path};
use support::route_matrix::route_matrix;

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
    send_with_bearer(http, method, base_url, path, None).await
}

async fn send_with_bearer(
    http: &reqwest::Client,
    method: reqwest::Method,
    base_url: &str,
    path: &str,
    bearer: Option<&str>,
) -> u16 {
    let mut request = http.request(method, format!("{base_url}{path}"));

    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }

    request
        .send()
        .await
        .expect("request must not error")
        .status()
        .as_u16()
}

/// Foreign-method probe order, widest-first, mirroring `pick_probe` above
/// and `api_v1_path_presence_guard.rs::CANDIDATE_ORDER` — the same "pick a
/// method this path does not register" problem, reused rather than
/// reimplemented for every exhaustive probe in this crate.
const CANDIDATE_ORDER: [HttpMethod; 7] = [
    HttpMethod::Patch,
    HttpMethod::Delete,
    HttpMethod::Put,
    HttpMethod::Post,
    HttpMethod::Get,
    HttpMethod::Head,
    HttpMethod::Options,
];

/// T5.1/T5.2/T7.6 (`v2-e3-s4` PR5/PR7, D1/D10): every declared route
/// resolves at BOTH `/api` and its own `/api/v2/<component>`, exhaustively
/// over `route_matrix()` (INV-DATA-DRIVEN,
/// not sampled). Mount proof never uses "not 404" (INV-LIVE-PROOF), and it
/// never uses an unauthenticated 401 either: `lib.rs::app()` merges a
/// PROTECTED root fallback last, so an unroutable path answers 401 to an
/// unauthenticated request exactly like a mounted protected route does
/// (`a_foreign_prefix_never_matches_any_declared_route` below asserts that
/// very 401 for paths that match nothing). A 401 therefore proves only that
/// `require_authn` ran, not that any route resolved. The proof for every
/// route is a live 405 on a foreign method (a method that path does not
/// declare — `pick_probe`'s and `api_v1_path_presence_guard.rs`'s reasoning
/// applies unchanged): a 405 can only come from a matched path whose
/// `MethodRouter` lacks that method, while the fallback answers 404 with an
/// empty body once authenticated. Public routes are probed unauthenticated
/// (to keep the D7 drift check below), protected routes with a bearer token
/// so the probe passes `require_authn` and reaches axum's routing. Every
/// path declares at most 3 methods today (the widest are the 3-method
/// `webhooks/{webhook_id}` and `task-views/{id}`), so the 7-entry
/// `CANDIDATE_ORDER` always yields a foreign method and needs no extension.
///
/// Governor budget: custos's `login`/`activate` governors (burst 5, refill
/// 1/s, one state shared by both mounts because `lib.rs::app()` nests a
/// clone of `custos_router` at `/api/v2/custos` and the original at `/api`)
/// see `POST /api/auth/login` once from `login_user_with_workspace`, plus one
/// login probe per namespace (3 total), and two activate probes per
/// namespace (GET and POST entries share the path; 4 total). Both stay under
/// the burst, so no sleep is needed. `AppState::for_test` sets
/// `rate_limiter: None`, so the authenticated probes have no other budget.
///
/// T5.18/T5.19 (D7): this is also the live half of the `is_public`
/// bidirectional audit at both mount points. A route wrongly declared
/// `is_public: true` while still wrapped in `require_authn` fails HERE, not
/// silently elsewhere: per this file's `every_component_router_is_merged_into_app`
/// doc, `Router::layer` wraps a protected path's entire `MethodRouter`, so an
/// unauthenticated foreign-method probe against a still-wrapped route gets
/// 401 from `require_authn` before axum's own routing ever produces the
/// expected 405 — this test's `assert_eq!(status, 405, ...)` for that route
/// would then fail with 401, naming the offending route. The opposite drift
/// (a route excluded from `require_authn` but not declared `is_public`) is
/// covered by `router_audit::public_route_set_matches_route_paths_union`,
/// the structural half of the same audit — namespace-agnostic by
/// construction (D7): it compares `RouteDeclaration.is_public` against the
/// router-accessor union built from the live `component_routes!` expansion,
/// neither of which reads a mount prefix at all.
///
/// The registry declares 9 public routes: 5 root-level
/// (`router_audit::ROOT_LEVEL_PATHS`: `/health`, `/ready`, `/version`,
/// `/openapi.json`, `/scalar`) and 4 namespaced. `mounted_path` returns a
/// root-level path unchanged for every namespace, so looping those entries
/// over `entry.namespaces()` would probe the same URL twice and prove
/// nothing about either mount. They are probed exactly once, outside the
/// namespace loop, and the set skipped inside it is asserted equal to
/// `ROOT_LEVEL_PATHS` so the skip cannot grow without that constant growing
/// with it.
#[tokio::test]
async fn every_declared_route_resolves_at_both_mounts() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();
    let (client, _, _) = support::login_user_with_workspace(&server, &db, "mount-assertion").await;
    let token = client.token().expect("authenticated token");

    let ws_slug = "mount-assertion-no-such-workspace";
    let matrix = route_matrix();

    let mut methods_by_path: HashMap<&str, Vec<HttpMethod>> = HashMap::new();
    for entry in &matrix {
        methods_by_path
            .entry(entry.path_template.as_str())
            .or_default()
            .push(entry.method);
    }

    let mut skipped_root_level: BTreeSet<String> = BTreeSet::new();

    for entry in &matrix {
        if !ROOT_LEVEL_PATHS.contains(&entry.path_template.as_str()) {
            continue;
        }
        skipped_root_level.insert(entry.path_template.clone());

        let declared_methods = methods_by_path
            .get(entry.path_template.as_str())
            .expect("path must have at least its own declared method");
        let foreign_method = CANDIDATE_ORDER
            .into_iter()
            .find(|candidate| !declared_methods.contains(candidate))
            .unwrap_or_else(|| {
                panic!(
                    "{}: every candidate probe method is already declared",
                    entry.path_template
                )
            });

        assert!(
            entry.is_public,
            "root-level route {} {} must be public: ROOT_LEVEL_PATHS sits outside both \
             mounts and outside require_authn, so a protected root-level route has no proof \
             shape here",
            entry.method, entry.path_template
        );

        let status = send(
            &http,
            axum_method(foreign_method),
            server.base_url(),
            &entry.path_template,
        )
        .await;
        assert_eq!(
            status, 405,
            "root-level: public route {} {} must be mounted (probed with a foreign method \
             {foreign_method:?}), got {status}",
            entry.method, entry.path_template
        );
    }

    let expected_root_level: BTreeSet<String> = ROOT_LEVEL_PATHS
        .iter()
        .map(|path| (*path).to_string())
        .collect();
    assert_eq!(
        skipped_root_level, expected_root_level,
        "the set of routes probed once as root-level must be exactly ROOT_LEVEL_PATHS: a \
         registry route can only leave the namespace loop by joining ROOT_LEVEL_PATHS, and \
         every ROOT_LEVEL_PATHS member must still be declared by the registry"
    );

    for entry in &matrix {
        if ROOT_LEVEL_PATHS.contains(&entry.path_template.as_str()) {
            continue;
        }

        let relative = entry.path_template.replace("{ws}", ws_slug);

        for namespace in entry.namespaces() {
            let mounted = mounted_path(&namespace, &relative);

            let declared_methods = methods_by_path
                .get(entry.path_template.as_str())
                .expect("path must have at least its own declared method");
            let foreign_method = CANDIDATE_ORDER
                .into_iter()
                .find(|candidate| !declared_methods.contains(candidate))
                .unwrap_or_else(|| {
                    panic!(
                        "{}: every candidate probe method is already declared",
                        entry.path_template
                    )
                });

            let bearer = (!entry.is_public).then_some(token);
            let visibility = if entry.is_public {
                "public"
            } else {
                "protected"
            };
            let status = send_with_bearer(
                &http,
                axum_method(foreign_method),
                server.base_url(),
                &mounted,
                bearer,
            )
            .await;
            assert_eq!(
                status, 405,
                "namespace {namespace}: {visibility} route {} {mounted} must be mounted (probed \
                 with a foreign method {foreign_method:?}), got {status}",
                entry.method
            );
        }
    }

    db.teardown().await;
}

/// T5.3/T5.4 (`v2-e3-s4` PR5, D1): a foreign prefix never matches any
/// declared route at either mount — `/apiv2/<rel>` and `/api/v3/<rel>` fall
/// through to the root fallback for every entry in `route_matrix()`. That
/// fallback is protected (`lib.rs::app()` merges
/// `protect(Router::new().fallback(not_found))` last), so the proof has two
/// halves per path, the same shape `api_unmatched_path_fallback.rs` proves
/// for hand-picked paths: an unauthenticated probe answers 401 from
/// `require_authn`, and an authenticated one reaches the fallback and
/// answers 404. Asserting a bare 404 on the unauthenticated probe would be
/// wrong (it gets 401), and asserting only 401 would not distinguish "no
/// route matched" from "a protected route matched", which is exactly what
/// the authenticated 404 rules out. Deliberately built with `format!`
/// rather than `mounted_path`/`joined`: the point here is proving the
/// LITERAL foreign-prefixed path never resolves, not reconstructing where
/// the route is actually mounted.
#[tokio::test]
async fn a_foreign_prefix_never_matches_any_declared_route() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();
    let (client, _, _) = support::login_user_with_workspace(&server, &db, "foreign-prefix").await;
    let token = client.token().expect("authenticated token");

    let ws_slug = "foreign-prefix-no-such-workspace";
    let matrix = route_matrix();

    for foreign_prefix in ["/apiv2", "/api/v3"] {
        for entry in &matrix {
            let relative = entry.path_template.replace("{ws}", ws_slug);
            let foreign = format!("{foreign_prefix}{relative}");

            let unauthenticated = send(
                &http,
                axum_method(entry.method),
                server.base_url(),
                &foreign,
            )
            .await;
            assert_eq!(
                unauthenticated, 401,
                "foreign prefix {foreign_prefix}: unauthenticated {} {foreign} must fall through \
                 to the protected fallback and get 401, got {unauthenticated}",
                entry.method
            );

            let authenticated = http
                .request(
                    axum_method(entry.method),
                    format!("{}{foreign}", server.base_url()),
                )
                .bearer_auth(token)
                .send()
                .await
                .expect("request must not error")
                .status()
                .as_u16();
            assert_eq!(
                authenticated, 404,
                "foreign prefix {foreign_prefix}: authenticated {} {foreign} must not match any \
                 route and reach the 404 fallback, got {authenticated}",
                entry.method
            );
        }
    }

    db.teardown().await;
}

/// The three component ids `lib.rs::app()` nests under `/api/v2` (`v2-e3-s4`
/// PR7, D10). Kept as a small fixed list rather than derived from the
/// registry: the registry has no "list every component id" accessor, and
/// this list is exactly the set `namespaces_for` and `app()`'s three
/// `.nest("/api/v2/<component>", ...)` calls are also hand-written against.
const ALL_COMPONENTS: [&str; 3] = ["platform", "custos", "acta"];

/// T7.8/T7.9 (`v2-e3-s4` PR7, D10): a flat `/api/v2/<rel>` (no component
/// segment) and a wrong-component `/api/v2/<other>/<rel>` both match
/// nothing, for every declared route — exactly the same protected-fallback
/// shape `a_foreign_prefix_never_matches_any_declared_route` proves for
/// `/apiv2` and `/api/v3`: an unauthenticated probe gets 401 from
/// `require_authn`, and an authenticated one reaches the fallback and gets
/// 404. `<other>` cycles through the two components that do NOT own the
/// entry, so a route is never accidentally probed against its own mount.
#[tokio::test]
async fn flat_and_wrong_component_v2_forms_never_match_any_declared_route() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();
    let (client, _, _) =
        support::login_user_with_workspace(&server, &db, "flat-wrong-component").await;
    let token = client.token().expect("authenticated token");

    let ws_slug = "flat-wrong-component-no-such-workspace";
    let matrix = route_matrix();

    for entry in &matrix {
        let relative = entry.path_template.replace("{ws}", ws_slug);

        let other_components: Vec<&str> = ALL_COMPONENTS
            .into_iter()
            .filter(|component| *component != entry.component)
            .collect();
        assert_eq!(
            other_components.len(),
            2,
            "{}: entry.component {} must be exactly one of {ALL_COMPONENTS:?}",
            entry.path_template,
            entry.component
        );

        let mut candidates: Vec<String> = vec![format!("/api/v2{relative}")];
        for other in other_components {
            candidates.push(format!("/api/v2/{other}{relative}"));
        }

        for candidate in candidates {
            let unauthenticated = send(
                &http,
                axum_method(entry.method),
                server.base_url(),
                &candidate,
            )
            .await;
            assert_eq!(
                unauthenticated, 401,
                "{}: unauthenticated {} {candidate} must fall through to the protected \
                 fallback and get 401, got {unauthenticated}",
                entry.path_template, entry.method
            );

            let authenticated = http
                .request(
                    axum_method(entry.method),
                    format!("{}{candidate}", server.base_url()),
                )
                .bearer_auth(token)
                .send()
                .await
                .expect("request must not error")
                .status()
                .as_u16();
            assert_eq!(
                authenticated, 404,
                "{}: authenticated {} {candidate} must not match any route and reach the 404 \
                 fallback, got {authenticated}",
                entry.path_template, entry.method
            );
        }
    }

    db.teardown().await;
}
