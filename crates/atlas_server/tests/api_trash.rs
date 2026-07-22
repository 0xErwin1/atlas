#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use atlas_server::routes::registry::ROUTE_REGISTRY;

#[test]
fn trash_routes_are_registered_for_openapi_and_protection_sweeps() {
    assert!(
        ROUTE_REGISTRY.iter().any(|entry| {
            entry.method == "GET" && entry.openapi_path == Some("/api/admin/trash")
        })
    );
    assert!(ROUTE_REGISTRY.iter().any(|entry| {
        entry.method == "POST" && entry.openapi_path == Some("/api/admin/trash/restore")
    }));
}
