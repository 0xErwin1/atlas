//! Closed metric label allowlist, `request_labels` as the only constructor
//! (E11-S7 design D3.2, gate G2).
//!
//! `request_labels` is the single place a metric label is built, and
//! `labels_within_allowlist` is the single checker of the closed set. These
//! cases prove, in order (review posture PR4): the checker actually
//! rejects a fabricated forbidden label (T4.5/T4.6), then that
//! `request_labels`'s real output equals `METRIC_LABEL_KEYS` in both
//! directions (T4.3/T4.4), then a container-free behavioral case proving
//! the recorded series carry only those labels end to end.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashSet;
use std::sync::Arc;

use atlas_core::registry::{
    Api, Authorization, Capabilities, ComponentEntry, ComponentId, ComponentKind, ContractVersion,
    Diagnostics, Experience, HttpMethod, Identity, RouteDeclaration, RoutePath, build,
};
use atlas_server::observability::metrics::{
    METRIC_LABEL_KEYS, REQUESTS_TOTAL, REQUESTS_UNMATCHED_TOTAL, labels_within_allowlist,
    record_request_metrics, request_labels,
};
use atlas_server::observability::route_index::{RouteIndex, RouteTag};
use atlas_server::router_audit::{mounted_path, v2_namespace};
use axum::http::StatusCode;
use axum::{
    body::Body,
    http::Request,
    middleware as axum_middleware,
    routing::{Router, get},
};
use metrics_exporter_prometheus::PrometheusBuilder;
use tower::ServiceExt;

fn probe_tag() -> RouteTag {
    RouteTag {
        component: Arc::from("s7_metric_probe"),
        operation: Arc::from("s7_metric_probe_owned"),
    }
}

// ---------------------------------------------------------------------------
// T4.5/T4.6 — the fabricated-label probe runs first (review posture PR4):
// prove the checker can actually detect a violation, not just pass vacuously.
// ---------------------------------------------------------------------------

/// A fabricated label carrying a forbidden key (`workspace_id`, never in
/// `METRIC_LABEL_KEYS`) is rejected by the same allowlist checker
/// `request_labels`'s own test uses. Without this probe, a checker that
/// always returns `true` would pass every other assertion in this file.
#[test]
fn fabricated_forbidden_label_is_rejected() {
    let fabricated: [(&str, String); 1] = [(
        "workspace_id",
        "11111111-1111-1111-1111-111111111111".to_string(),
    )];

    assert!(
        !labels_within_allowlist(&fabricated),
        "a workspace_id label must never pass the allowlist checker"
    );
}

/// The checker accepts a label set drawn only from `METRIC_LABEL_KEYS`, so
/// the rejection above is a real check and not an always-false stub.
#[test]
fn allowlisted_labels_are_accepted() {
    let allowed: [(&str, String); 2] = [
        ("component", "s7_metric_probe".to_string()),
        ("operation", "s7_metric_probe_owned".to_string()),
    ];

    assert!(labels_within_allowlist(&allowed));
}

// ---------------------------------------------------------------------------
// T4.3/T4.4 — request_labels's key set equals METRIC_LABEL_KEYS, both
// directions.
// ---------------------------------------------------------------------------

/// `request_labels`'s emitted key set equals `METRIC_LABEL_KEYS`, checked
/// in both directions: every emitted key is allowlisted, and every
/// allowlisted key is actually emitted. A subset in either direction would
/// silently narrow or widen the closed set this gate protects.
#[test]
fn request_labels_key_set_equals_allowlist_both_directions() {
    let tag = probe_tag();
    let labels = request_labels(&tag, StatusCode::OK);

    assert!(
        labels_within_allowlist(&labels),
        "request_labels must never emit a key outside METRIC_LABEL_KEYS"
    );

    let emitted: HashSet<&str> = labels.iter().map(|(key, _)| *key).collect();
    let allowed: HashSet<&str> = METRIC_LABEL_KEYS.iter().copied().collect();
    assert_eq!(
        emitted, allowed,
        "request_labels's key set must equal METRIC_LABEL_KEYS exactly, not a subset in either direction"
    );
}

/// Label values come from the tag and the status code, never a placeholder.
#[test]
fn request_labels_values_come_from_tag_and_status() {
    let tag = probe_tag();
    let labels = request_labels(&tag, StatusCode::NOT_FOUND);

    let as_map: std::collections::HashMap<&str, String> = labels.into_iter().collect();
    assert_eq!(
        as_map.get("component").map(String::as_str),
        Some("s7_metric_probe")
    );
    assert_eq!(
        as_map.get("operation").map(String::as_str),
        Some("s7_metric_probe_owned")
    );
    assert_eq!(as_map.get("status").map(String::as_str), Some("404"));
}

