//! Request-span `component`/`operation` fields, both directions (E11-S7
//! design D2, gate G3).
//!
//! `apply_layers`'s `make_span_with` closure opens the `"http"` span with
//! `component` and `operation` when the request's `MatchedPath` resolves
//! through the `RouteIndex`, and without either field otherwise — never an
//! empty placeholder (INV-SPAN-FIELDS-CLOSED). These cases capture the
//! span's fields at creation through a small `tracing_subscriber::Layer`,
//! since a span field's recorded value is not otherwise inspectable from
//! outside the subscriber that owns it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use atlas_core::registry::{
    Api, Authorization, Capabilities, ComponentEntry, ComponentId, ComponentKind, ContractVersion,
    Diagnostics, Experience, HttpMethod, Identity, RouteDeclaration, RoutePath, build,
};
use atlas_server::observability::route_index::RouteIndex;
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use atlas_server::router_audit::v2_namespace;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::{MethodRouter, delete, get, head, options, patch, post, put},
};
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

async fn ok() -> StatusCode {
    StatusCode::OK
}

/// A `tracing_subscriber::Layer` that records every `"http"` span's fields,
/// in creation order, into a shared buffer — the only way to inspect what a
/// span carried without instrumenting the production closure itself.
#[derive(Clone, Default)]
struct HttpSpanCapture {
    spans: Arc<Mutex<Vec<HashMap<String, String>>>>,
}

impl HttpSpanCapture {
    fn spans(&self) -> Vec<HashMap<String, String>> {
        self.spans.lock().unwrap().clone()
    }
}

struct FieldRecorder<'a>(&'a mut HashMap<String, String>);

impl Visit for FieldRecorder<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for HttpSpanCapture
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        if attrs.metadata().name() != "http" {
            return;
        }
        let mut fields = HashMap::new();
        attrs.record(&mut FieldRecorder(&mut fields));
        self.spans.lock().unwrap().push(fields);
    }
}

/// Runs `app` against `request` with `capture` installed as the default
/// subscriber for the call's duration, returning the response.
async fn oneshot_captured(
    app: Router,
    request: Request<Body>,
    capture: &HttpSpanCapture,
) -> axum::response::Response {
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let _guard = tracing::subscriber::set_default(subscriber);
    app.oneshot(request).await.unwrap()
}

