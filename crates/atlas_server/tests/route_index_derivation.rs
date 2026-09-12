//! `RouteIndex::from_registry` derivation guard (E11-S7 design D1, D-S7-2,
//! D7.1, gate G4).
//!
//! Three assertions ship together, in review order (D7.1):
//!
//! 1. **The synthetic-entry probe** (anti-vacuity): a registry augmented
//!    with one fabricated component/route must yield an index entry
//!    matching exactly that component and operation — a hardcoded table
//!    passes every other assertion here and fails this one.
//! 2. **The count**: `index.len()` equals the registry's own declared route
//!    count, measured independently in this test rather than against the
//!    literal `218`, so a future registry change fails loudly.
//! 3. **The document split**: 216 of the index's operation ids appear in
//!    the composed OpenAPI document, and the 2 that do not are exactly
//!    `/openapi.json` and `/scalar` (`UNANNOTATED_ROUTES`, design D1.2).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashSet;

use atlas_core::registry::{
    Api, Authorization, Capabilities, ComponentEntry, ComponentId, ComponentKind, ContractVersion,
    Diagnostics, Experience, HttpMethod, Identity, RouteDeclaration, RoutePath, build,
};
use atlas_server::observability::route_index::RouteIndex;
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use atlas_server::routes::openapi::openapi;

/// Routes with no `#[utoipa::path]` annotation of their own — mirrors
/// `openapi_zero_drift.rs`'s exclusion list (design D1.2). Nothing else may
/// join it silently.
const UNANNOTATED_ROUTES: &[(HttpMethod, &str)] = &[
    (HttpMethod::Get, "/openapi.json"),
    (HttpMethod::Get, "/scalar"),
];

/// A synthetic `Module`-shaped component declaring exactly one route
/// (`GET /probe/only`, `operation_id: "s7_probe"`), fabricated to prove
/// `RouteIndex::from_registry` actually reads the registry rather than
/// returning a hardcoded table (design D7.1).
fn synthetic_probe_entry() -> ComponentEntry {
    ComponentEntry {
        identity: Identity {
            stable_id: ComponentId::new("s7_probe").expect("valid component id"),
            kind: ComponentKind::Module,
            contract_version: ContractVersion::new(1),
        },
        dependencies: vec![],
        capabilities: Capabilities {
            provided: vec![],
            required_mandatory: vec![],
            required_optional: vec![],
        },
        api: Api {
            namespace: None,
            routes: vec![RouteDeclaration {
                method: HttpMethod::Get,
                path: RoutePath::new("/probe/only").expect("valid route path"),
                operation_id: "s7_probe".to_string(),
                action: None,
                idempotent: false,
                is_public: false,
            }],
            dto_owner: None,
        },
        authorization: Authorization {
            resource_kinds: vec![],
            actions: vec![],
            role_definitions: vec![],
            principal_sets: vec![],
            provider: false,
        },
        diagnostics: Diagnostics {
            health: false,
            readiness: true,
            doctor: false,
        },
        experience: Experience {
            navigation_providers: vec![],
            context_providers: vec![],
        },
        persistence: None,
        config: None,
        workers: vec![],
        satellites: vec![],
    }
}

/// The live REG-5 registry plus [`synthetic_probe_entry`], built once per
/// call so every test starts from the same augmented entry set.
fn augmented_registry() -> atlas_core::registry::Registry {
    let mut entries = reg5_component_entries(StorageBackend::Filesystem);
    entries.push(synthetic_probe_entry());

    build(entries).expect("augmented REG-5 entries must satisfy every registry::build() validator")
}

