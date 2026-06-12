#![allow(unused_imports)]

/// Re-export the canonical route registry from the library so the test suite
/// has a single source of truth for route enumeration.
///
/// Both the 401-sweep test and the OpenAPI drift test consume this registry
/// instead of maintaining parallel hand-written lists.
pub(crate) use atlas_server::routes::registry::{ROUTE_REGISTRY, RouteEntry, RouteKind};

/// Alias for backward-compat with the existing 401 sweep test.
pub(crate) static ROUTE_MATRIX: &[RouteEntry] = ROUTE_REGISTRY;
