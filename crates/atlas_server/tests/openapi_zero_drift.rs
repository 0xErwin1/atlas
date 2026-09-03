#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! D5's registry↔document zero-drift guard, replacing `openapi_digest.rs`'s
//! byte-snapshot and `openapi_drift.rs`'s `EXPECTED_OPENAPI_PATHS` hand
//! literal with a `HashSet`-based, bidirectional, registry-derived
//! comparison (INV-SET, INV-DATA-DRIVEN).
//!
//! Scope note, updated by PR4 (T4.X): PR2's own checkpoint compared two
//! genuinely `/api`-absolute sets directly, since neither `reg5.rs`'s route
//! literals nor the `#[utoipa::path(path = "...")]` annotations had been
//! rewritten yet. PR4's literal rewrite made both populations
//! namespace-relative, and `document()` (`routes/openapi.rs`) now re-mounts
//! its merged paths under `/api` at composition time
//! (`crate::routes::openapi::prefix_document_paths`), so the document's own
//! path keys stay `/api`-absolute, byte-identical to what they were before
//! the rewrite. This test's registry side is therefore joined through
//! [`atlas_server::router_audit::mounted_path`] before comparing against the
//! document, which is the same "root-level routes stay unprefixed, every
//! other route is `/api`-joined" rule `document()` itself applies.
//!
//! Scope statement, confirmed by PR5 (T5.20): this guard runs against `/api`
//! only, never `/api/v2` — stated explicitly here rather than left ambiguous
//! (`v2-e3-s4`'s cross-cutting-audit requirement forbids an unstated scope).
//! `document()`'s own path keys are `/api`-absolute by construction
//! (`prefix_document_paths(doc, "/api")`, unchanged by PR5); PR5's own scope
//! excludes any change to the composed OpenAPI document or its mount
//! prefix, so there is no `/api/v2`-prefixed rendering of this document for
//! this guard to compare against yet. Re-mounting the document's own path
//! keys under `/api/v2` (and re-scoping this guard to match) is a future
//! slice's job, not this PR's.
//!
//! PR6 update: `GET /api/workspaces/{ws}/events` (`routes::events`) is now
//! `#[utoipa::path]`-annotated and registered in `acta`'s fragment, so it is
//! no longer a member of [`UNANNOTATED_ROUTES`] — the unannotated list has
//! shrunk to exactly `/openapi.json` and `/scalar`.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use atlas_core::registry::{ComponentId, ComponentKind, HttpMethod, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use atlas_server::routes::openapi::openapi;

/// Routes with no `#[utoipa::path]` annotation of their own — a
/// self-checking, justified exclusion this test allows (design D5); nothing
/// else may join it silently.
///
/// - `/openapi.json`/`/scalar` are ordinary `platform`-owned
///   `RouteDeclaration` entries (`v2-e3-s4` PR1, D3): the document cannot
///   describe its own serving endpoint or the Scalar UI it mounts.
const UNANNOTATED_ROUTES: &[(HttpMethod, &str)] = &[
    (HttpMethod::Get, "/openapi.json"),
    (HttpMethod::Get, "/scalar"),
];

fn registry_route_set() -> HashSet<(HttpMethod, String)> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let mut routes = HashSet::new();
    for component_id in ["platform", "custos", "acta"] {
        let entry = registry
            .get(&ComponentId::new(component_id).expect("valid component id"))
            .unwrap_or_else(|| panic!("{component_id} must be a registered REG-5 component"));

        for route in &entry.api.routes {
            let mounted = atlas_server::router_audit::mounted_path("/api", route.path.as_str());
            routes.insert((route.method, mounted));
        }
    }
    routes
}

fn document_operation_set() -> HashSet<(HttpMethod, String)> {
    let document = openapi();

    let mut operations = HashSet::new();
    for (path, item) in &document.paths.paths {
        for (method, present) in [
            (HttpMethod::Get, item.get.is_some()),
            (HttpMethod::Put, item.put.is_some()),
            (HttpMethod::Post, item.post.is_some()),
            (HttpMethod::Delete, item.delete.is_some()),
            (HttpMethod::Options, item.options.is_some()),
            (HttpMethod::Head, item.head.is_some()),
            (HttpMethod::Patch, item.patch.is_some()),
        ] {
            if present {
                operations.insert((method, path.clone()));
            }
        }
    }
    operations
}

