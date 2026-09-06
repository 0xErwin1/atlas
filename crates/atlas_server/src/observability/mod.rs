//! Cross-cutting observability primitives (E11-S7).
//!
//! `route_index` (PR2) derives, once from the registry, the lookup a request
//! span or metric needs to name the component and operation a matched route
//! belongs to. `metrics` (PR4) records per-request counters and a latency
//! histogram through that same lookup. A later PR in this slice adds the
//! second, independently bound Prometheus exposition listener
//! (`exposition`, `drain_signal`) alongside it.

pub mod metrics;
pub mod route_index;
