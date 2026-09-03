#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! D8's `Idempotency-Key` annotation audit (`v2-e3-s4` PR6, T6.8–T6.14):
//! bidirectional INV-SET between the registry's `idempotent: true` set (the
//! final S3 34/176 split) and the composed document's annotated-operation
//! set, the placement of the replay/degrade markers on every response and
//! of the `Retry-After` header on the `409` alone, the explicit negative check on the 176
//! non-idempotent routes (which fold in the 12 dedup-excluded ones), and the
//! exact-text replayed-body note check.
//!
//! Mirrors `openapi_zero_drift.rs`'s own registry-rebuilding pattern: this
//! file derives its expected set from `reg5_component_entries` directly,
//! never from a hand-typed literal list (INV-DATA-DRIVEN). Header names are
//! compared case-insensitively, as HTTP itself does.

use std::collections::HashSet;

use atlas_core::registry::HttpMethod;
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use atlas_server::router_audit::{V1_NAMESPACE, diff_route_sets, mounted_path};
use atlas_server::routes::openapi::openapi;
use utoipa::openapi::path::Operation;
use utoipa::openapi::response::Response;
use utoipa::openapi::{OpenApi, RefOr};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const IDEMPOTENT_REPLAYED_HEADER: &str = "idempotent-replayed";
const IDEMPOTENCY_DEGRADED_HEADER: &str = "idempotency-degraded";
const RETRY_AFTER_HEADER: &str = "retry-after";
const CONFLICT_STATUS: &str = "409";
const CONFLICT_URN: &str = "urn:atlas:error:idempotency-key-conflict";
const IN_FLIGHT_URN: &str = "urn:atlas:error:idempotency-key-in-flight";
const REPLAYED_BODY_NOTE: &str =
    "A replayed problem body differs from the original response only by its `request_id` field.";

/// Every `(method, path)` the registry declares with the given `idempotent`
/// flag, joined to the same `/api`-mounted form the composed document's own
/// path keys carry. Pinned to `V1_NAMESPACE`, never the suite's flippable
/// default (`v2-e3-s5`): it is the document's namespace until S6/PR1 re-keys
/// it.
fn declared_routes(idempotent: bool) -> HashSet<(HttpMethod, String)> {
    let registry = atlas_core::registry::build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let mut routes = HashSet::new();
    for entry in registry.entries() {
        for route in &entry.api.routes {
            if route.idempotent == idempotent {
                routes.insert((
                    route.method,
                    mounted_path(V1_NAMESPACE, route.path.as_str()),
                ));
            }
        }
    }
    routes
}

/// The 34 routes T6.8 asserts carry the full annotation set.
fn declared_idempotent_routes() -> HashSet<(HttpMethod, String)> {
    declared_routes(true)
}

/// The 176 routes T6.10 asserts carry no annotation, including the 12
/// dedup-excluded ones (six streamed-upload, six one-shot-secret-returning),
/// all folded into `idempotent: false` per S3.
fn declared_non_idempotent_routes() -> HashSet<(HttpMethod, String)> {
    declared_routes(false)
}

/// Every `(method, operation)` pair mounted on one document path item.
fn operations(item: &utoipa::openapi::PathItem) -> Vec<(HttpMethod, &Operation)> {
    [
        (HttpMethod::Get, item.get.as_ref()),
        (HttpMethod::Put, item.put.as_ref()),
        (HttpMethod::Post, item.post.as_ref()),
        (HttpMethod::Delete, item.delete.as_ref()),
        (HttpMethod::Options, item.options.as_ref()),
        (HttpMethod::Head, item.head.as_ref()),
        (HttpMethod::Patch, item.patch.as_ref()),
    ]
    .into_iter()
    .filter_map(|(method, operation)| operation.map(|operation| (method, operation)))
    .collect()
}

fn operation_for<'a>(document: &'a OpenApi, method: HttpMethod, path: &str) -> &'a Operation {
    let item = document
        .paths
        .paths
        .get(path)
        .unwrap_or_else(|| panic!("{method:?} {path} must have a document path entry"));
    operations(item)
        .into_iter()
        .find(|(candidate, _)| *candidate == method)
        .map(|(_, operation)| operation)
        .unwrap_or_else(|| panic!("{method:?} {path} must have a matching document operation"))
}

fn inline_response<'a>(status: &str, response: &'a RefOr<Response>) -> &'a Response {
    let RefOr::T(response) = response else {
        panic!("{status} response must be an inline Response, not a $ref");
    };
    response
}

fn carries_key_parameter(operation: &Operation) -> bool {
    operation.parameters.as_ref().is_some_and(|params| {
        params
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(IDEMPOTENCY_KEY_HEADER))
    })
}

fn has_header(response: &Response, name: &str) -> bool {
    response
        .headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case(name))
}

/// Every `(method, path)` operation in the composed document carrying the
/// `Idempotency-Key` request-header parameter.
fn annotated_routes(document: &OpenApi) -> HashSet<(HttpMethod, String)> {
    let mut routes = HashSet::new();
    for (path, item) in &document.paths.paths {
        for (method, operation) in operations(item) {
            if carries_key_parameter(operation) {
                routes.insert((method, path.clone()));
            }
        }
    }
    routes
}

/// T6.8/T6.9 (INV-SET, bidirectional): the set of operations carrying the
/// `Idempotency-Key` request-header parameter equals exactly the registry's
/// `idempotent: true` set — an annotated-but-undeclared operation or a
/// declared-but-unannotated route is named on failure via `diff_route_sets`.
#[test]
fn idempotency_key_annotation_matches_the_declared_idempotent_set_exactly() {
    let declared = declared_idempotent_routes();
    let annotated = annotated_routes(&openapi());

    let diff = diff_route_sets(&declared, &annotated);
    assert!(
        diff.is_empty(),
        "declared-but-unannotated: {:?}; annotated-but-undeclared: {:?}",
        diff.left_only,
        diff.right_only
    );
}

