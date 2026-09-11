//! Cross-cutting observability primitives (E11-S7).
//!
//! `route_index` (PR2) derives, once from the registry, the lookup a request
//! span or metric needs to name the component and operation a matched route
//! belongs to. `metrics` (PR4) records per-request counters and a latency
//! histogram through that same lookup. `exposition` (PR5) is the second,
//! independently bound Prometheus exposition router, and [`drain_signal`]
//! is the one shared shutdown-drain future both listeners await (design D5,
//! D7.3).

pub mod exposition;
pub mod metrics;
pub mod route_index;

use tokio::sync::watch;

/// Resolves once `rx` observes `true` on the shared shutdown-drain channel.
///
/// Extracted from `main.rs`'s original inline closure (design D5, D7.3) so
/// the shared-drain claim (INV-SHARED-DRAIN) is testable without binding a
/// socket: both the primary and the metrics listener's
/// `axum::serve(...).with_graceful_shutdown(...)` pass a clone of the same
/// `watch::Receiver` to this function, never a fresh channel each.
pub async fn drain_signal(mut rx: watch::Receiver<bool>) {
    let _ = rx.wait_for(|drained| *drained).await;
}
