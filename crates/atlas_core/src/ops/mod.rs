//! Diagnostics aggregation and worker state reporting (E11-S2/PR2).
//!
//! No type in this module owns a runtime: every timeout crosses the crate
//! boundary as a caller-implemented deadline port (`ReadinessDeadline`,
//! `DoctorDeadline`), and `WorkerStateHandle` is a plain `std::sync::Mutex`
//! updated by whichever runtime the caller drives. The supervisor that
//! actually starts and drains workers (ordered start, bounded reverse drain)
//! lives in `atlas_server::ops` (E11-S3b), built against the contracts here.

pub mod doctor;
pub mod readiness;
pub mod state;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use doctor::{DoctorDeadline, DoctorReport, run_doctor};
pub use readiness::{ReadinessDeadline, ReadinessReport, aggregate_readiness};
pub use state::{WorkerState, WorkerStateHandle, WorkerStatus, component_readiness};

use crate::capabilities::HealthStatus;

/// The process's own health: a constant, argument-free check that never
/// polls a component (SHELL-OPS-1). Distinct from the per-component
/// `Health` trait.
pub fn process_health() -> HealthStatus {
    HealthStatus::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_health_takes_no_component_argument() {
        // The call below is the assertion: `process_health()` compiles with
        // zero arguments, so no `ComponentId`/`Registry` reference can have
        // been threaded through it.
        assert_eq!(process_health(), HealthStatus::Ok);
    }
}