/// T2.12/T2.13 (D5, INV-SET): every registry-declared route has exactly one
/// composed-document operation, and vice versa — both directions, naming
/// the offending `(method, path)` on failure, modulo the two self-checking
/// exclusions above.
#[test]
fn every_registry_route_has_exactly_one_document_operation_and_vice_versa() {
    let mut registry_set = registry_route_set();
    for (method, path) in UNANNOTATED_ROUTES {
        let removed = registry_set.remove(&(*method, (*path).to_string()));
        assert!(
            removed,
            "{method:?} {path} is listed in UNANNOTATED_ROUTES but is not a registry route at \
             all — the exclusion list itself has drifted"
        );
    }

    let document_set = document_operation_set();

    let missing_from_document: Vec<_> = registry_set.difference(&document_set).collect();
    let missing_from_registry: Vec<_> = document_set.difference(&registry_set).collect();

    assert!(
        missing_from_document.is_empty(),
        "registry-declared routes with no composed-document operation: {missing_from_document:?}"
    );
    assert!(
        missing_from_registry.is_empty(),
        "composed-document operations with no registry-declared route: {missing_from_registry:?}"
    );
}

/// Every REG-5 Module-kind entry, deduplicated by `stable_id`. A Module
/// entry declares no HTTP surface at all, so its facts are read straight out
/// of the registry rather than out of `document()` (D4: `document()` never
/// reads the registry, and this file's other test already proves that fact
/// independently).
///
/// `reg5_component_entries` takes a `StorageBackend`, and `build()` rejects
/// two mandatory `storage.blob` providers in the same call
/// (`MandatoryCapabilityAmbiguous`) — `storage.filesystem` and `storage.s3`
/// can therefore never appear together in one registry build. Building once
/// per backend and taking the union by `stable_id` is the only way to see
/// all four Module entries (`storage.filesystem`, `storage.s3`,
/// `search.postgres_fts`, `search.pgvector_embeddings`) without silently
/// dropping one of the two mutually exclusive storage backends.
fn module_stable_ids() -> HashSet<String> {
    let mut modules = HashSet::new();
    for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
        let registry = build(reg5_component_entries(backend))
            .expect("REG-5 entries must satisfy every registry::build() validator");
        for entry in registry.entries() {
            if entry.identity.kind == ComponentKind::Module {
                modules.insert(entry.identity.stable_id.as_str().to_string());
            }
        }
    }
    modules
}

/// Every `utoipa::openapi::path::Operation` on `path_item`, across all
/// eight HTTP methods utoipa models as discrete optional fields.
fn document_operations(
    path_item: &utoipa::openapi::path::PathItem,
) -> impl Iterator<Item = &utoipa::openapi::path::Operation> {
    [
        path_item.get.as_ref(),
        path_item.put.as_ref(),
        path_item.post.as_ref(),
        path_item.delete.as_ref(),
        path_item.options.as_ref(),
        path_item.head.as_ref(),
        path_item.patch.as_ref(),
        path_item.trace.as_ref(),
    ]
    .into_iter()
    .flatten()
}

/// The single owning `stable_id` `stamp_component_ownership` wrote onto
/// `operation`'s `x-atlas-component` extension — the authoritative ownership
/// signal. An operation's `tags` list also carries the same `stable_id`
/// (`stamp_component_ownership` pushes both together), but `tags` also
/// carries unrelated feature-area tags (`"audit"`, `"workspaces"`, ...), so
/// it cannot itself be treated as a set of owners.
fn operation_owner(operation: &utoipa::openapi::path::Operation) -> Option<&str> {
    operation
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("x-atlas-component"))
        .and_then(|value| value.as_str())
}

