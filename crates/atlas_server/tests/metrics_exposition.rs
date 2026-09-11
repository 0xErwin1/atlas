//! `/metrics` exposition and the shared shutdown drain (E11-S7 design D5,
//! D6, D7.3, D7.4; gates G1a, G8).
//!
//! Container-free throughout: `observability::exposition::router` takes no
//! `AppState` (`tests/error_model.rs:19` precedent), the registry/audit
//! check below reads in-process data structures only, and `drain_signal` is
//! a pure function of a `watch::Receiver`. G1b — the one container-backed
//! half of gate G1 — is a case appended to `tests/api_unmatched_path_fallback.rs`
//! instead, per design D6.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_core::registry::build;
use atlas_server::observability::drain_signal;
use atlas_server::observability::exposition;
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use metrics_exporter_prometheus::PrometheusBuilder;
use tokio::sync::watch;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// U5.2 — the exposition router, exercised directly (D7.4)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn metrics_route_answers_200_with_both_metric_names() {
    let recorder = PrometheusBuilder::new()
        .set_buckets(atlas_server::observability::metrics::REQUEST_DURATION_BUCKETS)
        .expect("bucket list must be non-empty")
        .build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);

    metrics::counter!(atlas_server::observability::metrics::REQUESTS_TOTAL).increment(1);
    metrics::histogram!(atlas_server::observability::metrics::REQUEST_DURATION_SECONDS)
        .record(0.01);

    let app = exposition::router(handle);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .expect("content-type header must be present")
        .to_str()
        .expect("content-type must be valid utf-8");
    assert!(
        content_type.starts_with("text/plain"),
        "expected a text/plain content type, got {content_type}"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();

    assert!(
        body.contains(atlas_server::observability::metrics::REQUESTS_TOTAL),
        "expected {} in the rendered body, got:\n{body}",
        atlas_server::observability::metrics::REQUESTS_TOTAL
    );
    assert!(
        body.contains(atlas_server::observability::metrics::REQUEST_DURATION_SECONDS),
        "expected {} in the rendered body, got:\n{body}",
        atlas_server::observability::metrics::REQUEST_DURATION_SECONDS
    );
}

#[tokio::test]
async fn any_other_path_answers_404() {
    let recorder = PrometheusBuilder::new()
        .set_buckets(atlas_server::observability::metrics::REQUEST_DURATION_BUCKETS)
        .expect("bucket list must be non-empty")
        .build_recorder();
    let handle = recorder.handle();

    let app = exposition::router(handle);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// U5.3 — G1a: `/metrics` is structurally absent from the public surface
// ---------------------------------------------------------------------------

/// G1a (design D6): `/metrics` appears in no `RouteDeclaration` in the
/// registry, and in no component's full declared `(method, path)` union
/// (`router_audit::{platform,custos,acta}_declared_route_paths`, which
/// exposes the same table `router_audit::declared_routes()` builds,
/// including auth-layered routes — `pub(crate)` and therefore unreachable
/// from this integration-test crate directly). Combined with the
/// already-green `api_router_mount_assertion.rs` (router ⇔ registry, both
/// directions), a path absent from both is a path `app()` cannot serve
/// (INV-METRICS-SECOND-LISTENER-ONLY). Written as a standing guard: nothing
/// in this slice touches the registry or any component router, so this is
/// expected to already pass.
#[test]
fn metrics_is_absent_from_the_registry_and_every_declared_routes_table() {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    for entry in registry.entries() {
        for route in &entry.api.routes {
            assert_ne!(
                route.path.as_str(),
                "/metrics",
                "component {:?} declares /metrics as a registry route",
                entry.identity.stable_id
            );
        }
    }

    let declared_paths: Vec<(&str, &'static str)> =
        atlas_server::router_audit::platform_declared_route_paths()
            .into_iter()
            .map(|(_, path)| ("platform", path))
            .chain(
                atlas_server::router_audit::custos_declared_route_paths()
                    .into_iter()
                    .map(|(_, path)| ("custos", path)),
            )
            .chain(
                atlas_server::router_audit::acta_declared_route_paths()
                    .into_iter()
                    .map(|(_, path)| ("acta", path)),
            )
            .collect();

    for (component, path) in &declared_paths {
        assert_ne!(
            *path, "/metrics",
            "component {component} declares /metrics in its declared_routes() table"
        );
    }

    // Anti-vacuity: the three tables together are non-empty, so an absence
    // check against an accidentally-empty table could not pass vacuously.
    assert!(!declared_paths.is_empty());
}

// ---------------------------------------------------------------------------
// U5.4 — the shared drain signal, proven on the signal (D7.3)
// ---------------------------------------------------------------------------

/// D7.3: `drain_signal` is a pure function of a `watch::Receiver`. This is
/// the thing that can actually be wired wrong — a second server given a
/// fresh channel instead of the one shared with the primary listener — so
/// it is proven directly rather than by binding a socket, which no test in
/// this slice does (design D7.3/F8, stated as a limit).
#[tokio::test]
async fn drain_signal_is_pending_before_send_and_resolves_after() {
    let (tx, rx) = watch::channel(false);
    let mut future = Box::pin(drain_signal(rx));

    assert!(
        futures::poll!(&mut future).is_pending(),
        "drain_signal must not resolve before the channel observes true"
    );

    tx.send(true).expect("receiver still live");

    future.await;
}
