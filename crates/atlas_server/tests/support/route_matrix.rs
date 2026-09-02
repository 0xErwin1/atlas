#![allow(dead_code)]

use atlas_core::registry::{HttpMethod, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

/// One live route the workspace registry declares (`v2-e3-s2` PR5). Replaces
/// the old hand-maintained route registry, retired in the same PR: the registry
/// itself is now the audited contract for the router (spec acceptance gate
/// 5), so the 401 sweep reads its `(method, path)` surface straight from
/// `atlas_core::registry::Registry` instead of a hand-maintained list.
pub(crate) struct RouteMatrixEntry {
    pub method: HttpMethod,
    pub path_template: String,
    /// `RouteDeclaration.is_public` (`v2-e3-s4`, D7): a direct registry
    /// read, not a recomputed union of `router_audit::*_route_paths()` — D7
    /// promoted that computation into the registry itself, so this field
    /// mirrors the registry's own value rather than re-deriving it.
    pub is_public: bool,
}

/// Builds the full `(method, path)` surface from the live REG-5 registry,
/// classified public/protected by each route's own `is_public` field.
pub(crate) fn route_matrix() -> Vec<RouteMatrixEntry> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    registry
        .entries()
        .iter()
        .flat_map(|entry| entry.api.routes.iter())
        .map(|route| RouteMatrixEntry {
            method: route.method,
            path_template: route.path.as_str().to_string(),
            is_public: route.is_public,
        })
        .collect()
}
