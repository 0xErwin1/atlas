//! Cross-cutting observability primitives (E11-S7).
//!
//! `route_index` (PR2) derives, once from the registry, the lookup a request
//! span or metric needs to name the component and operation a matched route
//! belongs to. Later PRs in this slice add request-metrics recording
//! (`metrics`) and the second, independently bound Prometheus exposition
//! listener (`exposition`, `drain_signal`) alongside it.

pub mod route_index;