// ---------------------------------------------------------------------------
// Behavioral case — container-free, real recorder, real request flow.
// ---------------------------------------------------------------------------

async fn ok() -> StatusCode {
    StatusCode::OK
}

/// A synthetic single-route component, mirroring `span_fields.rs`'s
/// anti-vacuity fixture (design D7.1): the route lives only in this index,
/// so a recorded `component`/`operation` pair can only have come from
/// `RouteIndex::from_registry`, not a coincidence with a real registry
/// entry.
fn synthetic_index() -> RouteIndex {
    let entry = ComponentEntry {
        identity: Identity {
            stable_id: ComponentId::new("s7_metric_probe").expect("valid component id"),
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
                operation_id: "s7_metric_probe_owned".to_string(),
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

/// Extracts every label key found on lines belonging to `metric_name` in a
/// Prometheus text-exposition body, so the test can assert the closed set
/// directly against what the recorder actually rendered rather than
/// trusting `request_labels`'s output alone.
///
/// Excludes `le` (histogram bucket upper bound): the Prometheus exposition
/// format adds it automatically to every bucket line of a histogram-typed
/// series, it is never constructed by [`request_labels`] or
/// `record_request_metrics`, and its values are bounded by the fixed
/// bucket list rather than by request data, so it falls outside what
/// INV-METRIC-LABEL-ALLOWLIST governs.
fn label_keys_for_metric(rendered: &str, metric_name: &str) -> HashSet<String> {
    let mut keys = HashSet::new();
    for line in rendered.lines() {
        if !line.starts_with(metric_name) {
            continue;
        }
        let Some(open) = line.find('{') else { continue };
        let Some(close) = line.find('}') else {
            continue;
        };
        for pair in line[open + 1..close].split(',') {
            if let Some((key, _)) = pair.split_once('=')
                && key != "le"
            {
                keys.insert(key.to_string());
            }
        }
    }
    keys
}

/// One matched request and one unmatched request, driven through the real
/// `record_request_metrics` middleware wrapping `test_app_with_router_and_index`
/// (mirroring `app()`'s exact composition: the metrics layer applied
/// immediately before `apply_layers`, design D3.3), against a
/// `PrometheusBuilder::build_recorder()` installed as this thread's default
/// local recorder for the test's duration. The rendered series carry only
/// allowed labels, with values equal to the registry-derived tag and the
/// response's real status code — never a request-derived string.
#[tokio::test]
async fn recorded_series_carry_only_allowlisted_labels() {
    let index = Arc::new(synthetic_index());
    let owned_path = mounted_path(&v2_namespace("s7_metric_probe"), "/owned");

    let inner =
        Router::new()
            .route(&owned_path, get(ok))
            .layer(axum_middleware::from_fn_with_state(
                index.clone(),
                record_request_metrics,
            ));
    let app = atlas_server::test_app_with_router_and_index(inner, index);

    // Without explicit buckets, this exporter renders a histogram as a
    // summary carrying a `quantile` label instead of true buckets — set
    // the same buckets `observability/metrics.rs` documents (design D3.2)
    // so this test observes the real histogram label shape.
    let recorder = PrometheusBuilder::new()
        .set_buckets(atlas_server::observability::metrics::REQUEST_DURATION_BUCKETS)
        .expect("bucket list must be non-empty")
        .build_recorder();
    let handle = recorder.handle();
    let _guard = metrics::set_default_local_recorder(&recorder);

    let matched = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(owned_path.as_str())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(matched.status(), StatusCode::OK);

    let unmatched = app
        .oneshot(
            Request::builder()
                .uri("/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unmatched.status(), StatusCode::NOT_FOUND);

    let rendered = handle.render();

    assert!(
        rendered.contains(&format!(
            "{REQUESTS_TOTAL}{{component=\"s7_metric_probe\",operation=\"s7_metric_probe_owned\",status=\"200\"}} 1"
        )),
        "expected a matched-route series with component/operation/status labels, got:\n{rendered}"
    );
    assert!(
        rendered.contains(&format!("{REQUESTS_UNMATCHED_TOTAL} 1"))
            || rendered.contains(&format!("{REQUESTS_UNMATCHED_TOTAL}{{}} 1")),
        "expected one unlabelled unmatched-request count, got:\n{rendered}"
    );

    let allowed: HashSet<String> = METRIC_LABEL_KEYS.iter().map(|s| s.to_string()).collect();
    for metric in [
        REQUESTS_TOTAL,
        atlas_server::observability::metrics::REQUEST_DURATION_SECONDS,
    ] {
        let found = label_keys_for_metric(&rendered, metric);
        assert!(
            found.is_subset(&allowed),
            "metric {metric} rendered labels outside the allowlist: {found:?}"
        );
    }

    assert!(
        !rendered.contains("workspace_id"),
        "no rendered series may carry a workspace_id label"
    );
}
