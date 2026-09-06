//! Per-request metrics, recorded through the `metrics` facade against the
//! same registry-derived lookup the request span uses (E11-S7 design D3,
//! D3.2, D3.3; gate G2).
//!
//! [`request_labels`] is the only place a metric label is constructed, and
//! [`labels_within_allowlist`] is the only checker of the closed label set —
//! both the real assertion and the fabricated-label probe in
//! `tests/metric_labels.rs` go through this one function
//! (INV-METRIC-LABEL-ALLOWLIST).

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use metrics::{counter, histogram};

use crate::observability::route_index::{RouteIndex, RouteTag};

/// The closed set of label keys any metric emitted from this module may
/// carry. `request_labels`'s output and every rejection check are measured
/// against this set, never a literal repeated elsewhere.
pub const METRIC_LABEL_KEYS: &[&str] = &["component", "operation", "status"];

/// Explicit histogram buckets (seconds) for `atlas_http_request_duration_seconds`
/// (design D3.2). Consumed by the second listener's `PrometheusBuilder` setup
/// (E11-S7 PR5); defined here alongside the metric it bounds so the two never
/// drift apart.
pub const REQUEST_DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Counter incremented once per matched request, labelled `component`,
/// `operation`, and the numeric HTTP status code.
pub const REQUESTS_TOTAL: &str = "atlas_http_requests_total";

/// Histogram of request latency in seconds, labelled `component` and
/// `operation` only — status is deliberately excluded (design D3.2) to
/// avoid multiplying the bucket series by status cardinality.
pub const REQUEST_DURATION_SECONDS: &str = "atlas_http_request_duration_seconds";

/// Unlabelled counter incremented on a `MatchedPath`/`RouteIndex` miss, so
/// 404/401 traffic still contributes a signal without ever producing a
/// labelled series outside the registry's closed set.
pub const REQUESTS_UNMATCHED_TOTAL: &str = "atlas_http_requests_unmatched_total";

/// Builds the label set for one matched request. The **only** place a
/// metric label is constructed (design D3.2) — every label value comes
/// from `tag` (registry-derived) or `status` (the response itself), never
/// from the URI, a path parameter, a body, or a principal.
pub fn request_labels(tag: &RouteTag, status: StatusCode) -> [(&'static str, String); 3] {
    [
        ("component", tag.component.to_string()),
        ("operation", tag.operation.to_string()),
        ("status", status.as_u16().to_string()),
    ]
}

/// Whether every label key in `labels` belongs to [`METRIC_LABEL_KEYS`].
/// The single checker used both to prove [`request_labels`]'s output is
/// closed and, by the fabricated-label probe, that a forbidden key is
/// actually rejected rather than the check passing vacuously.
pub fn labels_within_allowlist(labels: &[(&str, String)]) -> bool {
    labels
        .iter()
        .all(|(key, _)| METRIC_LABEL_KEYS.contains(key))
}

/// Records `atlas_http_requests_total` and `atlas_http_request_duration_seconds`
/// for a matched route, or `atlas_http_requests_unmatched_total` for a miss
/// (design D3.3). Applied in `app()` immediately before `apply_layers`, so
/// it reads the same `MatchedPath` extension the request span reads and
/// resolves through the same `RouteIndex` — the trace and the metric cannot
/// disagree about ownership.
pub async fn record_request_metrics(
    State(route_index): State<Arc<RouteIndex>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let matched_template = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_string());

    let tag = matched_template
        .as_deref()
        .and_then(|template| route_index.get(&method, template))
        .cloned();

    let start = Instant::now();
    let response = next.run(request).await;
    let elapsed = start.elapsed();

    match tag {
        Some(tag) => {
            let [component, operation, status] = request_labels(&tag, response.status());

            counter!(
                REQUESTS_TOTAL,
                component.0 => component.1.clone(),
                operation.0 => operation.1.clone(),
                status.0 => status.1,
            )
            .increment(1);

            histogram!(
                REQUEST_DURATION_SECONDS,
                component.0 => component.1,
                operation.0 => operation.1,
            )
            .record(elapsed.as_secs_f64());
        }
        None => {
            counter!(REQUESTS_UNMATCHED_TOTAL).increment(1);
        }
    }

    response
}
