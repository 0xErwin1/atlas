//! The one seam every `atlas_server` integration test uses to turn a
//! `(component, relative)` pair into a concrete request path (`v2-e3-s5`,
//! design D1). `api_path`/`api_url` add no logic beyond composing
//! `router_audit::namespaces_for`/`router_audit::mounted_path`, plus D2's
//! component-ownership assertion: at `DEFAULT_NAMESPACE_INDEX == 0` the
//! component argument is otherwise discarded by `namespaces_for(..)[0]`, so
//! an unvalidated wrong component would pass silently until S7's cutover.
//!
//! `canonical_store_path` is a distinct primitive, not a third spelling of
//! the same thing: it always reads `router_audit::V1_NAMESPACE` directly,
//! never `DEFAULT_NAMESPACE_INDEX`, because the idempotency middleware
//! canonicalizes its store key to the V1 form regardless of which mount a
//! client used (`v2-e3-s5` design D1, D5).

use super::route_matrix::component_declares;

/// The namespace index every `atlas_server` integration test resolves its
/// request paths under. `0` is V1 (`/api/...`), `1` is the route's own V2
/// namespace (`/api/v2/<component>/...`). S7 flips this constant; no test
/// file changes.
pub(crate) const DEFAULT_NAMESPACE_INDEX: usize = 0;

/// The concrete path `relative` is served at for `component`, under the
/// suite's current default namespace.
///
/// # Panics
/// Panics when `component` does not declare a route matching `relative`
/// (design D2) — this is the one place that fact is checked, so a wrong
/// component cannot silently produce a byte-identical V1 path.
pub(crate) fn api_path(component: &str, relative: &str) -> String {
    assert_declares(component, relative);
    let namespaces = atlas_server::router_audit::namespaces_for(component);
    atlas_server::router_audit::mounted_path(&namespaces[DEFAULT_NAMESPACE_INDEX], relative)
}

/// `api_path` prefixed with an absolute `base_url` (`server.base_url()`, or
/// a captured `String` base where the `TestServer` itself is out of scope).
pub(crate) fn api_url(base_url: &str, component: &str, relative: &str) -> String {
    format!("{base_url}{}", api_path(component, relative))
}

/// This `(component, relative)` pair's path as the composed OpenAPI document
/// keys it (`v2-e3-s6` D1.3): always `component`'s own V2 namespace, never
/// `DEFAULT_NAMESPACE_INDEX`. A document-side audit (comparing against
/// `atlas_server::routes::openapi::openapi()`'s own path keys) MUST use this,
/// and MUST NOT use [`api_path`] — the document's mount is not the same fact
/// as the suite's flippable request mount, and S7's flip of
/// `DEFAULT_NAMESPACE_INDEX` moves only [`api_path`].
pub(crate) fn document_path(component: &str, relative: &str) -> String {
    atlas_server::router_audit::mounted_path(
        &atlas_server::router_audit::v2_namespace(component),
        relative,
    )
}

/// The canonical idempotency store key for `relative` — always the `/api`
/// form, never the suite's flippable default. Pinned to
/// `router_audit::V1_NAMESPACE` by the middleware's own canonicalization,
/// so S7's flip must NOT move it.
pub(crate) fn canonical_store_path(relative: &str) -> String {
    atlas_server::router_audit::mounted_path(atlas_server::router_audit::V1_NAMESPACE, relative)
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
    use atlas_server::router_audit::{mounted_path, namespaces_for};

    #[test]
    fn api_path_composes_namespaces_for_and_mounted_path_with_no_added_logic() {
        let cases = [
            ("acta", "/workspaces/{ws}/tasks"),
            ("custos", "/workspaces/{ws}/grants"),
            ("platform", "/me/ui-state"),
        ];

        for (component, relative) in cases {
            let expected = mounted_path(
                &namespaces_for(component)[DEFAULT_NAMESPACE_INDEX],
                relative,
            );
            assert_eq!(api_path(component, relative), expected);
        }
    }

    #[test]
    fn root_level_paths_stay_unprefixed_independent_of_component_or_index() {
        let root_level_paths = ["/health", "/ready", "/version", "/openapi.json", "/scalar"];

        for relative in root_level_paths {
            // Proves inheritance from `mounted_path`'s own `ROOT_LEVEL_PATHS`
            // exemption, not a restatement of it: a locally-hardcoded index
            // `1` computation (never the shipped constant) is exercised
            // alongside `api_path`'s real index-`0` result, so the exemption
            // is shown to hold for both, independent of `component`.
            assert_eq!(api_path("acta", relative), relative);
            assert_eq!(
                mounted_path(&namespaces_for("custos")[1], relative),
                relative
            );
        }
    }

    #[test]
    fn default_namespace_is_v1() {
        // The spec's original acceptance gate 2 example used a placeholder
        // path (`/x`) not declared by any component; design D2's
        // registry-backed `component_declares` assertion (wired in below)
        // would panic on that input, so this test exercises the same
        // property — `DEFAULT_NAMESPACE_INDEX == 0` resolves to the `/api`
        // form — against a real acta-declared relative path instead.
        assert_eq!(
            api_path("acta", "/workspaces/{ws}/tasks"),
            "/api/workspaces/{ws}/tasks"
        );
    }

    #[test]
    fn api_path_accepts_a_query_string_after_a_declared_route() {
        let composed = api_path("acta", "/workspaces/{ws}/tasks?feed=full");

        assert_eq!(
            composed,
            mounted_path(
                &namespaces_for("acta")[DEFAULT_NAMESPACE_INDEX],
                "/workspaces/{ws}/tasks?feed=full"
            )
        );
    }

    #[test]
    #[should_panic(expected = "component `custos` does not declare a route matching \
                                `/workspaces/{ws}/tasks`")]
    fn api_path_panics_naming_component_and_relative_on_a_wrong_pair() {
        api_path("custos", "/workspaces/{ws}/tasks");
    }

    #[test]
    fn canonical_store_path_always_reads_v1_namespace_directly() {
        assert_eq!(
            canonical_store_path("/workspaces/ws/tasks"),
            "/api/workspaces/ws/tasks"
        );
    }

    /// T1.16 (D1.3): `document_path` always resolves at `v2_namespace`,
    /// independent of `DEFAULT_NAMESPACE_INDEX`'s value — asserted with a
    /// locally hardcoded alternate index (`1`), never the shipped constant,
    /// mirroring S5's `root_level_paths_stay_unprefixed_independent_of_
    /// component_or_index` technique.
    #[test]
    fn document_path_always_resolves_at_v2_namespace_independent_of_default_index() {
        const ALTERNATE_INDEX: usize = 1;
        let cases = [
            ("acta", "/workspaces/{ws}/tasks"),
            ("custos", "/workspaces/{ws}/grants"),
            ("platform", "/me/ui-state"),
        ];

        for (component, relative) in cases {
            let expected = mounted_path(&namespaces_for(component)[ALTERNATE_INDEX], relative);
            assert_eq!(document_path(component, relative), expected);
        }
    }
}
