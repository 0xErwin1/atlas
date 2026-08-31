//! Workspace test for SHELL-REG-5 (`v2-e3-s1-registry-population`).
//!
//! Proves `atlas_server::reg5::reg5_component_entries()` builds a valid
//! `atlas_core::registry::Registry`: exactly the REG-5 `stable_id` set, zero
//! `RegistryBuildError`s, and every declared `platform`/`custos`/`acta` route
//! accounted for against the live `atlas_server::lib::app()` router.
//!
//! `storage.filesystem` and `storage.s3` are proven independently valid
//! (`storage_backend_selection_is_independently_valid_either_way`) rather
//! than together, since both provide the mandatory `storage.blob` capability
//! and `registry::build()` rejects two providers of the same mandatory
//! capability. See `docs/registry-route-ownership.md` for the full
//! rationale.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_core::registry::{ComponentId, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

fn component(value: &str) -> ComponentId {
    ComponentId::new(value).expect("valid component id")
}

#[test]
fn reg5_component_entries_build_successfully() {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let mut stable_ids: Vec<&str> = registry
        .entries()
        .iter()
        .map(|entry| entry.identity.stable_id.as_str())
        .collect();
    stable_ids.sort_unstable();
    assert_eq!(
        stable_ids,
        vec![
            "acta",
            "custos",
            "platform",
            "search.pgvector_embeddings",
            "search.postgres_fts",
            "storage.filesystem",
        ],
        "the active REG-5 stable_id set must equal exactly this (storage.s3 excluded, see StorageBackend)"
    );

    for absent in [
        "hermes",
        "infrastructure",
        "minerva",
        "mnemosyne",
        "storage.s3",
    ] {
        assert!(
            registry.get(&component(absent)).is_none(),
            "REG-5 must not contain a `{absent}` entry"
        );
    }

    let migration_order: Vec<&str> = registry
        .migration_order()
        .iter()
        .map(ComponentId::as_str)
        .collect();
    assert_eq!(
        migration_order,
        vec!["platform", "custos", "acta"],
        "Custos migrations must run before Acta (SHELL schema_contracts_required)"
    );
}

/// `storage.s3` is the mandatory `storage.blob` alternative to
/// `storage.filesystem`: proving it also builds cleanly on its own confirms
/// the REG-5 Module entry itself is valid, without ever passing both
/// storage backends into the same `build()` call (which `build()` would
/// reject as `MandatoryCapabilityAmbiguous`).
#[test]
fn storage_backend_selection_is_independently_valid_either_way() {
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        build(reg5_component_entries(backend)).unwrap_or_else(|errors| {
            panic!("REG-5 entries with {backend:?} must build: {errors:?}")
        });
    }
}

/// Stand-in for S2's full bidirectional router<->registry audit (which needs
/// router derivation this slice explicitly excludes): proves no route was
/// dropped or double-counted while partitioning the 138 live `.route()`
/// calls in `atlas_server::app()` across `platform`/`custos`/`acta`. It does
/// not prove route-by-route identity — see
/// `docs/registry-route-ownership.md`.
#[test]
fn declared_route_count_matches_the_live_router_enumeration() {
    let entries = reg5_component_entries(StorageBackend::Filesystem);

    let declared_route_count: usize = entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.identity.stable_id.as_str(),
                "platform" | "custos" | "acta"
            )
        })
        .map(|entry| entry.api.routes.len())
        .sum();

    // 138 manual `.route()` calls in `atlas_server::lib::app()` expand to 211
    // (method, path) pairs once every `.get()/.post()/...` combinator on a
    // shared path is counted individually. `/openapi.json` is one of them but
    // cannot be represented as a `RouteDeclaration` (`RoutePath` rejects the
    // `.` in its only segment), leaving 210 declared. See
    // `docs/registry-route-ownership.md` for the full table.
    //
    // This assertion compares totals only, which is deliberately weak: a
    // dropped declaration paired with an equally undercounted expectation
    // passes it (that is exactly how `PATCH /api/admin/status-templates/
    // {template_id}` survived the first draft of this slice). The real
    // guarantee is S2's bidirectional audit, which compares the declared set
    // against the live router's set element by element; this count is only a
    // cheap tripwire until that lands.
    assert_eq!(
        declared_route_count, 210,
        "platform + custos + acta declared routes must equal the live router's 210 representable (method, path) pairs"
    );
}