/// A synthetic single-route component, mirroring
/// `route_index_derivation.rs`'s anti-vacuity fixture (design D7.1): the
/// route lives only in this index, so a captured `component`/`operation`
/// pair can only have come from `RouteIndex::from_registry`, not from a
/// coincidence with a real registry entry.
fn synthetic_index() -> RouteIndex {
    let entry = ComponentEntry {
        identity: Identity {
            stable_id: ComponentId::new("s7_span_probe").expect("valid component id"),
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
                path: RoutePath::new("/owned").expect("valid route path"),
                operation_id: "s7_span_probe_owned".to_string(),
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
    };

    let registry =
        build(vec![entry]).expect("the synthetic single-entry registry must satisfy build()");
    RouteIndex::from_registry(&registry)
}

/// A matched route owned by a named component produces a span carrying
/// `component` and `operation` equal to that entry's stable id and
/// operation id.
#[tokio::test]
async fn matched_route_span_carries_component_and_operation() {
    let index = synthetic_index();
    let path = atlas_server::router_audit::mounted_path(
        &atlas_server::router_audit::v2_namespace("s7_span_probe"),
        "/owned",
    );

    let app = atlas_server::test_app_with_route_and_index(&path, get(ok), Arc::new(index));
    let capture = HttpSpanCapture::default();

    let response = oneshot_captured(
        app,
        Request::builder()
            .uri(path.as_str())
            .body(Body::empty())
            .unwrap(),
        &capture,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let spans = capture.spans();
    assert_eq!(spans.len(), 1, "exactly one http span must be opened");
    assert_eq!(
        spans[0].get("component").map(String::as_str),
        Some("s7_span_probe")
    );
    assert_eq!(
        spans[0].get("operation").map(String::as_str),
        Some("s7_span_probe_owned")
    );
}

/// A request to a path with no registry entry produces a span with neither
/// `component` nor `operation` — the field is absent, not present with an
/// empty value.
#[tokio::test]
async fn unmatched_path_span_carries_neither_field() {
    let app = atlas_server::test_app_with_route("/known", get(ok));
    let capture = HttpSpanCapture::default();

    let response = oneshot_captured(
        app,
        Request::builder()
            .uri("/does-not-exist")
            .body(Body::empty())
            .unwrap(),
        &capture,
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let spans = capture.spans();
    assert_eq!(spans.len(), 1, "an unmatched path still opens a span");
    assert!(
        !spans[0].contains_key("component"),
        "component must be absent, not empty: {:?}",
        spans[0]
    );
    assert!(
        !spans[0].contains_key("operation"),
        "operation must be absent, not empty: {:?}",
        spans[0]
    );
}

/// A matched route that `RouteIndex::empty()` does not cover behaves like an
/// unmatched one: neither field is added. Gives G3's absent-field case a
/// second, container-free witness through the published `test_app_with_route`
/// seam (design D2).
#[tokio::test]
async fn matched_route_against_empty_index_carries_neither_field() {
    let app = atlas_server::test_app_with_route("/known", get(ok));
    let capture = HttpSpanCapture::default();

    let response = oneshot_captured(
        app,
        Request::builder()
            .uri("/known")
            .body(Body::empty())
            .unwrap(),
        &capture,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let spans = capture.spans();
    assert_eq!(spans.len(), 1);
    assert!(!spans[0].contains_key("component"));
    assert!(!spans[0].contains_key("operation"));
}

/// `/health`, `/version`, and `/ready` keep their existing span-suppression
/// behavior unchanged: no span is opened at all, so neither new field can
/// appear.
#[tokio::test]
async fn root_diagnostic_paths_keep_span_suppression_unchanged() {
    for path in ["/health", "/version", "/ready"] {
        let app = atlas_server::test_app_with_route(path, get(ok));
        let capture = HttpSpanCapture::default();

        let response = oneshot_captured(
            app,
            Request::builder().uri(path).body(Body::empty()).unwrap(),
            &capture,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK, "case {path}");
        assert_eq!(
            capture.spans().len(),
            0,
            "case {path}: span suppression must be unchanged — no span at all, not just \
             missing fields"
        );
    }
}

/// One real REG-5 route declaring at least one `{param}` placeholder,
/// picked deterministically (first, in ascending `stable_id` order, then
/// first in the owning entry's own declaration order) so the choice does
/// not depend on iteration order or on today's registry contents staying
/// fixed.
struct PlaceholderRoute {
    stable_id: String,
    method: HttpMethod,
    path: String,
    operation_id: String,
}

fn first_placeholder_route() -> PlaceholderRoute {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let mut entries: Vec<&ComponentEntry> = registry.entries().iter().collect();
    entries.sort_by(|a, b| a.identity.stable_id.cmp(&b.identity.stable_id));

    for entry in entries {
        if let Some(route) = entry
            .api
            .routes
            .iter()
            .find(|route| route.path.as_str().contains('{'))
        {
            return PlaceholderRoute {
                stable_id: entry.identity.stable_id.as_str().to_string(),
                method: route.method,
                path: route.path.as_str().to_string(),
                operation_id: route.operation_id.clone(),
            };
        }
    }

    panic!("no REG-5 route declares a `{{param}}` placeholder");
}

/// Builds the `MethodRouter` for `method`, mounting `ok` — the concrete
/// handler is irrelevant to the span assertion, only the route's shape is.
fn method_router_for(method: HttpMethod) -> MethodRouter {
    match method {
        HttpMethod::Get => get(ok),
        HttpMethod::Post => post(ok),
        HttpMethod::Put => put(ok),
        HttpMethod::Patch => patch(ok),
        HttpMethod::Delete => delete(ok),
        HttpMethod::Head => head(ok),
        HttpMethod::Options => options(ok),
    }
}

/// Converts the registry's own `HttpMethod` into `axum::http::Method`
/// through its `Display` impl's wire-form text, matching the mapping
/// `RouteIndex::get` uses on the request-handling side.
fn axum_method(method: HttpMethod) -> axum::http::Method {
    axum::http::Method::from_bytes(method.to_string().as_bytes())
        .expect("every HttpMethod variant's Display text is a valid HTTP method token")
}

/// Replaces every `{placeholder}` segment in `template` with a fixed
/// literal, deriving a concrete request path independently of
/// `router_audit::mounted_path` — the seam this test exists to check for
/// key agreement with, not to lean on.
fn concrete_path(template: &str) -> String {
    let mut result = String::new();
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        let end = rest[start..]
            .find('}')
            .map(|offset| start + offset)
            .expect("registry route paths carry balanced braces");
        result.push_str(&rest[..start]);
        result.push_str("test-value");
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}

/// A matched route mounted the way the application mounts a component's
/// routes — nested under `/api/v2/<component>`, not built from a hand-typed
/// template — produces a span carrying `component`/`operation` equal to the
/// owning registry entry's stable id and operation id. Closes the gap the
/// other cases here leave open: they build the index and the served route
/// from the same literal string, so key agreement is true by construction
/// rather than proven against how `app()` actually nests routes.
#[tokio::test]
async fn placeholder_route_mounted_like_the_app_carries_component_and_operation() {
    let route = first_placeholder_route();
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");
    let index = RouteIndex::from_registry(&registry);

    let namespace = v2_namespace(&route.stable_id);
    let mounted_router = Router::new().nest(
        &namespace,
        Router::new().route(&route.path, method_router_for(route.method)),
    );
    let app = atlas_server::test_app_with_router_and_index(mounted_router, Arc::new(index));

    let uri = format!("{namespace}{}", concrete_path(&route.path));
    let capture = HttpSpanCapture::default();

    let response = oneshot_captured(
        app,
        Request::builder()
            .method(axum_method(route.method))
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
        &capture,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let spans = capture.spans();
    assert_eq!(spans.len(), 1, "exactly one http span must be opened");
    assert_eq!(
        spans[0].get("component").map(String::as_str),
        Some(route.stable_id.as_str())
    );
    assert_eq!(
        spans[0].get("operation").map(String::as_str),
        Some(route.operation_id.as_str())
    );
}

/// The same route, mounted one level deeper than the application ever
/// mounts it, produces a `MatchedPath` the index does not recognize: the
/// span carries neither field. Proves the previous test is sensitive to key
/// disagreement rather than passing regardless of the mount shape.
#[tokio::test]
async fn placeholder_route_mounted_deeper_than_the_app_carries_neither_field() {
    let route = first_placeholder_route();
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");
    let index = RouteIndex::from_registry(&registry);

    let namespace = v2_namespace(&route.stable_id);
    let mounted_router = Router::new().nest(
        "/extra",
        Router::new().nest(
            &namespace,
            Router::new().route(&route.path, method_router_for(route.method)),
        ),
    );
    let app = atlas_server::test_app_with_router_and_index(mounted_router, Arc::new(index));

    let uri = format!("/extra{namespace}{}", concrete_path(&route.path));
    let capture = HttpSpanCapture::default();

    let response = oneshot_captured(
        app,
        Request::builder()
            .method(axum_method(route.method))
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
        &capture,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);

    let spans = capture.spans();
    assert_eq!(spans.len(), 1, "exactly one http span must be opened");
    assert!(
        !spans[0].contains_key("component"),
        "component must be absent when the mount shape disagrees with the index key: {:?}",
        spans[0]
    );
    assert!(
        !spans[0].contains_key("operation"),
        "operation must be absent when the mount shape disagrees with the index key: {:?}",
        spans[0]
    );
}