/// D7.1's anti-vacuity probe: the fabricated entry's route must appear in
/// the index under its own component and operation. A hardcoded table
/// passes every other assertion in this file and fails this one.
#[test]
fn synthetic_probe_route_appears_with_its_own_component_and_operation() {
    let registry = augmented_registry();
    let index = RouteIndex::from_registry(&registry);

    let mounted = atlas_server::router_audit::mounted_path(
        &atlas_server::router_audit::v2_namespace("s7_probe"),
        "/probe/only",
    );
    let tag = index
        .get(&axum::http::Method::GET, &mounted)
        .expect("the synthetic probe route must appear in the derived index");

    assert_eq!(&*tag.component, "s7_probe");
    assert_eq!(&*tag.operation, "s7_probe");
}

/// `index.len()` equals the plain, live (non-augmented) REG-5 registry's
/// own declared route count, measured independently here rather than
/// against a literal, so a future registry change fails loudly instead of
/// silently.
#[test]
fn index_len_equals_the_registrys_own_declared_route_count() {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let declared_route_count: usize = registry
        .entries()
        .iter()
        .map(|entry| entry.api.routes.len())
        .sum();

    let index = RouteIndex::from_registry(&registry);

    assert_eq!(index.len(), declared_route_count);
    assert_eq!(
        declared_route_count, 218,
        "the registry's own declared route count has moved off the pinned 218 (design D1.2); if \
         this is an intended registry change, update this count-pin comment"
    );
}

/// 216 of the index's operation ids appear in the composed document, and
/// the 2 that do not are exactly `/openapi.json` and `/scalar`
/// (`UNANNOTATED_ROUTES`, design D1.2). A failure here means the index is
/// keyed on the wrong path form — a `mounted_path` regression — not that
/// the exclusion list needs to grow.
#[test]
fn two_sixteen_of_218_operation_ids_appear_in_the_document_and_the_rest_are_unannotated() {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");
    let index = RouteIndex::from_registry(&registry);

    let document = openapi();
    let document_operation_ids: HashSet<String> = document
        .paths
        .paths
        .values()
        .flat_map(|item| {
            [
                item.get.as_ref(),
                item.put.as_ref(),
                item.post.as_ref(),
                item.delete.as_ref(),
                item.options.as_ref(),
                item.head.as_ref(),
                item.patch.as_ref(),
                item.trace.as_ref(),
            ]
            .into_iter()
            .flatten()
        })
        .filter_map(|operation| operation.operation_id.clone())
        .collect();

    let mut present = 0usize;
    let mut absent: Vec<String> = Vec::new();

    for entry in registry.entries() {
        let namespace = atlas_server::router_audit::v2_namespace(entry.identity.stable_id.as_str());
        for route in &entry.api.routes {
            let mounted = atlas_server::router_audit::mounted_path(&namespace, route.path.as_str());
            let tag = index
                .get(&to_axum_method(route.method), &mounted)
                .unwrap_or_else(|| panic!("index must contain {mounted}"));

            assert_eq!(&*tag.operation, route.operation_id.as_str());

            if document_operation_ids.contains(route.operation_id.as_str()) {
                present += 1;
            } else {
                absent.push(format!("{} {mounted}", route.method));
            }
        }
    }

    assert_eq!(
        present, 216,
        "expected 216 index operation ids to appear in the composed document"
    );

    let mut expected_absent: Vec<String> = UNANNOTATED_ROUTES
        .iter()
        .map(|(method, path)| format!("{method} {path}"))
        .collect();
    absent.sort();
    expected_absent.sort();

    assert_eq!(
        absent, expected_absent,
        "the 2 operation ids absent from the document must be exactly UNANNOTATED_ROUTES"
    );
}

fn to_axum_method(method: HttpMethod) -> axum::http::Method {
    match method {
        HttpMethod::Get => axum::http::Method::GET,
        HttpMethod::Post => axum::http::Method::POST,
        HttpMethod::Put => axum::http::Method::PUT,
        HttpMethod::Patch => axum::http::Method::PATCH,
        HttpMethod::Delete => axum::http::Method::DELETE,
        HttpMethod::Head => axum::http::Method::HEAD,
        HttpMethod::Options => axum::http::Method::OPTIONS,
    }
}
