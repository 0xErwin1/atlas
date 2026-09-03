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

/// Re-keys one fragment's own paths under its owning component's V2
/// namespace (`v2-e3-s6` D1), before that fragment is merged into the
/// composed document. `stable_id` is the same string
/// `stamp_component_ownership` already uses for `tag` and
/// `x-atlas-component` on this fragment's operations — never a second,
/// independently maintained mapping.
fn mounted_fragment(doc: utoipa::openapi::OpenApi, stable_id: &str) -> utoipa::openapi::OpenApi {
    prefix_document_paths(doc, &crate::router_audit::v2_namespace(stable_id))
}

/// Composes the full OpenAPI document from each component's own fragment
/// (D4): `ApiDoc` contributes only `info(...)`, then `platform`'s,
/// `custos`'s, and `acta`'s fragments are merged in, in this fixed order.
/// No other component can structurally contribute — this function only
/// ever calls these three named fragment constructors (T2.18). Each
/// fragment is re-mounted under its own `/api/v2/<component>` namespace
/// (`mounted_fragment`, `v2-e3-s6` D1) BEFORE it merges into `ApiDoc`, since
/// the owning component is known unambiguously per fragment and would
/// otherwise have to be recovered from the `x-atlas-component` stamp the
/// same fragment just wrote — a circular derivation the ownership tests
/// refuse. Root-level paths (`crate::router_audit::ROOT_LEVEL_PATHS`) stay
/// unprefixed through `prefix_document_paths`' existing exemption. The
/// `Idempotency-Key` header set (D8, T6.3) is derived last, once the
/// document's own path keys already match their final V2-mounted form.
pub(crate) fn document() -> utoipa::openapi::OpenApi {
    let mut doc = ApiDoc::openapi();
    doc.merge(mounted_fragment(
        crate::routes::platform::openapi(),
        crate::routes::platform::OPENAPI_STABLE_ID,
    ));
    doc.merge(mounted_fragment(
        crate::routes::custos::openapi(),
        crate::routes::custos::OPENAPI_STABLE_ID,
    ));
    doc.merge(mounted_fragment(
        crate::routes::acta::openapi(),
        crate::routes::acta::OPENAPI_STABLE_ID,
    ));
    idempotency::apply_idempotency_annotations(doc, &idempotency::idempotent_route_set())
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

/// D8's post-merge `Idempotency-Key` header derivation (`v2-e3-s4` PR6): a
/// composition-time modifier over `document()`'s already-built `OpenApi`
/// value, never a per-operation `#[utoipa::path]` hand-annotation. The
/// annotated set tracks `RouteDeclaration.idempotent` automatically — a
/// route's classification changing in `reg5.rs` moves its header annotation
/// with it, with no second place to edit.
pub(crate) mod idempotency {
    use std::collections::HashSet;

    use atlas_core::registry::{HttpMethod, build};
    use axum::http::header::RETRY_AFTER;
    use utoipa::openapi::header::{Header, HeaderBuilder};
    use utoipa::openapi::path::{Operation, Parameter, ParameterIn};
    use utoipa::openapi::response::Response;
    use utoipa::openapi::schema::{Object, Type};
    use utoipa::openapi::{OpenApi, RefOr, Required};

    use crate::error::IDEMPOTENCY_KEY_IN_FLIGHT_RETRY_AFTER;
    use crate::middleware::idempotency::{
        IDEMPOTENCY_DEGRADED_HEADER, IDEMPOTENCY_DEGRADED_STORE_UNAVAILABLE,
        IDEMPOTENCY_KEY_HEADER, IDEMPOTENT_REPLAYED_HEADER, IDEMPOTENT_REPLAYED_VALUE,
    };
    use crate::reg5::{StorageBackend, reg5_component_entries};

    /// The one response status the middleware emits on its own, before the
    /// handler runs, for both idempotency problem types.
    const CONFLICT_STATUS: &str = "409";

    /// Exact text shared by every one of the 34 annotated operations'
    /// `409` idempotency-conflict response description (T6.7/T6.13): a
    /// replayed problem body differs from the original response only by its
    /// `request_id` field.
    pub(crate) const REPLAYED_BODY_NOTE: &str = "A replayed problem body differs from the original response only by its `request_id` field.";

    /// `(method, path)` for every registry entry with `idempotent: true`
    /// (S3's final 34-route split), joined to the same V2-mounted form
    /// `document()`'s own path keys carry as of `v2-e3-s6` D1.2 — each
    /// entry's own `v2_namespace`, never a shared `/api` literal. Both this
    /// set and `document()`'s path keys MUST move together: joining at the
    /// wrong namespace makes `apply_idempotency_annotations`' `(method,
    /// path)` lookup miss every operation with no error
    /// (`openapi_idempotency_annotations.rs` is what catches this).
    /// `StorageBackend::Filesystem` is the same arbitrary-but-consistent
    /// choice `document()`'s own tests already make: storage-backend Module
    /// entries declare zero routes, so neither backend can change this set.
    #[allow(
        clippy::expect_used,
        reason = "the registry is already proven valid by the startup gate (main.rs's \
                  run_registry_gate) before any request is served; a build() failure here would \
                  mean the process never should have started serving traffic"
    )]
    pub(crate) fn idempotent_route_set() -> HashSet<(HttpMethod, String)> {
        let registry = build(reg5_component_entries(StorageBackend::Filesystem))
            .expect("REG-5 entries must satisfy every registry::build() validator");

        let mut routes = HashSet::new();
        for entry in registry.entries() {
            let namespace = crate::router_audit::v2_namespace(entry.identity.stable_id.as_str());
            for route in &entry.api.routes {
                if route.idempotent {
                    let mounted =
                        crate::router_audit::mounted_path(&namespace, route.path.as_str());
                    routes.insert((route.method, mounted));
                }
            }
        }
        routes
    }

    /// Walks every operation in `doc` and stamps the full `Idempotency-Key`
    /// annotation set (T6.4–T6.7) onto exactly the operations whose
    /// `(method, path)` is in `idempotent_routes` — never any other
    /// operation. `doc`'s path keys are assumed already `/api`-mounted
    /// (called after `prefix_document_paths` in `document()`).
    pub(crate) fn apply_idempotency_annotations(
        mut doc: OpenApi,
        idempotent_routes: &HashSet<(HttpMethod, String)>,
    ) -> OpenApi {
        for (path, item) in doc.paths.paths.iter_mut() {
            for (method, operation) in [
                (HttpMethod::Get, item.get.as_mut()),
                (HttpMethod::Put, item.put.as_mut()),
                (HttpMethod::Post, item.post.as_mut()),
                (HttpMethod::Delete, item.delete.as_mut()),
                (HttpMethod::Options, item.options.as_mut()),
                (HttpMethod::Head, item.head.as_mut()),
                (HttpMethod::Patch, item.patch.as_mut()),
            ] {
                let Some(operation) = operation else {
                    continue;
                };
                if idempotent_routes.contains(&(method, path.clone())) {
                    stamp_idempotency_annotations(operation);
                }
            }
        }
        doc
    }

    /// Mutates `operation` in place with the full annotation set (T6.4–T6.7):
    /// the `Idempotency-Key` request header parameter, the
    /// `Idempotent-Replayed`/`Idempotency-Degraded` response headers on
    /// every response entry including the `409`, and the `409`
    /// idempotency-conflict response entry carrying `Retry-After` and the
    /// replayed-body note. The `409` entry carries the markers because a
    /// stored domain conflict (a duplicate slug, say) can be replayed or
    /// produced by a degraded request like any other handler status; only
    /// the two idempotency problem types themselves, which the middleware
    /// emits before the handler runs, never carry them, and the entry's
    /// description says so. A pre-existing `409` entry on the operation has
    /// this idempotency-specific text and header appended, never silently
    /// replaced. Every header and parameter name is written in canonical
    /// HTTP casing derived from the middleware's own constants.
    fn stamp_idempotency_annotations(operation: &mut Operation) {
        operation
            .parameters
            .get_or_insert_with(Vec::new)
            .push(idempotency_key_parameter());

        let responses = &mut operation.responses.responses;

        responses
            .entry(CONFLICT_STATUS.to_string())
            .or_insert_with(|| RefOr::T(Response::new(String::new())));

        for response in responses.values_mut() {
            if let RefOr::T(response) = response {
                response.headers.insert(
                    canonical_header_name(IDEMPOTENT_REPLAYED_HEADER),
                    replayed_header(),
                );
                response.headers.insert(
                    canonical_header_name(IDEMPOTENCY_DEGRADED_HEADER),
                    degraded_header(),
                );
            }
        }

        if let Some(RefOr::T(response)) = responses.get_mut(CONFLICT_STATUS) {
            response.headers.insert(
                canonical_header_name(RETRY_AFTER.as_str()),
                retry_after_header(),
            );

            response.description = if response.description.is_empty() {
                conflict_response_description()
            } else {
                format!(
                    "{}\n\n{}",
                    response.description,
                    conflict_response_description()
                )
            };
        }
    }

    /// The `Idempotency-Key` request header parameter (T6.4): optional, since
    /// an idempotent route ignores a missing key rather than rejecting the
    /// request (S3 §2).
    fn idempotency_key_parameter() -> Parameter {
        let mut parameter = Parameter::new(canonical_header_name(IDEMPOTENCY_KEY_HEADER));
        parameter.parameter_in = ParameterIn::Header;
        parameter.required = Required::False;
        parameter.description = Some(
            "Client-supplied key used to dedupe a retried request. Optional: a route that \
             honors this header runs and stores its response as usual when the header is \
             absent, and every route that does not honor it ignores the header entirely."
                .to_string(),
        );
        parameter.schema = Some(RefOr::T(Object::with_type(Type::String).into()));
        parameter
    }

    /// The `Idempotent-Replayed` response header (T6.5): present only when
    /// the response was served from the idempotency store, never on the
    /// original response. Any completed status the store holds can be
    /// replayed, so this header is documented on every handler response.
    fn replayed_header() -> Header {
        HeaderBuilder::new()
            .schema(RefOr::T(Object::with_type(Type::Boolean).into()))
            .description(Some(format!(
                "Present with value `{IDEMPOTENT_REPLAYED_VALUE}` when this response was \
                 served from the idempotency store as a replay of a previously completed \
                 request carrying the same Idempotency-Key. Absent on the original response."
            )))
            .build()
    }

    /// The `Idempotency-Degraded` response header (T6.5): present only when
    /// the idempotency store was unavailable and the request executed
    /// without dedup protection. The handler's own response, whatever its
    /// status, is what carries it.
    fn degraded_header() -> Header {
        HeaderBuilder::new()
            .schema(RefOr::T(Object::with_type(Type::String).into()))
            .description(Some(format!(
                "Present with value `{IDEMPOTENCY_DEGRADED_STORE_UNAVAILABLE}` when the \
                 idempotency store was unavailable and the request executed without dedup \
                 protection. Never present together with the replayed marker."
            )))
            .build()
    }

    /// The `Retry-After` response header on the `409` entry: present only
    /// for the `idempotency-key-in-flight` problem type, with the value the
    /// error module sets.
    fn retry_after_header() -> Header {
        HeaderBuilder::new()
            .schema(RefOr::T(Object::with_type(Type::Integer).into()))
            .description(Some(format!(
                "Present with value `{IDEMPOTENCY_KEY_IN_FLIGHT_RETRY_AFTER}` (seconds) only \
                 for the `urn:atlas:error:idempotency-key-in-flight` problem type: the first \
                 request for this Idempotency-Key is still executing."
            )))
            .build()
    }

    /// The `409` idempotency-conflict response description (T6.6/T6.7):
    /// names both problem types a `409` can carry on an idempotent route,
    /// states that neither of them ever carries the replay/degrade markers
    /// while a replayed domain conflict does, and ends with the
    /// replayed-body note.
    fn conflict_response_description() -> String {
        format!(
            "Returned when the Idempotency-Key was already used for a different request \
             (`urn:atlas:error:idempotency-key-conflict`), or when a request carrying the \
             same Idempotency-Key is still executing concurrently \
             (`urn:atlas:error:idempotency-key-in-flight`). Neither of those two problem \
             types ever carries the `{replayed}` or `{degraded}` header; a replayed or \
             degraded domain conflict on this operation carries them like any other \
             response. {REPLAYED_BODY_NOTE}",
            replayed = canonical_header_name(IDEMPOTENT_REPLAYED_HEADER),
            degraded = canonical_header_name(IDEMPOTENCY_DEGRADED_HEADER),
        )
    }

    /// Canonical HTTP casing for a lowercase header name: the first letter
    /// of every `-`-separated segment upper-cased, so `idempotency-key`
    /// becomes `Idempotency-Key`. The document's header and parameter names
    /// are derived from the middleware constants through this function,
    /// never retyped.
    pub(crate) fn canonical_header_name(name: &str) -> String {
        name.split('-')
            .map(|segment| {
                let mut chars = segment.chars();
                match chars.next() {
                    Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join("-")
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use utoipa::openapi::Paths;
        use utoipa::openapi::path::{HttpMethod as UtoipaMethod, PathItem};

        fn operation_with_responses(statuses: &[&str]) -> Operation {
            let mut operation = Operation::new();
            for status in statuses {
                operation.responses.responses.insert(
                    (*status).to_string(),
                    RefOr::T(Response::new(format!("{status} fixture"))),
                );
            }
            operation
        }

        fn stub_document() -> OpenApi {
            let mut paths = Paths::new();
            paths.paths.insert(
                "/api/idempotent/route".to_string(),
                PathItem::new(
                    UtoipaMethod::Post,
                    operation_with_responses(&["201", "401", "422"]),
                ),
            );
            paths.paths.insert(
                "/api/plain/route".to_string(),
                PathItem::new(UtoipaMethod::Get, operation_with_responses(&["200", "404"])),
            );
            OpenApi::new(utoipa::openapi::Info::new("fixture", "0.0.0"), paths)
        }

        fn inline_response<'a>(operation: &'a Operation, status: &str) -> &'a Response {
            let RefOr::T(response) = operation
                .responses
                .responses
                .get(status)
                .unwrap_or_else(|| panic!("{status} response entry must be present"))
            else {
                panic!("{status} response must be an inline Response, not a $ref");
            };
            response
        }

        /// Every documented header and parameter name comes out of the
        /// lowercase middleware constants in canonical HTTP casing.
        #[test]
        fn canonical_header_name_upper_cases_each_segment_of_the_constants() {
            assert_eq!(
                canonical_header_name(IDEMPOTENCY_KEY_HEADER),
                "Idempotency-Key"
            );
            assert_eq!(
                canonical_header_name(IDEMPOTENT_REPLAYED_HEADER),
                "Idempotent-Replayed"
            );
            assert_eq!(
                canonical_header_name(IDEMPOTENCY_DEGRADED_HEADER),
                "Idempotency-Degraded"
            );
            assert_eq!(canonical_header_name(RETRY_AFTER.as_str()), "Retry-After");
        }

        /// T6.2: the derivation pass mutates in the full annotation set for
        /// exactly the operations whose `(method, path)` is in the given
        /// `idempotent: true` set, and touches nothing else. The replay and
        /// degrade markers land on every response including the `409`,
        /// which additionally carries `Retry-After`.
        #[test]
        fn derivation_annotates_only_the_declared_idempotent_operations() {
            let mut idempotent_routes = HashSet::new();
            idempotent_routes.insert((HttpMethod::Post, "/api/idempotent/route".to_string()));

            let annotated = apply_idempotency_annotations(stub_document(), &idempotent_routes);

            let idempotent_op = annotated
                .paths
                .paths
                .get("/api/idempotent/route")
                .and_then(|item| item.post.as_ref())
                .expect("post operation present");
            assert!(
                idempotent_op
                    .parameters
                    .as_ref()
                    .is_some_and(|params| params
                        .iter()
                        .any(|p| p.name == canonical_header_name(IDEMPOTENCY_KEY_HEADER))),
                "idempotent operation must carry the Idempotency-Key request header parameter"
            );

            for status in ["201", "401", "422", CONFLICT_STATUS] {
                let response = inline_response(idempotent_op, status);
                assert!(
                    response
                        .headers
                        .contains_key(&canonical_header_name(IDEMPOTENT_REPLAYED_HEADER)),
                    "{status} must document the replayed marker"
                );
                assert!(
                    response
                        .headers
                        .contains_key(&canonical_header_name(IDEMPOTENCY_DEGRADED_HEADER)),
                    "{status} must document the degraded marker"
                );
                assert_eq!(
                    response
                        .headers
                        .contains_key(&canonical_header_name(RETRY_AFTER.as_str())),
                    status == CONFLICT_STATUS,
                    "only the 409 documents Retry-After ({status})"
                );
            }

            let conflict_response = inline_response(idempotent_op, CONFLICT_STATUS);
            assert!(conflict_response.description.contains(REPLAYED_BODY_NOTE));
            assert!(
                conflict_response
                    .description
                    .contains("urn:atlas:error:idempotency-key-conflict")
            );
            assert!(
                conflict_response
                    .description
                    .contains("urn:atlas:error:idempotency-key-in-flight")
            );

            let plain_op = annotated
                .paths
                .paths
                .get("/api/plain/route")
                .and_then(|item| item.get.as_ref())
                .expect("get operation present");
            assert!(
                plain_op.parameters.is_none(),
                "non-idempotent operation must carry no request header parameter"
            );
            assert!(
                !plain_op.responses.responses.contains_key(CONFLICT_STATUS),
                "non-idempotent operation must carry no 409 idempotency-conflict entry"
            );
            for status in ["200", "404"] {
                let response = inline_response(plain_op, status);
                assert!(
                    response.headers.is_empty(),
                    "non-idempotent {status} must carry no idempotency response header"
                );
            }
        }

        /// T6.7: a pre-existing `409` entry (e.g. a domain-level conflict
        /// like `revision-conflict`) keeps its own description first and
        /// gains the idempotency text as an additional paragraph, never
        /// overwritten, plus the `Retry-After` header and both markers (a
        /// stored domain conflict can be replayed).
        #[test]
        fn derivation_appends_to_a_pre_existing_409_entry_without_erasing_it() {
            let mut paths = Paths::new();
            let mut operation = Operation::new();
            operation.responses.responses.insert(
                CONFLICT_STATUS.to_string(),
                RefOr::T(Response::new("Revision conflict".to_string())),
            );
            paths.paths.insert(
                "/api/conflicting/route".to_string(),
                PathItem::new(UtoipaMethod::Post, operation),
            );
            let doc = OpenApi::new(utoipa::openapi::Info::new("fixture", "0.0.0"), paths);

            let mut idempotent_routes = HashSet::new();
            idempotent_routes.insert((HttpMethod::Post, "/api/conflicting/route".to_string()));

            let annotated = apply_idempotency_annotations(doc, &idempotent_routes);

            let operation = annotated
                .paths
                .paths
                .get("/api/conflicting/route")
                .and_then(|item| item.post.as_ref())
                .expect("post operation present");
            let response = inline_response(operation, CONFLICT_STATUS);

            assert_eq!(
                response.description,
                format!("Revision conflict\n\n{}", conflict_response_description()),
                "the original description must be kept first, the idempotency text appended"
            );
            assert!(
                response
                    .description
                    .contains("urn:atlas:error:idempotency-key-conflict")
            );
            assert!(
                response
                    .description
                    .contains("urn:atlas:error:idempotency-key-in-flight")
            );
            assert!(
                response
                    .headers
                    .contains_key(&canonical_header_name(RETRY_AFTER.as_str()))
            );
            assert!(
                response
                    .headers
                    .contains_key(&canonical_header_name(IDEMPOTENT_REPLAYED_HEADER))
            );
            assert!(
                response
                    .headers
                    .contains_key(&canonical_header_name(IDEMPOTENCY_DEGRADED_HEADER))
            );
        }

        /// T1.10/T1.11 (D1.2): every entry in `idempotent_route_set()` is
        /// joined at that entry's OWN component V2 namespace, never a shared
        /// `/api` literal — the failure mode that silently drops all 34
        /// annotations if the document keys move to V2 while this set stays
        /// at V1.
        #[test]
        fn idempotent_route_set_joins_at_each_entrys_own_v2_namespace() {
            let registry = build(reg5_component_entries(StorageBackend::Filesystem))
                .expect("REG-5 entries must satisfy every registry::build() validator");

            let mut expected = HashSet::new();
            for entry in registry.entries() {
                let namespace =
                    crate::router_audit::v2_namespace(entry.identity.stable_id.as_str());
                for route in &entry.api.routes {
                    if route.idempotent {
                        expected.insert((
                            route.method,
                            crate::router_audit::mounted_path(&namespace, route.path.as_str()),
                        ));
                    }
                }
            }

            assert_eq!(idempotent_route_set(), expected);
            assert!(
                idempotent_route_set()
                    .iter()
                    .all(|(_, path)| path.starts_with("/api/v2/")),
                "every idempotent route must be joined at its V2 namespace, not /api"
            );
        }
    }
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
            let namespace = crate::router_audit::v2_namespace(component_id);
            for route in &entry.api.routes {
                let mounted = crate::router_audit::mounted_path(&namespace, route.path.as_str());
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

    /// T1.3/T1.4 (D1): `mounted_fragment` re-keys a fabricated single-operation
    /// fragment under `stable_id`'s own V2 namespace, reusing
    /// `prefix_document_paths` — no separate string-building logic of its
    /// own.
    #[test]
    fn mounted_fragment_prefixes_paths_under_the_stable_ids_own_v2_namespace() {
        use utoipa::openapi::path::{HttpMethod as UtoipaMethod, Operation, PathItem};
        use utoipa::openapi::{Info, Paths};

        let mut paths = Paths::new();
        paths.paths.insert(
            "/x".to_string(),
            PathItem::new(UtoipaMethod::Get, Operation::new()),
        );
        let fragment = utoipa::openapi::OpenApi::new(Info::new("fixture", "0.0.0"), paths);

        let mounted = mounted_fragment(fragment, "acta");

        assert!(
            mounted.paths.paths.contains_key("/api/v2/acta/x"),
            "expected /api/v2/acta/x, got keys: {:?}",
            mounted.paths.paths.keys().collect::<Vec<_>>()
        );
    }

    /// T1.5/T1.6 (D1): the real, merged `document()` keys every operation
    /// under its owning component's own V2 namespace and carries no bare
    /// `/api/...` (V1-form) key.
    #[test]
    fn document_paths_are_keyed_under_each_components_own_v2_namespace() {
        let doc = document();

        for prefix in ["/api/v2/acta/", "/api/v2/custos/", "/api/v2/platform/"] {
            assert!(
                doc.paths.paths.keys().any(|key| key.starts_with(prefix)),
                "expected at least one document key starting with {prefix}"
            );
        }

        let bare_v1_keys: Vec<&String> = doc
            .paths
            .paths
            .keys()
            .filter(|key| {
                key.starts_with("/api/")
                    && !key.starts_with("/api/v2/")
                    && !crate::router_audit::ROOT_LEVEL_PATHS.contains(&key.as_str())
            })
            .collect();
        assert!(
            bare_v1_keys.is_empty(),
            "no document key should still carry the bare /api (V1) form: {bare_v1_keys:?}"
        );
    }

    /// T1.7/T1.8 (D1): `/health`, `/ready`, and `/version` stay unprefixed on
    /// the real document, even after the per-fragment V2 prefixing —
    /// inherited from `prefix_document_paths`'s existing `ROOT_LEVEL_PATHS`
    /// exemption, not a separate implementation. `/openapi.json` and
    /// `/scalar` are also `ROOT_LEVEL_PATHS` members but, per
    /// `openapi_zero_drift.rs`'s `UNANNOTATED_ROUTES`, carry no
    /// `#[utoipa::path]` annotation of their own and so never produce a
    /// document operation to inspect here.
    #[test]
    fn root_level_paths_stay_unprefixed_on_the_real_document() {
        let doc = document();

        for path in ["/health", "/ready", "/version"] {
            assert!(
                doc.paths.paths.contains_key(path),
                "expected root-level path {path} to appear unprefixed in the document"
            );
        }
    }

    /// T1.24: after this slice, no document path key begins with the V1
    /// form (`/api/<rel>` where `<rel>` is not `v2/<component>/...`) — the
    /// same check as `document_paths_are_keyed_under_each_components_own_v2_namespace`'s
    /// negative assertion, restated as its own named scenario per the spec's
    /// acceptance gate.
    #[test]
    fn document_contains_no_v1_form_path_key() {
        let doc = document();

        for key in doc.paths.paths.keys() {
            if key.starts_with("/api/") {
                assert!(
                    key.starts_with("/api/v2/"),
                    "document key {key} is neither V2-form nor a root-level exemption"
                );
            } else {
                assert!(
                    crate::router_audit::ROOT_LEVEL_PATHS.contains(&key.as_str()),
                    "document key {key} carries no /api prefix and is not a root-level path"
                );
            }
        }
    }
}