/// Every `stable_id` that owns at least one operation in the composed
/// document, read from each operation's `x-atlas-component` extension.
fn stamped_component_ids(document: &utoipa::openapi::OpenApi) -> HashSet<String> {
    document
        .paths
        .paths
        .values()
        .flat_map(document_operations)
        .filter_map(operation_owner)
        .map(str::to_string)
        .collect()
}

/// T2.16/T2.17 (D5, data-driven over REG-5, INV-SET): every Module-kind
/// registry entry declares zero routes, and no operation in the composed
/// document is ever stamped with a Module's `stable_id` — checked both as a
/// per-module membership fact and, bidirectionally, as a whole-document
/// INV-SET: the set of `stable_id`s stamped anywhere in the document equals
/// exactly the set of non-Module (`Product`/`PlatformService`) `stable_id`s
/// the registry declares.
///
/// Replaces the former `absent_search_pgvector_module_contributes_nothing_to_the_document`,
/// which compared two serializations of the same argument-free `openapi()`
/// call and discarded the filtered `build(...)` result — a comparison that
/// could never fail regardless of what the registry declared.
#[test]
fn no_module_kind_entry_declares_a_route_or_owns_a_document_operation() {
    let modules = module_stable_ids();
    assert_eq!(
        modules.len(),
        4,
        "expected exactly the four REG-5 Module entries (storage.filesystem, storage.s3, \
         search.postgres_fts, search.pgvector_embeddings); found {modules:?} — this test's own \
         fixture has drifted"
    );

    let filesystem_registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");
    let s3_registry = build(reg5_component_entries(StorageBackend::S3))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let mut routeful_modules = Vec::new();
    for registry in [&filesystem_registry, &s3_registry] {
        for entry in registry.entries() {
            if entry.identity.kind == ComponentKind::Module && !entry.api.routes.is_empty() {
                routeful_modules.push(entry.identity.stable_id.as_str().to_string());
            }
        }
    }
    assert!(
        routeful_modules.is_empty(),
        "Module-kind registry entries must declare zero routes: {routeful_modules:?}"
    );

    let document = openapi();

    let mut module_leaks = Vec::new();
    for path_item in document.paths.paths.values() {
        for operation in document_operations(path_item) {
            let tag_leak = operation
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| modules.contains(tag)));
            let extension_leak =
                operation_owner(operation).is_some_and(|owner| modules.contains(owner));
            if tag_leak || extension_leak {
                module_leaks.push(operation.operation_id.clone());
            }
        }
    }
    assert!(
        module_leaks.is_empty(),
        "operation(s) carrying a Module-kind stable_id as a tag or x-atlas-component \
         extension: {module_leaks:?}"
    );

    let stamped = stamped_component_ids(&document);

    let non_module_ids: HashSet<String> = filesystem_registry
        .entries()
        .iter()
        .filter(|entry| entry.identity.kind != ComponentKind::Module)
        .map(|entry| entry.identity.stable_id.as_str().to_string())
        .collect();

    let stamped_with_no_owning_entry: Vec<_> = stamped.difference(&non_module_ids).collect();
    let owning_entries_never_stamped: Vec<_> = non_module_ids.difference(&stamped).collect();

    assert!(
        stamped_with_no_owning_entry.is_empty(),
        "stable_id(s) stamped in the document with no matching non-Module registry entry: \
         {stamped_with_no_owning_entry:?}"
    );
    assert!(
        owning_entries_never_stamped.is_empty(),
        "non-Module registry entries never stamped as the owner of any document operation: \
         {owning_entries_never_stamped:?}"
    );
}

/// R3 (schema-set coverage): with the old drift list and digest fixture
/// deleted, nothing independently guarded `components.schemas` — a DTO
/// dropped from one of `routes/{platform,custos,acta}.rs`'s hand-written
/// `components(schemas(...))` lists would leave a dangling `$ref` unnoticed.
/// These two tests restore that coverage from two independent angles: (a)
/// every `$ref` the document actually uses resolves to a declared schema,
/// and (b) every declared schema is backed by a real `#[derive(ToSchema)]`
/// type, so an unused/renamed DTO can't silently sit in the list either.
mod schema_set_coverage {
    use super::*;

