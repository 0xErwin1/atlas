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

use atlas_core::registry::{ComponentEntry, ComponentId, ComponentKind, WorkerId, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

fn component(value: &str) -> ComponentId {
    ComponentId::new(value).expect("valid component id")
}

fn worker(value: &str) -> WorkerId {
    WorkerId::new(value).expect("valid worker id")
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
    // shared path is counted individually. `/openapi.json` was one of them
    // but could not be represented as a `RouteDeclaration` before
    // `v2-e3-s4` D3 (`RoutePath` rejected the `.` in its only segment), and
    // `/scalar` was never among the 211 (`.merge()`, not a `.route()` call).
    // D3 widened `RoutePath` and declared both as ordinary `platform`
    // routes, raising the total to 212. See `docs/registry-route-ownership.md`
    // for the full table. E11-S3a design D2 added 4 more: `custos`'s and
    // `acta`'s own namespaced `/health`/`/ready` probes, raising the total
    // to 216.
    //
    // This assertion compares totals only, which is deliberately weak: a
    // dropped declaration paired with an equally undercounted expectation
    // passes it (that is exactly how `PATCH /api/admin/status-templates/
    // {template_id}` survived the first draft of this slice). The real
    // guarantee is S2's bidirectional audit, which compares the declared set
    // against the live router's set element by element; this count is only a
    // cheap tripwire until that lands.
    assert_eq!(
        declared_route_count, 216,
        "platform + custos + acta declared routes must equal the live router's 216 representable (method, path) pairs"
    );
}

/// The six E11-S2 PR1 worker declarations: `acta`'s five workers plus
/// `search.pgvector_embeddings`'s one, all `critical: false` (SH3 forces the
/// dispatcher to be, and no other worker has a stated readiness
/// consequence). Run for both storage backends since neither declares a
/// worker — a non-vacuous, real-data proof (design D5's non-vacuity note).
#[test]
fn reg5_declares_exactly_six_workers_all_non_critical() {
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        let entries = reg5_component_entries(backend);

        let mut worker_ids: Vec<&str> = entries
            .iter()
            .flat_map(|entry: &ComponentEntry| entry.workers.iter())
            .map(|declaration| declaration.id.as_str())
            .collect();
        worker_ids.sort_unstable();

        assert_eq!(
            worker_ids,
            vec![
                "acta.attachment_reconciler",
                "acta.live_listener",
                "acta.presence_agent",
                "acta.presence_sweeper",
                "acta.webhook_dispatcher",
                "search.pgvector_embeddings.index_worker",
            ],
            "REG-5 must declare exactly these six workers for {backend:?}"
        );

        assert!(
            entries
                .iter()
                .flat_map(|entry| entry.workers.iter())
                .all(|declaration| !declaration.critical()),
            "every REG-5 worker must be critical: false for {backend:?}"
        );

        build(entries).unwrap_or_else(|errors| {
            panic!("REG-5 entries with their six workers must build for {backend:?}: {errors:?}")
        });
    }
}

/// The design's headline measured regression (§0.5): a dependency-only sort
/// puts `acta` before `search.pgvector_embeddings` (both are ready at the
/// same step once `platform`/`custos` are processed, since Modules declare
/// no `dependencies`, and `"acta" < "search.pgvector_embeddings"`
/// lexicographically). `Registry::startup_order()` merges capability edges
/// (`acta` optionally requires `search.semantic`, provided by
/// `search.pgvector_embeddings`), which reverses that and puts the provider
/// first — SHELL-OPS-6's "Custos antes que Acta" restated through the merged
/// sort, on the one pair `startup_order()`'s worker-bearing filter still
/// makes observable in real REG-5 data (`storage.filesystem`/`storage.s3`
/// and `custos` decare no worker in this PR, per spec's "computed only over
/// components that declare at least one worker", so neither appears in the
/// filtered result at all — the full, unfiltered merge is exercised by the
/// synthetic tests in `atlas_core::registry::build::tests::startup_order`).
#[test]
fn startup_order_places_search_pgvector_embeddings_before_acta() {
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        let registry = build(reg5_component_entries(backend)).unwrap_or_else(|errors| {
            panic!("REG-5 entries must build for {backend:?}: {errors:?}")
        });

        let order: Vec<&str> = registry
            .startup_order()
            .iter()
            .map(ComponentId::as_str)
            .collect();

        assert_eq!(
            order,
            vec!["search.pgvector_embeddings", "acta"],
            "REG-5's only two worker-bearing components for {backend:?} must start in this order"
        );
    }
}

/// Proves `BoundWorkers::bind` refuses at startup when the runtime
/// implementation set drifts from REG-5's six declarations (design D1, R1).
#[test]
fn bind_refuses_when_a_declared_worker_has_no_implementation() {
    use atlas_core::registry::BoundWorkers;

    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must build");

    let errors = BoundWorkers::bind(&registry, vec![]).expect_err("no implementations bound");

    assert_eq!(
        errors.len(),
        6,
        "all six declared workers must be reported unbound"
    );
    for id in [
        "acta.webhook_dispatcher",
        "acta.attachment_reconciler",
        "acta.live_listener",
        "acta.presence_sweeper",
        "acta.presence_agent",
        "search.pgvector_embeddings.index_worker",
    ] {
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, atlas_core::registry::WorkerBindError::UnboundWorker { worker } if *worker == self::worker(id)))
        );
    }
}

