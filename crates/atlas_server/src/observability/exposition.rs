//! The second, independently bound Prometheus exposition router (design
//! D7.4, E11-S7 D-S7-1). Takes no [`crate::state::AppState`] — it never
//! touches the database — so it is exercised container-free through
//! `tower::ServiceExt::oneshot` (`tests/error_model.rs:19` precedent).
//! `main.rs` mounts it on its own `TcpListener`, never inside [`crate::app`]
//! (INV-METRICS-SECOND-LISTENER-ONLY): `/metrics` is never a registry entry
//! and never reachable from the public listener.

use axum::Router;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use axum::routing::get;
use metrics_exporter_prometheus::PrometheusHandle;

/// Builds the exposition router: exactly one route, `GET /metrics`, which
/// renders `handle`'s current scrape text. Any other path answers 404 —
/// the router declares nothing else.
pub fn router(handle: PrometheusHandle) -> Router {
    Router::new()
        .route("/metrics", get(render_metrics))
        .with_state(handle)
}

/// Renders the installed recorder's current Prometheus text exposition
/// format (`text/plain; version=0.0.4`, the Prometheus exposition-format
/// content type).
async fn render_metrics(State(handle): State<PrometheusHandle>) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4")],
        handle.render(),
    )
}