    /// Every `#/components/schemas/<Name>` string found anywhere in `value`,
    /// found by walking the whole serialized document rather than assuming
    /// `$ref`s only ever appear in one shape of container (property, array
    /// item, `allOf` branch, ...).
    fn collect_schema_refs(value: &serde_json::Value, refs: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, entry) in map {
                    if key == "$ref"
                        && let serde_json::Value::String(target) = entry
                    {
                        refs.push(target.clone());
                    }
                    collect_schema_refs(entry, refs);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_schema_refs(item, refs);
                }
            }
            _ => {}
        }
    }

    /// The dangling-ref detector both the real test and its teeth-proof
    /// below call: every `$ref` in `document` that names a schema absent
    /// from `document`'s own `components.schemas`.
    fn dangling_schema_refs(document: &serde_json::Value) -> Vec<String> {
        let schemas = document
            .pointer("/components/schemas")
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();

        let mut refs = Vec::new();
        collect_schema_refs(document, &mut refs);

        refs.into_iter()
            .filter_map(|target| {
                target
                    .strip_prefix("#/components/schemas/")
                    .map(str::to_string)
            })
            .filter(|name| !schemas.contains_key(name))
            .collect()
    }

    /// (a) Every `$ref` in the composed document resolves to a declared
    /// `components.schemas` entry — the dangling-ref detector proper.
    #[test]
    fn every_schema_ref_in_the_document_resolves() {
        let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");

        let offenders = dangling_schema_refs(&document);
        assert!(
            offenders.is_empty(),
            "dangling $ref(s) with no matching components.schemas entry: {offenders:?}"
        );
    }

    /// Proves `dangling_schema_refs` has teeth: a fabricated `$ref` to a
    /// schema name that does not exist must be caught, not silently ignored.
    #[test]
    fn dangling_ref_detector_rejects_a_fabricated_dangling_ref() {
        let mut document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");
        document
            .pointer_mut("/components/schemas")
            .and_then(serde_json::Value::as_object_mut)
            .expect("document must declare components.schemas")
            .insert(
                "__FabricatedProbe__".to_string(),
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "victim": { "$ref": "#/components/schemas/__ThisSchemaDoesNotExist__" }
                    }
                }),
            );

        let offenders = dangling_schema_refs(&document);
        assert_eq!(
            offenders,
            vec!["__ThisSchemaDoesNotExist__".to_string()],
            "the detector must catch a fabricated dangling $ref, proving it has teeth"
        );
    }

    /// A type name discovered by scanning `atlas_api`'s source for a
    /// `#[derive(ToSchema)]` (bare or `cfg_attr`-gated) directly above a
    /// `struct`/`enum` declaration.
    #[derive(Debug, Clone)]
    struct ToSchemaType {
        name: String,
    }

    /// Walks every `.rs` file under `crates/atlas_api/src` (recursively:
    /// `dtos/` is one level deep, and `lib.rs`/`pagination.rs`/`problem.rs`
    /// sit directly under `src/`) and collects the name of every
    /// `struct`/`enum` immediately preceded by a `derive(...)` that includes
    /// `ToSchema` — handling both `#[derive(ToSchema)]` and
    /// `#[cfg_attr(feature = "openapi", derive(ToSchema))]`, this codebase's
    /// only two spellings.
    ///
    /// No `ToSchema` derive lives under `crates/atlas_server/src` today (a
    /// prerequisite this scan does not silently assume: `no_to_schema_derives_outside_atlas_api`
    /// below re-checks it on every run).
    fn discover_to_schema_types() -> Vec<ToSchemaType> {
        let struct_re = regex::Regex::new(r"^\s*(?:pub\s+)?struct\s+(\w+)").expect("valid regex");
        let enum_re = regex::Regex::new(r"^\s*(?:pub\s+)?enum\s+(\w+)").expect("valid regex");

        let mut types = Vec::new();
        for path in rs_files_under(&atlas_api_src_dir()) {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("must be able to read {}: {e}", path.display()));

            let mut pending = false;
            for line in source.lines() {
                if line.contains("derive(") && line.contains("ToSchema") {
                    pending = true;
                    continue;
                }
                if !pending {
                    continue;
                }
                if let Some(caps) = struct_re.captures(line).or_else(|| enum_re.captures(line)) {
                    types.push(ToSchemaType {
                        name: caps[1].to_string(),
                    });
                    pending = false;
                }
            }
        }
        types
    }

    fn atlas_api_src_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../atlas_api/src")
    }

    fn atlas_server_src_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// Recursively collects every `.rs` file under `dir`.
    fn rs_files_under(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut files = Vec::new();
        let entries = fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("must be able to read {}: {e}", dir.display()));
        for entry in entries {
            let entry = entry.expect("readable dir entry");
            let path = entry.path();
            if path.is_dir() {
                files.extend(rs_files_under(&path));
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
        files
    }

    /// A `components.schemas` name is either a real `ToSchema` type
    /// verbatim, or a utoipa generic instantiation (`Page_AuditEntryDto`,
    /// `Vec_FooDto`, ...) whose base (`Page`, `Vec`) and inner name
    /// (`AuditEntryDto`, `FooDto`) are both known `ToSchema` types.
    fn is_known_schema_type(name: &str, known: &std::collections::HashSet<&str>) -> bool {
        if known.contains(name) {
            return true;
        }
        if let Some((base, inner)) = name.split_once('_') {
            return known.contains(base) && known.contains(inner);
        }
        false
    }

    /// (b) An independent inventory scan, mirroring
    /// `crates/atlas_api/tests/rfc3339_conformance.rs`'s source-level
    /// approach: every name in `components.schemas` must be a real
    /// `#[derive(ToSchema)]` type (or a generic instantiation of one), never
    /// a name that only ever existed in a hand-written `components(schemas(...))`
    /// list.
    #[test]
    fn every_document_schema_is_a_known_to_schema_type() {
        let types = discover_to_schema_types();
        assert!(
            types.len() >= 100,
            "expected at least 100 #[derive(ToSchema)] types under atlas_api/src (a sharp drop \
             suggests the scan regressed); found {}",
            types.len()
        );

        let known: std::collections::HashSet<&str> =
            types.iter().map(|t| t.name.as_str()).collect();

        let document = serde_json::to_value(openapi()).expect("serialize OpenAPI document");
        let schemas = document
            .pointer("/components/schemas")
            .and_then(serde_json::Value::as_object)
            .expect("document must declare components.schemas");

        let offenders: Vec<&String> = schemas
            .keys()
            .filter(|name| !is_known_schema_type(name, &known))
            .collect();

        assert!(
            offenders.is_empty(),
            "components.schemas name(s) with no matching #[derive(ToSchema)] type (or generic \
             instantiation of one): {offenders:?}"
        );
    }

    /// This scan's own prerequisite: no `ToSchema` derive lives under
    /// `atlas_server/src` today. If one is ever added there,
    /// `discover_to_schema_types` must be extended to scan it too, or this
    /// coverage test would silently miss it.
    #[test]
    fn no_to_schema_derives_outside_atlas_api() {
        let mut offenders = Vec::new();
        for path in rs_files_under(&atlas_server_src_dir()) {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("must be able to read {}: {e}", path.display()));
            if source.contains("derive(") && source.contains("ToSchema") {
                offenders.push(path);
            }
        }
        assert!(
            offenders.is_empty(),
            "ToSchema derive(s) found under atlas_server/src, but every_document_schema_is_a_known_to_schema_type \
             only scans atlas_api/src: {offenders:?}"
        );
    }

    /// Proves `is_known_schema_type` has teeth: a fabricated schema name
    /// with no backing `ToSchema` type must be rejected.
    #[test]
    fn schema_type_detector_rejects_a_fabricated_schema_name() {
        let known: std::collections::HashSet<&str> = ["RealDto"].into_iter().collect();
        assert!(is_known_schema_type("RealDto", &known));
        assert!(
            !is_known_schema_type("FabricatedDtoNotInInventory", &known),
            "the detector must reject a schema name with no backing ToSchema type, proving it \
             has teeth"
        );
    }
}