/// `Registry::readiness_components()` (E11-S2/PR2 design D4.2) derives the
/// mandatory readiness set from `diagnostics.readiness == true` over the
/// real REG-5 entries, for both storage backends — a non-vacuous, real-data
/// proof (spec Acceptance item 8) alongside the synthetic tests in
/// `atlas_core::registry::validated::tests`.
#[test]
fn readiness_components_returns_platform_custos_and_acta() {
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        let registry = build(reg5_component_entries(backend)).unwrap_or_else(|errors| {
            panic!("REG-5 entries must build for {backend:?}: {errors:?}")
        });

        let mandatory: Vec<String> = registry
            .readiness_components()
            .into_iter()
            .map(|id| id.as_str().to_string())
            .collect();

        assert_eq!(
            mandatory,
            vec!["platform", "custos", "acta"],
            "REG-5's readiness-mandatory set for {backend:?} must be exactly platform, custos, acta"
        );
    }
}

/// The optional-Module readiness pin (E11-S2/PR2 design R6): every optional
/// Module in REG-5 declares `diagnostics.readiness == false`. Deriving the
/// readiness-mandatory set from `diagnostics.readiness` would silently
/// admit an optional Module the moment one flips this flag — this test pins
/// today's state against that regression, revisitable in E8 rather than a
/// permanent structural guarantee.
#[test]
fn every_optional_module_declares_no_readiness() {
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        let entries = reg5_component_entries(backend);

        let modules: Vec<&ComponentEntry> = entries
            .iter()
            .filter(|entry| entry.identity.kind == ComponentKind::Module)
            .collect();
        assert!(
            !modules.is_empty(),
            "REG-5 for {backend:?} must declare at least one Module for this pin to be non-vacuous"
        );

        for module in modules {
            assert!(
                !module.diagnostics.readiness,
                "optional Module `{}` must declare diagnostics.readiness == false for {backend:?} \
                 — otherwise it would silently join the readiness-mandatory set",
                module.identity.stable_id.as_str()
            );
        }
    }
}

/// A pin, not a fix (E11-S3b design D1.2, T1.42): `acta`'s `workers`
/// declaration vec already mirrors today's spawn order exactly (dispatcher,
/// reconciler, listener, sweeper, agent), so the supervisor's intra-component
/// start order is byte-identical to `main.rs`'s original hand-spawns with no
/// `reg5.rs` change required.
#[test]
fn acta_workers_declaration_order_mirrors_todays_spawn_order() {
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        let entries = reg5_component_entries(backend);
        let acta = entries
            .iter()
            .find(|entry| entry.identity.stable_id.as_str() == "acta")
            .expect("REG-5 must declare an acta entry");

        let ids: Vec<&str> = acta
            .workers
            .iter()
            .map(|declaration| declaration.id.as_str())
            .collect();

        assert_eq!(
            ids,
            vec![
                "acta.webhook_dispatcher",
                "acta.attachment_reconciler",
                "acta.live_listener",
                "acta.presence_sweeper",
                "acta.presence_agent",
            ],
            "acta's workers declaration order for {backend:?} must mirror today's spawn order"
        );
    }
}

/// T2.23 (E11-S3b design D5, orchestrator resolution on T2.18): `custos`,
/// `acta`, `platform` and the four Modules all declare `diagnostics.doctor
/// == true` — every REG-5 entry declares a doctor in this slice.
#[test]
fn every_reg5_entry_declares_a_doctor() {
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        for entry in reg5_component_entries(backend) {
            assert!(
                entry.diagnostics.doctor,
                "{:?}'s `{}` entry must declare diagnostics.doctor == true",
                backend,
                entry.identity.stable_id.as_str()
            );
        }
    }
}

/// T2.25: `Registry::doctor_components()` is non-vacuous over real REG-5
/// data, for both storage backends — every present component names itself.
#[test]
fn doctor_components_is_non_vacuous_over_real_reg5_data() {
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        let registry = build(reg5_component_entries(backend))
            .expect("REG-5 entries must satisfy every registry::build() validator");

        let mut doctors: Vec<String> = registry
            .doctor_components()
            .iter()
            .map(|id| id.as_str().to_string())
            .collect();
        doctors.sort_unstable();

        let entries = reg5_component_entries(backend);
        let mut expected: Vec<String> = entries
            .iter()
            .map(|entry| entry.identity.stable_id.as_str().to_string())
            .collect();
        expected.sort_unstable();

        assert!(
            !doctors.is_empty(),
            "must be non-vacuous over real REG-5 data"
        );
        assert_eq!(doctors, expected);
    }
}
