//! Registry-derived `(method, mounted path) -> (component, operation)` map
//! (design D1, D-S7-2). Built once at `AppState` construction and cached
//! there; nothing on the request path calls [`RouteIndex::from_registry`] or
//! rebuilds the registry (INV-MAP-BUILT-ONCE).

use std::collections::HashMap;
use std::sync::Arc;

use atlas_core::registry::{HttpMethod, Registry};

use crate::router_audit::{mounted_path, v2_namespace};

/// The component and OpenAPI operation that own one served route.
#[derive(Debug, Clone)]
pub struct RouteTag {
    pub component: Arc<str>,
    pub operation: Arc<str>,
}

/// `(method, mounted path template) -> RouteTag`, built once from the
/// registry (design D1).
#[derive(Debug, Default)]
pub struct RouteIndex(HashMap<HttpMethod, HashMap<String, RouteTag>>);

impl RouteIndex {
    /// Walks every declared route in `registry` and keys it on
    /// `(route.method, mounted_path(&v2_namespace(stable_id), route.path))`
    /// — the same two functions `idempotent_route_set`
    /// (`routes/openapi.rs`) already uses, never a second path-building
    /// rule (design §0.2).
    pub fn from_registry(registry: &Registry) -> Self {
        let mut map: HashMap<HttpMethod, HashMap<String, RouteTag>> = HashMap::new();

        for entry in registry.entries() {
            let namespace = v2_namespace(entry.identity.stable_id.as_str());
            let component: Arc<str> = Arc::from(entry.identity.stable_id.as_str());

            for route in &entry.api.routes {
                let mounted = mounted_path(&namespace, route.path.as_str());
                let tag = RouteTag {
                    component: component.clone(),
                    operation: Arc::from(route.operation_id.as_str()),
                };

                map.entry(route.method).or_default().insert(mounted, tag);
            }
        }

        Self(map)
    }

    /// Looks up the `RouteTag` owning `template` under `method`, converting
    /// `method` through [`to_registry_method`]. Returns `None` for an
    /// unmappable HTTP method or an unindexed `(method, template)` pair.
    /// The lookup borrows `template`, so the per-request path allocates
    /// nothing.
    pub fn get(&self, method: &axum::http::Method, template: &str) -> Option<&RouteTag> {
        let method = to_registry_method(method)?;
        self.0.get(&method)?.get(template)
    }

    /// The number of `(method, path)` entries the index holds.
    pub fn len(&self) -> usize {
        self.0.values().map(HashMap::len).sum()
    }

    /// Whether the index holds no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// An index with no entries — test-only (`test_app_with_route`, which
    /// needs no registry).
    pub fn empty() -> Self {
        Self(HashMap::new())
    }
}

/// Converts `axum`'s wire-level HTTP method into the registry's own
/// [`HttpMethod`] through the registry's `FromStr`, so the two never
/// disagree on the accepted set. Methods the registry does not declare
/// (`CONNECT`, `TRACE`, and any extension method) yield `None`, and a
/// request using one of them is looked up as untagged.
fn to_registry_method(method: &axum::http::Method) -> Option<HttpMethod> {
    method.as_str().parse().ok()
}
