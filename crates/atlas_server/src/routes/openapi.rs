use axum::{Json, response::IntoResponse};
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable as _};

/// Carries only `info(...)`: zero `paths`/`schemas` of its own, so merging
/// the three component fragments onto it (`document()`) never duplicates
/// anything and `info` is set exactly once (D4).
#[derive(OpenApi)]
#[openapi(info(
    title = "Atlas API",
    version = env!("CARGO_PKG_VERSION"),
    description = "Atlas knowledge and project-management platform REST API"
))]
struct ApiDoc;

/// Adds `tag = <stable_id>` and a matching `x-atlas-component` extension to
/// every operation in `doc` (D4/T2.6). Called exactly once per fragment
/// module, from that module's own `openapi()` function, with that module's
/// single `stable_id` constant — never a per-operation hand-typed literal.
pub(crate) fn stamp_component_ownership(
    mut doc: utoipa::openapi::OpenApi,
    stable_id: &str,
) -> utoipa::openapi::OpenApi {
    for path_item in doc.paths.paths.values_mut() {
        for operation in operations_mut(path_item) {
            operation
                .tags
                .get_or_insert_with(Vec::new)
                .push(stable_id.to_string());

            let extensions = operation
                .extensions
                .get_or_insert_with(utoipa::openapi::extensions::Extensions::default);
            extensions.insert(
                "x-atlas-component".to_string(),
                serde_json::Value::String(stable_id.to_string()),
            );
        }
    }
    doc
}

/// Every [`utoipa::openapi::path::Operation`] present on a
/// [`utoipa::openapi::path::PathItem`], across all eight HTTP methods utoipa
/// models as discrete optional fields — there is no method-keyed map to
/// iterate directly.
fn operations_mut(
    path_item: &mut utoipa::openapi::path::PathItem,
) -> impl Iterator<Item = &mut utoipa::openapi::path::Operation> {
    [
        path_item.get.as_mut(),
        path_item.put.as_mut(),
        path_item.post.as_mut(),
        path_item.delete.as_mut(),
        path_item.options.as_mut(),
        path_item.head.as_mut(),
        path_item.patch.as_mut(),
        path_item.trace.as_mut(),
    ]
    .into_iter()
    .flatten()
}

/// Rewrites every path key in `doc.paths.paths` to the concrete path it is
/// actually mounted at (`v2-e3-s4` D2/T4.X): each fragment's own
/// `#[utoipa::path(path = "...")]` annotations were rewritten to
/// namespace-relative form in the same script that rewrote the registry's
/// literals, so the document must re-apply the mount prefix at composition
/// time to keep serving the exact same V1 (`/api`-absolute) paths it served
/// before that rewrite — [`crate::router_audit::mounted_path`] is the single
/// place that decision is made, so a root-level route
/// (`crate::router_audit::ROOT_LEVEL_PATHS`) never accidentally gains a
/// prefix here.
fn prefix_document_paths(
    mut doc: utoipa::openapi::OpenApi,
    namespace: &str,
) -> utoipa::openapi::OpenApi {
    let old_paths = std::mem::take(&mut doc.paths.paths);
    for (path, item) in old_paths {
        let mounted = crate::router_audit::mounted_path(namespace, &path);
        doc.paths.paths.insert(mounted, item);
    }
    doc
}

/// Composes the full OpenAPI document from each component's own fragment
/// (D4): `ApiDoc` contributes only `info(...)`, then `platform`'s,
/// `custos`'s, and `acta`'s fragments are merged in, in this fixed order.
/// No other component can structurally contribute — this function only
/// ever calls these three named fragment constructors (T2.18). The merged
/// result's paths are then re-mounted under `/api` (`prefix_document_paths`,
/// T4.X), since every fragment's own paths are namespace-relative as of
/// this slice's literal rewrite.
pub(crate) fn document() -> utoipa::openapi::OpenApi {
    let mut doc = ApiDoc::openapi();
    doc.merge(crate::routes::platform::openapi());
    doc.merge(crate::routes::custos::openapi());
    doc.merge(crate::routes::acta::openapi());
    prefix_document_paths(doc, "/api")
}

