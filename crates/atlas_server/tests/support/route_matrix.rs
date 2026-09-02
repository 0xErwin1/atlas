#![allow(dead_code)]

use std::collections::HashSet;

use atlas_core::registry::{HttpMethod, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use atlas_server::router_audit::{acta_route_paths, custos_route_paths, platform_route_paths};

/// One live route the workspace registry declares (`v2-e3-s2` PR5). Replaces
/// the old hand-maintained route registry, retired in the same PR: the registry
/// itself is now the audited contract for the router (spec acceptance gate
/// 5), so the 401 sweep reads its `(method, path)` surface straight from
/// `atlas_core::registry::Registry` instead of a hand-maintained list.
pub(crate) struct RouteMatrixEntry {
    pub method: HttpMethod,
    pub path_template: String,
    /// Mirrors the old hand-maintained registry's public classification:
    /// true for exactly the routes exposed by a component's
    /// `router_audit::*_route_paths()`
    /// accessor, i.e. the routes PR2-PR4 mounted with no `require_authn`
    /// layer at all. Every other route needs authentication.
    pub is_public: bool,
}

/// Builds the full `(method, path)` surface from the live REG-5 registry,
/// classified public/protected by each component's public route accessor.
pub(crate) fn route_matrix() -> Vec<RouteMatrixEntry> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let public: HashSet<(HttpMethod, &'static str)> = platform_route_paths()
        .into_iter()
        .chain(custos_route_paths())
        .chain(acta_route_paths())
        .collect();

    registry
        .entries()
        .iter()
        .flat_map(|entry| entry.api.routes.iter())
        .map(|route| {
            let path_template = route.path.as_str().to_string();
            let is_public = public
                .iter()
                .any(|(method, path)| *method == route.method && *path == path_template);

            RouteMatrixEntry {
                method: route.method,
                path_template,
                is_public,
            }
        })
        .collect()
}
