//! The one seam every `atlas_server` integration test uses to turn a
//! `(component, relative)` pair into a concrete request path. Since
//! `v2-e3-s7`'s cutover, every route is reachable at exactly one mount —
//! `component`'s own `/api/v2/<component>` namespace — so `api_path`/
//! `api_url` add no logic beyond composing `router_audit::v2_namespace`/
//! `router_audit::mounted_path`, plus D2's component-ownership assertion: an
//! unvalidated wrong component would otherwise pass silently.
//!
//! `canonical_store_path` is a distinct primitive, not a third spelling of
//! the same thing: it always reads
//! `middleware::idempotency::IDEMPOTENCY_STORE_PATH_PREFIX` directly, never
//! `v2_namespace`, because the idempotency middleware canonicalizes its
//! store key to that fixed prefix regardless of which mount served the
//! request (`v2-e3-s5` design D1, D5; `v2-e3-s7` D2).

use super::route_matrix::component_declares;

/// The concrete path `relative` is served at for `component`.
///
/// # Panics
/// Panics when `component` does not declare a route matching `relative`
/// (design D2) — this is the one place that fact is checked, so a wrong
/// component cannot silently produce a byte-identical path.
pub(crate) fn api_path(component: &str, relative: &str) -> String {
    assert_declares(component, relative);
    atlas_server::router_audit::mounted_path(
        &atlas_server::router_audit::v2_namespace(component),
        relative,
    )
}

/// `api_path` prefixed with an absolute `base_url` (`server.base_url()`, or
/// a captured `String` base where the `TestServer` itself is out of scope).
pub(crate) fn api_url(base_url: &str, component: &str, relative: &str) -> String {
    format!("{base_url}{}", api_path(component, relative))
}

/// The canonical idempotency store key for `relative` — the fixed row-key
/// namespace, never a routed mount. Pinned to
/// `middleware::idempotency::IDEMPOTENCY_STORE_PATH_PREFIX` by the
/// middleware's own canonicalization.
pub(crate) fn canonical_store_path(relative: &str) -> String {
    atlas_server::router_audit::mounted_path(
        atlas_server::middleware::idempotency::IDEMPOTENCY_STORE_PATH_PREFIX,
        relative,
    )
}

/// Validates the path part of `relative` against the registry. A query string
/// is not part of any route template, so it is stripped before matching and
/// passes through the composed path untouched.
fn assert_declares(component: &str, relative: &str) {
    let path_only = relative.split('?').next().unwrap_or(relative);

    assert!(
        component_declares(component, path_only),
        "api_path: component `{component}` does not declare a route matching `{relative}` — \
         check the (component, relative) pair against the live registry"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_server::router_audit::{mounted_path, v2_namespace};

    #[test]
    fn api_path_composes_v2_namespace_and_mounted_path_with_no_added_logic() {
        let cases = [
            ("acta", "/workspaces/{ws}/tasks"),
            ("custos", "/workspaces/{ws}/grants"),
            ("platform", "/me/ui-state"),
        ];

        for (component, relative) in cases {
            let expected = mounted_path(&v2_namespace(component), relative);
            assert_eq!(api_path(component, relative), expected);
        }
    }

    #[test]
    fn root_level_paths_stay_unprefixed_only_for_platform() {
        let root_level_paths = ["/health", "/ready", "/version", "/openapi.json", "/scalar"];

        for relative in root_level_paths {
            // E11-S3a design D2/§0.3: the `ROOT_LEVEL_PATHS` exemption is
            // scoped to `platform` — `("platform", GET, "/health")` already
            // IS platform's own probe, so it stays unprefixed.
            assert_eq!(mounted_path(&v2_namespace("platform"), relative), relative);
        }
    }

    #[test]
    fn a_non_platform_components_root_level_named_path_is_namespaced() {
        // The other half of the bidirectional property (design D2/§0.3):
        // `custos`/`acta` declaring a route that shares a name with a
        // `ROOT_LEVEL_PATHS` member (`/health`, `/ready`) does NOT inherit
        // platform's root-level exemption — it is namespaced like any other
        // route, since `custos`'s `/health` is a genuinely different URL
        // from platform's own `/health`.
        for component in ["custos", "acta"] {
            assert_eq!(
                mounted_path(&v2_namespace(component), "/health"),
                format!("/api/v2/{component}/health")
            );
            assert_eq!(
                api_path(component, "/health"),
                format!("/api/v2/{component}/health")
            );
        }
    }

    #[test]
    fn api_path_resolves_at_the_components_v2_mount() {
        assert_eq!(
            api_path("acta", "/workspaces/{ws}/tasks"),
            "/api/v2/acta/workspaces/{ws}/tasks"
        );
    }

    #[test]
    fn api_path_accepts_a_query_string_after_a_declared_route() {
        let composed = api_path("acta", "/workspaces/{ws}/tasks?feed=full");

        assert_eq!(
            composed,
            mounted_path(&v2_namespace("acta"), "/workspaces/{ws}/tasks?feed=full")
        );
    }

    #[test]
    #[should_panic(expected = "component `custos` does not declare a route matching \
                                `/workspaces/{ws}/tasks`")]
    fn api_path_panics_naming_component_and_relative_on_a_wrong_pair() {
        api_path("custos", "/workspaces/{ws}/tasks");
    }

    #[test]
    fn canonical_store_path_always_reads_the_storage_prefix_directly() {
        assert_eq!(
            canonical_store_path("/workspaces/ws/tasks"),
            "/api/workspaces/ws/tasks"
        );
    }
}