pub(crate) async fn openapi_json() -> impl IntoResponse {
    Json(document())
}

pub(crate) fn scalar_router<S>() -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    axum::Router::from(Scalar::with_url("/scalar", document()))
}

/// Expose the assembled `OpenApi` document for test assertions.
pub fn openapi() -> utoipa::openapi::OpenApi {
    document()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use atlas_core::registry::{ComponentId, build};

    use super::*;
    use crate::reg5::{StorageBackend, reg5_component_entries};

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn fragment_schemas(
        doc: &utoipa::openapi::OpenApi,
    ) -> std::collections::BTreeMap<String, utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>>
    {
        doc.components
            .as_ref()
            .map(|components| components.schemas.clone())
            .unwrap_or_default()
    }

    type SchemaMap =
        std::collections::BTreeMap<String, utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>>;

    /// Every schema name shared between two of the given `(fragment_name,
    /// schemas)` pairs whose SHAPE disagrees — the exact hazard
    /// `OpenApi::merge`'s first-write-wins dedup would otherwise silently
    /// paper over (D4's own grounding). Both the real detector
    /// (`shared_schema_names_never_disagree_on_shape`) and its adversarial
    /// teeth test (`shape_mismatch_detection_has_teeth`) call this exact
    /// function, so a bug in its comparison loop cannot hide behind either
    /// caller.
    fn shape_mismatches(fragments: &[(&str, &SchemaMap)]) -> Vec<(String, (String, String))> {
        let mut mismatches = Vec::new();
        for i in 0..fragments.len() {
            for j in (i + 1)..fragments.len() {
                let (left_name, left) = *fragments.get(i).expect("i is within the fragment list");
                let (right_name, right) = *fragments.get(j).expect("j is within the fragment list");
                for (name, left_schema) in left.iter() {
                    if let Some(right_schema) = right.get(name)
                        && left_schema != right_schema
                    {
                        mismatches.push((
                            name.clone(),
                            (left_name.to_string(), right_name.to_string()),
                        ));
                    }
                }
            }
        }
        mismatches
    }

    /// T2.1/T2.3 (D4, schema-collision): the three fragments' own
    /// `components.schemas` maps, built independently before merge, never
    /// disagree about the SHAPE of a shared name. A name legitimately
    /// appears in more than one fragment's built map when a schema owned by
    /// one component (e.g. `acta`'s `ActorDto`) is embedded as a field of a
    /// schema owned by another (`custos`'s `AuditEntryDto`) — utoipa
    /// transitively re-discovers the embedded type inside every fragment
    /// that references it, which is not a collision as long as both
    /// fragments describe the exact same shape for it (the same canonical
    /// Rust type, referenced from two call sites). A REAL collision — two
    /// different types sharing one schema name — is the case this test
    /// exists to catch: it would silently lose one type's shape to
    /// `OpenApi::merge`'s first-write-wins dedup (D4's own grounding).
    #[test]
    fn shared_schema_names_never_disagree_on_shape() {
        let platform = fragment_schemas(&crate::routes::platform::openapi());
        let custos = fragment_schemas(&crate::routes::custos::openapi());
        let acta = fragment_schemas(&crate::routes::acta::openapi());

        let mismatches = shape_mismatches(&[
            ("platform", &platform),
            ("custos", &custos),
            ("acta", &acta),
        ]);

        assert!(
            mismatches.is_empty(),
            "schema name(s) shared between fragments with DIFFERING shapes — a genuine \
             collision, not a legitimate shared reference: {mismatches:?}"
        );
    }

    /// T2.4 (adversarial proof): `shape_mismatches` must actually flag a
    /// genuine cross-fragment collision — a same-name schema with two
    /// different shapes fabricated across two stand-in fragments — not just
    /// prove that `RefOr<Schema>`'s `PartialEq` distinguishes two values in
    /// isolation.
    #[test]
    fn shape_mismatch_detection_has_teeth() {
        use utoipa::openapi::schema::{Object, Schema, Type};

        let mut left: SchemaMap = std::collections::BTreeMap::new();
        left.insert(
            "SharedName".to_string(),
            utoipa::openapi::RefOr::T(Schema::Object(
                Object::builder().schema_type(Type::String).build(),
            )),
        );

        let mut right: SchemaMap = std::collections::BTreeMap::new();
        right.insert(
            "SharedName".to_string(),
            utoipa::openapi::RefOr::T(Schema::Object(
                Object::builder().schema_type(Type::Integer).build(),
            )),
        );

        let mismatches = shape_mismatches(&[("left-fragment", &left), ("right-fragment", &right)]);

        assert_eq!(
            mismatches,
            vec![(
                "SharedName".to_string(),
                ("left-fragment".to_string(), "right-fragment".to_string())
            )],
            "shape_mismatches must flag a genuine same-name/different-shape collision, naming it"
        );
    }

    /// T2.14/T2.15 (D5, schema-presence): every schema name in `document()`'s
    /// `components.schemas` traces back to at least one fragment's own set,
    /// and every fragment-declared schema survives into the merged document
    /// — both directions. "Exactly one" owner does not hold in general (see
    /// the collision test's doc comment on legitimate cross-fragment
    /// references), so this only asserts reachability, not exclusivity.
    #[test]
    fn every_document_schema_traces_back_to_a_fragment_and_vice_versa() {
        let platform: HashSet<String> = fragment_schemas(&crate::routes::platform::openapi())
            .into_keys()
            .collect();
        let custos: HashSet<String> = fragment_schemas(&crate::routes::custos::openapi())
            .into_keys()
            .collect();
        let acta: HashSet<String> = fragment_schemas(&crate::routes::acta::openapi())
            .into_keys()
            .collect();
        let fragment_union: HashSet<String> = platform
            .union(&custos)
            .cloned()
            .collect::<HashSet<_>>()
            .union(&acta)
            .cloned()
            .collect();

        let document_schemas: HashSet<String> = fragment_schemas(&document()).into_keys().collect();

        let not_owned: Vec<_> = document_schemas.difference(&fragment_union).collect();
        assert!(
            not_owned.is_empty(),
            "document schemas with no owning fragment: {not_owned:?}"
        );

        let orphaned: Vec<_> = fragment_union.difference(&document_schemas).collect();
        assert!(
            orphaned.is_empty(),
            "fragment-owned schemas absent from the merged document: {orphaned:?}"
        );
    }

    /// Every `(method, path, operation)` present in `document`'s
    /// `paths`, method-aware — the same eight-field enumeration
    /// [`operations`]/[`operations_mut`] flatten, but keeping the concrete
    /// [`atlas_core::registry::HttpMethod`] so it can be compared against a
    /// registry-declared route.
    fn document_operations_by_route(
        document: &utoipa::openapi::OpenApi,
    ) -> Vec<(
        atlas_core::registry::HttpMethod,
        String,
        &utoipa::openapi::path::Operation,
    )> {
        use atlas_core::registry::HttpMethod as Method;

        let mut out = Vec::new();
        for (path, item) in &document.paths.paths {
            for (method, operation) in [
                (Method::Get, item.get.as_ref()),
                (Method::Put, item.put.as_ref()),
                (Method::Post, item.post.as_ref()),
                (Method::Delete, item.delete.as_ref()),
                (Method::Options, item.options.as_ref()),
                (Method::Head, item.head.as_ref()),
                (Method::Patch, item.patch.as_ref()),
            ] {
                if let Some(operation) = operation {
                    out.push((method, path.clone(), operation));
                }
            }
        }
        out
    }

    /// The result of comparing `document`'s stamped operations against
    /// `owner_by_route` — the registry's own, independently-declared
    /// per-component route membership, never the tag/extension an
    /// operation's own fragment just stamped onto it. Both the real
    /// ownership test (`every_operation_is_tagged_with_its_owning_component`)
    /// and its adversarial teeth test
    /// (`ownership_diff_detects_a_mis_owned_route`) call this exact
    /// function, so a bug in its comparison cannot hide behind either
    /// caller.
    #[derive(Debug, Default)]
    struct OwnershipDiff {
        /// A registry-declared route with no matching document operation.
        missing_from_document: Vec<(atlas_core::registry::HttpMethod, String)>,
        /// A document operation stamped with a different `stable_id` than
        /// the registry says actually owns its `(method, path)`, as
        /// `(method, path, expected_owner, actual_owner)`.
        wrong_owner: Vec<(atlas_core::registry::HttpMethod, String, String, String)>,
        /// A document operation whose `(method, path)` has no
        /// registry-declared owner at all.
        unowned_in_document: Vec<(atlas_core::registry::HttpMethod, String)>,
    }

    fn ownership_diff(
        document: &utoipa::openapi::OpenApi,
        owner_by_route: &std::collections::HashMap<
            (atlas_core::registry::HttpMethod, String),
            &str,
        >,
    ) -> OwnershipDiff {
        let document_operations = document_operations_by_route(document);
        let document_routes: HashSet<(atlas_core::registry::HttpMethod, String)> =
            document_operations
                .iter()
                .map(|(method, path, _)| (*method, path.clone()))
                .collect();

        let mut diff = OwnershipDiff {
            missing_from_document: owner_by_route
                .keys()
                .filter(|route| !document_routes.contains(route))
                .cloned()
                .collect(),
            ..Default::default()
        };

        for (method, path, operation) in document_operations {
            let Some(&expected_owner) = owner_by_route.get(&(method, path.clone())) else {
                diff.unowned_in_document.push((method, path));
                continue;
            };

            let tag_matches = operation
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| tag == expected_owner));
            let extension_owner = operation
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.get("x-atlas-component"))
                .and_then(|value| value.as_str());

            if !tag_matches || extension_owner != Some(expected_owner) {
                diff.wrong_owner.push((
                    method,
                    path,
                    expected_owner.to_string(),
                    extension_owner.unwrap_or("<none>").to_string(),
                ));
            }
        }

        diff
    }

    /// T2.7/T2.8 (D4/T2.6, INV-SET at per-route granularity): every
    /// composed-document operation's `(method, path)` is looked up in the
    /// live registry (`reg5_component_entries` → the `ComponentEntry` whose
    /// `api.routes` contains that pair), never against the `stable_id` its
    /// own fragment just stamped onto it — a route mechanically
    /// miscategorized into the wrong fragment's `paths(...)` list would
    /// have failed this test before it could ever pass by construction.
    #[test]
    fn every_operation_is_tagged_with_its_owning_component() {
        let registry = build(reg5_component_entries(StorageBackend::Filesystem))
            .expect("REG-5 entries must satisfy every registry::build() validator");

        let mut owner_by_route = std::collections::HashMap::new();
        for (component_id, stable_id) in [
            ("platform", crate::routes::platform::OPENAPI_STABLE_ID),
            ("custos", crate::routes::custos::OPENAPI_STABLE_ID),
            ("acta", crate::routes::acta::OPENAPI_STABLE_ID),
        ] {
            let entry = registry
                .get(&component(component_id))
                .unwrap_or_else(|| panic!("{component_id} must be a registered REG-5 component"));
            for route in &entry.api.routes {
                let mounted = crate::router_audit::mounted_path("/api", route.path.as_str());
                owner_by_route.insert((route.method, mounted), stable_id);
            }
        }

        // Routes with no `#[utoipa::path]` annotation of their own (mirrors
        // `openapi_zero_drift.rs`'s `UNANNOTATED_ROUTES`): they never
        // produce a document operation, so ownership cannot be checked
        // against them here — their presence is `openapi_zero_drift.rs`'s
        // concern, not this test's.
        for (method, path) in [
            (atlas_core::registry::HttpMethod::Get, "/openapi.json"),
            (atlas_core::registry::HttpMethod::Get, "/scalar"),
            (
                atlas_core::registry::HttpMethod::Get,
                "/api/workspaces/{ws}/events",
            ),
        ] {
            owner_by_route.remove(&(method, path.to_string()));
        }

        let diff = ownership_diff(&document(), &owner_by_route);

        assert!(
            diff.missing_from_document.is_empty(),
            "registry-declared routes with no composed-document operation: {:?}",
            diff.missing_from_document
        );
        assert!(
            diff.wrong_owner.is_empty(),
            "operations stamped with the wrong owning component (method, path, expected, \
             actual): {:?}",
            diff.wrong_owner
        );
        assert!(
            diff.unowned_in_document.is_empty(),
            "stamped operations with no registry-declared owner: {:?}",
            diff.unowned_in_document
        );
    }

    /// T2.8 adversarial proof: `ownership_diff` must actually catch a route
    /// mechanically miscategorized into the wrong fragment's `paths(...)`
    /// list. Builds a single fabricated operation, stamps it as
    /// `custos`-owned via the real [`stamp_component_ownership`], but tells
    /// `ownership_diff` the registry says `acta` actually owns it — the
    /// exact failure mode `every_operation_is_tagged_with_its_owning_component`
    /// exists to catch and its former circular version could not.
    #[test]
    fn ownership_diff_detects_a_mis_owned_route() {
        use utoipa::openapi::path::{HttpMethod as UtoipaMethod, Operation, PathItem};
        use utoipa::openapi::{Info, OpenApi, Paths};

        let mut paths = Paths::new();
        paths.paths.insert(
            "/acta/only/mine".to_string(),
            PathItem::new(UtoipaMethod::Get, Operation::new()),
        );
        let fabricated =
            stamp_component_ownership(OpenApi::new(Info::new("fixture", "0.0.0"), paths), "custos");

        let mut owner_by_route = std::collections::HashMap::new();
        owner_by_route.insert(
            (
                atlas_core::registry::HttpMethod::Get,
                "/acta/only/mine".to_string(),
            ),
            "acta",
        );

        let diff = ownership_diff(&fabricated, &owner_by_route);

        assert!(diff.missing_from_document.is_empty());
        assert!(diff.unowned_in_document.is_empty());
        assert_eq!(
            diff.wrong_owner,
            vec![(
                atlas_core::registry::HttpMethod::Get,
                "/acta/only/mine".to_string(),
                "acta".to_string(),
                "custos".to_string(),
            )],
            "ownership_diff must name the mis-owned (method, path) and its expected/actual owner"
        );
    }

    /// T2.33: confirms each fragment module's single `OPENAPI_STABLE_ID`
    /// literal actually matches the registry's own `stable_id` for that
    /// component — the one hand-typed literal per module is checked against
    /// ground truth, not merely trusted by construction.
    #[test]
    fn fragment_stable_id_constants_match_the_registry() {
        let registry = build(reg5_component_entries(StorageBackend::Filesystem))
            .expect("REG-5 entries must satisfy every registry::build() validator");

        for (module_stable_id, component_id) in [
            (crate::routes::platform::OPENAPI_STABLE_ID, "platform"),
            (crate::routes::custos::OPENAPI_STABLE_ID, "custos"),
            (crate::routes::acta::OPENAPI_STABLE_ID, "acta"),
        ] {
            let entry = registry
                .get(&component(component_id))
                .unwrap_or_else(|| panic!("{component_id} must be a registered REG-5 component"));
            assert_eq!(module_stable_id, entry.identity.stable_id.as_str());
        }
    }

    /// T2.24/T2.25 (D6): `ConflictProblemDto`'s schema in the composed
    /// document is `allOf: [ProblemDetails, <extra fields>]`, not a flat
    /// object — the one deliberate schema-shape change in this slice.
    #[test]
    fn conflict_problem_dto_schema_is_all_of() {
        let doc = serde_json::to_value(document()).expect("serialize OpenAPI document");
        let schema = doc
            .pointer("/components/schemas/ConflictProblemDto")
            .expect("ConflictProblemDto must be a documented schema");

        let all_of = schema
            .get("allOf")
            .and_then(|value| value.as_array())
            .unwrap_or_else(|| {
                panic!("ConflictProblemDto must be an allOf composition, got: {schema}")
            });

        assert_eq!(
            all_of.len(),
            2,
            "ConflictProblemDto's allOf must have exactly two members: ProblemDetails and its \
             own extra fields, got: {all_of:?}"
        );
        assert_eq!(
            all_of.first().and_then(|member| member.get("$ref")),
            Some(&serde_json::Value::String(
                "#/components/schemas/ProblemDetails".to_string()
            )),
            "ConflictProblemDto's allOf must reference ProblemDetails first (the flattened field)"
        );
    }
}
