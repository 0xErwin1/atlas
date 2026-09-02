//! `platform` component router (design D1/D5/D6, `v2-e3-s2-router-audit`
//! PR2, the pilot conversion).
//!
//! Platform's original six routes sit in two of today's `lib.rs` route trees
//! exactly as they do today: `/health`, `/ready`, `/version` carry no auth
//! layer (`lib.rs`'s `public` router); `/api/me/ui-state` and `/api/meta` sit
//! in `lib.rs`'s `protected` router, behind `require_authn` →
//! `require_rate_limit` → CSRF-for-cookie-mutations. `router()` below
//! reproduces that split internally via two private sub-modules (D6), so the
//! layering after this conversion is provably identical to before it — same
//! layers, same order — not just "close enough." `GET /openapi.json` and
//! `GET /scalar` (`v2-e3-s4`, D3) are two more platform-owned routes added
//! to the registry in this slice; they are hand-declared for audit purposes
//! (`openapi_document_declared_routes()` below) but stay physically mounted
//! in `routes::acta::public::router()`, unaffected by this slice.
//!
//! None of the eight routes take an `Authorized<R, M, S>` extractor at all:
//! `health`/`ready`/`version` take no parameters or `State<AppState>` only,
//! `meta` takes `State<AppState>` only, `get_ui_state`/`set_ui_state` take
//! `Extension<Principal>` (an authenticated-human check, not a capability
//! gate), and `openapi_json`/`scalar_router` take no principal at all. All
//! eight are therefore on the capability-extraction exemption list (D5); the
//! registry declares `action: None` for every one, matching.
//!
//! Resolving the open question (design §5): `/api/meta`'s handler
//! (`routes::health::meta`) is genuinely unauthenticated at the
//! capability-extraction level — its only parameter is `State<AppState>`,
//! there is no `Authorized<...>` extractor to read `S::CAPABILITY` off — so
//! it belongs on the exemption list, not on `Authorized<_, _, NoScope>`.
//! Verified by reading `crates/atlas_server/src/routes/health.rs:82`
//! directly.
#![allow(
    unreachable_pub,
    reason = "component_routes! always emits `pub fn router` to match the real \
              per-component contract (D1); this module's own path is `pub(crate)` \
              (`routes::platform` in `routes/mod.rs`), so the `pub` here is never \
              reachable from outside the crate, which is by design, not an oversight"
)]

use axum::Router;

use crate::router_audit::{AuditedRoute, DeclaredScope};
use crate::state::AppState;

/// `GET /openapi.json` and `GET /scalar` (`v2-e3-s4`, D3): owned by
/// `platform` per `docs/registry-route-ownership.md`, but physically mounted
/// in `routes::acta::public::router()` (unaffected by this slice — see that
/// module's own doc for why). Hand-declared here, not built by
/// `component_routes!`, because `router()` below does not construct these
/// two routes itself — mirroring how `routes::custos` and `routes::acta`
/// hand-declare a route their own `router()` mounts outside the macro.
/// Declaring them here (rather than leaving them undeclared, as
/// `ROUTE_SET_EXCLUSIONS` did before this slice) is what lets platform's
/// bidirectional registry audit see them as ordinary members instead of a
/// carve-out. Neither takes an `Authorized<...>` extractor, matching every
/// other platform route's exemption (D5).
fn openapi_document_declared_routes() -> Vec<AuditedRoute> {
    vec![
        AuditedRoute {
            method: atlas_core::registry::HttpMethod::Get,
            path: "/openapi.json",
            scope: DeclaredScope::Unauthenticated,
            idempotent: false,
            one_shot: false,
        },
        AuditedRoute {
            method: atlas_core::registry::HttpMethod::Get,
            path: "/scalar",
            scope: DeclaredScope::Unauthenticated,
            idempotent: false,
            one_shot: false,
        },
    ]
}

/// Routes with no auth layer today (`lib.rs`'s `public` router, pre-PR2:
/// `lib.rs:803-806`).
mod public {
    use crate::routes::health;
    use crate::state::AppState;

    crate::component_routes! {
        state: AppState;
        "/health" => [ get(health::health, exempt) ];
        "/ready" => [ get(health::ready, exempt) ];
        "/version" => [ get(health::version, exempt) ];
    }
}

/// Routes behind `require_authn` → `require_rate_limit` → CSRF-for-cookie-
/// mutations today (`lib.rs`'s `protected` router, pre-PR2: `lib.rs:178-184`).
mod protected {
    use crate::routes::{health, ui_state};
    use crate::state::AppState;

    crate::component_routes! {
        state: AppState;
        "/api/me/ui-state" => [
            get(ui_state::get_ui_state, exempt),
            put(ui_state::set_ui_state, exempt)
        ];
        "/api/meta" => [ get(health::meta, exempt) ];
    }
}

/// Builds platform's router, reproducing today's exact public/protected
/// split and layer stack (D6): no layer added, removed, or reordered, only
/// the `.route()` call sites moved out of `lib.rs::app()` into this module.
pub fn router(state: AppState) -> Router {
    let public = public::router(state.clone());
    let protected = protected::router(state.clone())
        .layer(axum::middleware::from_fn(
            crate::auth::csrf::require_csrf_for_cookie_mutations,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::rate_limit::require_rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::middleware::require_authn,
        ));

    public.merge(protected)
}