/// T6.9: confirmed against the final, already-merged 34-route split.
#[test]
fn exactly_34_routes_are_declared_idempotent() {
    let declared = declared_idempotent_routes();
    assert_eq!(
        declared.len(),
        34,
        "expected exactly 34 idempotent: true routes per S3's final split, found {}: {:?}",
        declared.len(),
        declared
    );
}

/// Placement audit over all 34 idempotent operations: every response entry,
/// the `409` included, documents both the replayed and the degraded marker
/// (the middleware replays any stored status, a stored domain `409` among
/// them, and degrades onto the handler's own response, whatever its status);
/// only the `409` entry documents `Retry-After`, and it names both
/// problem-type URNs.
#[test]
fn markers_sit_on_every_response_and_retry_after_on_the_409() {
    let document = openapi();
    let declared = declared_idempotent_routes();

    let mut offenders = Vec::new();
    for (method, path) in &declared {
        let operation = operation_for(&document, *method, path);

        let mut saw_conflict = false;
        for (status, response) in &operation.responses.responses {
            let response = inline_response(status, response);
            let replayed = has_header(response, IDEMPOTENT_REPLAYED_HEADER);
            let degraded = has_header(response, IDEMPOTENCY_DEGRADED_HEADER);
            let retry_after = has_header(response, RETRY_AFTER_HEADER);

            let well_placed = if status == CONFLICT_STATUS {
                saw_conflict = true;
                replayed
                    && degraded
                    && retry_after
                    && response.description.contains(CONFLICT_URN)
                    && response.description.contains(IN_FLIGHT_URN)
            } else {
                replayed && degraded && !retry_after
            };

            if !well_placed {
                offenders.push((*method, path.clone(), status.clone()));
            }
        }

        if !saw_conflict {
            offenders.push((*method, path.clone(), "missing 409".to_string()));
        }
    }

    assert!(
        offenders.is_empty(),
        "idempotent operation response(s) with misplaced idempotency headers: {offenders:?}"
    );
}

/// T6.10/T6.11: none of the 176 `idempotent: false` routes (which folds in
/// all 12 dedup-excluded routes per D8) carries the `Idempotency-Key`
/// request-header parameter, either marker header on any response, a
/// `Retry-After` on a `409`, or either idempotency problem-type URN in a
/// `409` description.
#[test]
fn no_non_idempotent_route_carries_any_idempotency_annotation() {
    let document = openapi();
    let non_idempotent = declared_non_idempotent_routes();

    let mut offenders = Vec::new();
    for (path, item) in &document.paths.paths {
        for (method, operation) in operations(item) {
            if !non_idempotent.contains(&(method, path.clone())) {
                continue;
            }

            if carries_key_parameter(operation) {
                offenders.push((method, path.clone(), "parameter".to_string()));
            }

            for (status, response) in &operation.responses.responses {
                let response = inline_response(status, response);
                let is_conflict = status == CONFLICT_STATUS;

                let leaked = has_header(response, IDEMPOTENT_REPLAYED_HEADER)
                    || has_header(response, IDEMPOTENCY_DEGRADED_HEADER)
                    || (is_conflict
                        && (has_header(response, RETRY_AFTER_HEADER)
                            || response.description.contains(CONFLICT_URN)
                            || response.description.contains(IN_FLIGHT_URN)));

                if leaked {
                    offenders.push((method, path.clone(), status.clone()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "non-idempotent route(s) carrying an idempotency annotation: {offenders:?}"
    );
}

/// T6.12 (adversarial proof): `diff_route_sets` must actually catch a
/// drifted route — a declared-idempotent route the document never annotated
/// (simulated here by removing one route from the "annotated" side without
/// touching the real registry or document), naming exactly that route.
#[test]
fn the_bidirectional_audit_detects_a_drifted_route() {
    let declared = declared_idempotent_routes();
    let mut annotated = declared.clone();

    let dropped = annotated
        .iter()
        .next()
        .cloned()
        .expect("the 34-route idempotent set must be non-empty");
    annotated.remove(&dropped);

    let diff = diff_route_sets(&declared, &annotated);
    assert_eq!(
        diff.left_only,
        vec![dropped.clone()],
        "diff_route_sets must name exactly the route dropped from the annotated side"
    );
    assert!(diff.right_only.is_empty());
}

/// T6.13/T6.14: the replayed-body note's exact text is present, verbatim, on
/// every one of the 34 operations' `409` idempotency-conflict response
/// description — checked exhaustively, not on a sample.
#[test]
fn the_replayed_body_note_is_present_verbatim_on_every_idempotent_operation() {
    let document = openapi();
    let declared = declared_idempotent_routes();

    let mut missing_note = Vec::new();
    for (method, path) in &declared {
        let operation = operation_for(&document, *method, path);

        let has_note = operation
            .responses
            .responses
            .get(CONFLICT_STATUS)
            .is_some_and(|response| {
                inline_response(CONFLICT_STATUS, response)
                    .description
                    .contains(REPLAYED_BODY_NOTE)
            });

        if !has_note {
            missing_note.push((*method, path.clone()));
        }
    }

    assert!(
        missing_note.is_empty(),
        "idempotent operation(s) missing the exact replayed-body note on their 409 response: \
         {missing_note:?}"
    );
}