/// The union of both sub-routers' declared routes. Order-independent: the
/// audits below compare sets, never sequences. Only this module's own
/// bidirectional/declare-and-verify audit tests call the full union today
/// (T4.9's mount assertion deliberately uses the narrower
/// `public_declared_routes()` below instead); PR5's T5.7 crate-wide sweep is
/// this function's next production caller.
#[allow(
    dead_code,
    reason = "only this module's own #[cfg(test)] audit tests call the full \
              declared_routes() union outside a test build; PR5's T5.7 crate-wide \
              sweep is its next production caller"
)]
pub(crate) fn declared_routes() -> Vec<AuditedRoute> {
    let mut routes = public::declared_routes();
    routes.extend(protected::declared_routes());
    routes.extend(openapi_document_declared_routes());
    routes
}

/// Just the routes mounted with no auth layer at all (`public`, above), read
/// by `router_audit::platform_route_paths()` for T4.9's per-component mount
/// assertion: unlike the full `declared_routes()` union, every route here is
/// safe to probe with an unauthenticated request and a foreign method,
/// since none of them sit behind `require_authn` — a probe against a
/// `protected` route would get 401 from that layer before ever reaching the
/// method dispatch that would otherwise answer 405.
pub(crate) fn public_declared_routes() -> Vec<AuditedRoute> {
    let mut routes = public::declared_routes();
    routes.extend(openapi_document_declared_routes());
    routes
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use atlas_core::registry::{ComponentId, HttpMethod, build};

    use super::*;
    use crate::reg5::{StorageBackend, reg5_component_entries};
    use crate::router_audit::{
        DeclaredScope, capability_from_action_id, diff_declared_and_enforced, diff_route_sets,
    };

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    /// Builds the same registry the live server assembles at startup. The
    /// storage backend selection is irrelevant here — `platform` never
    /// depends on `storage.blob` — but a value is required to call
    /// `reg5_component_entries`. `ComponentEntry` has no `Clone`, so callers
    /// build and query the registry within one scope rather than extracting
    /// an owned entry from this helper.
    fn build_registry() -> atlas_core::registry::Registry {
        build(reg5_component_entries(StorageBackend::Filesystem))
            .expect("REG-5 entries must satisfy every registry::build() validator")
    }

    /// T2.2/T2.4 (bidirectional audit, D2/INV-SET): `platform::declared_routes()`
    /// (co-generated with `router()` by the same macro invocation, so it IS
    /// the live router's route set by construction — the mechanism PR1's
    /// SH10 test already proved has teeth) must equal the registry's
    /// `platform.api.routes` set exactly, in both directions.
    #[test]
    fn platform_router_and_registry_route_sets_match_exactly() {
        let registry = build_registry();
        let entry = registry
            .get(&component("platform"))
            .expect("platform is a REG-5 component");

        let router_set: std::collections::HashSet<(HttpMethod, String)> = declared_routes()
            .iter()
            .map(|route| (route.method, route.path.to_string()))
            .collect();

        let registry_set: std::collections::HashSet<(HttpMethod, String)> = entry
            .api
            .routes
            .iter()
            .map(|route| (route.method, route.path.as_str().to_string()))
            .collect();

        let diff = diff_route_sets(&router_set, &registry_set);

        assert!(
            diff.is_empty(),
            "platform's router and registry route sets must match exactly: {diff:?}"
        );
    }

    /// T2.7 (declare-and-verify, D5): for every one of platform's six
    /// routes, the registry's declared `action` must agree with the
    /// handler's enforced capability. Platform declares `action: None` for
    /// all six (none enforce an `Authorized<...>` capability gate), so this
    /// degenerates to the exemption-side assertion — the meaningful check
    /// per T2.7, since there is no `action: Some(_)` route to exercise the
    /// positive case for this component.
    #[test]
    fn platform_declared_actions_match_enforced_capabilities() {
        let registry = build_registry();
        let entry = registry
            .get(&component("platform"))
            .expect("platform is a REG-5 component");
        assert!(
            entry.api.routes.iter().all(|route| route.action.is_none()),
            "platform declares no Some(_) action; this test's positive case is empty by fact, \
             not by omission — see the module doc"
        );

        let routes = declared_routes();
        assert!(
            routes
                .iter()
                .all(|route| route.scope == DeclaredScope::Unauthenticated),
            "every platform route is capability-extraction exempt (D5): none take an \
             Authorized<...> extractor"
        );

        let declared_actions: HashMap<(HttpMethod, &'static str), Option<_>> = routes
            .iter()
            .map(|route| {
                let action = entry
                    .api
                    .routes
                    .iter()
                    .find(|declared| {
                        declared.method == route.method && declared.path.as_str() == route.path
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "registry has no declaration for {:?} {}",
                            route.method, route.path
                        )
                    })
                    .action
                    .as_ref()
                    .map(capability_from_action_id);
                ((route.method, route.path), action)
            })
            .collect();

        let mismatches = diff_declared_and_enforced(&routes, &declared_actions);

        assert!(
            mismatches.is_empty(),
            "declared vs enforced must agree for every platform route: {mismatches:?}"
        );
    }
}
