//! `TokioDeadline`: the `atlas_server`-side implementation of
//! `atlas_core::ops::readiness::ReadinessDeadline` (design D3), enforcing
//! root `/ready`'s per-component timeout with `tokio::time::timeout`.
//! `atlas_core` owns no timer (S2's runtime-free orchestrator decision); this
//! is the one place the bound actually runs.

use std::time::Duration;

use async_trait::async_trait;
use atlas_core::capabilities::{Doctor, DoctorFinding, Readiness, ReadinessStatus};
use atlas_core::registry::ComponentId;

/// Bounds each component's readiness probe to `per_component`. An elapsed
/// timer maps to `None` (never a hang, never an `Err`): `aggregate_readiness`
/// turns that into a fixed `NotReady` reason on the `atlas_core` side.
pub struct TokioDeadline {
    pub per_component: Duration,
}

/// `ReadinessDeadline` and `DoctorDeadline` are not imported into this
/// module's own scope (only fully qualified here): both declare a `bounded`
/// method on `TokioDeadline`, and importing both would make every
/// `.bounded(...)` call in the `#[cfg(test)]` submodules below ambiguous
/// (E0034). Each test submodule imports only the one trait it exercises.
#[async_trait]
impl atlas_core::ops::readiness::ReadinessDeadline for TokioDeadline {
    async fn bounded(
        &self,
        _component: &ComponentId,
        probe: &dyn Readiness,
    ) -> Option<ReadinessStatus> {
        tokio::time::timeout(self.per_component, probe.readiness())
            .await
            .ok()
    }
}

/// Also bounds `POST /api/v2/platform/doctor`'s per-component `Doctor` call
/// (E11-S3b design D6.1): the same port, the same `tokio::time::timeout`
/// mechanism, reused verbatim rather than duplicated for the doctor route.
#[async_trait]
impl atlas_core::ops::doctor::DoctorDeadline for TokioDeadline {
    async fn bounded(
        &self,
        _component: &ComponentId,
        probe: &dyn Doctor,
    ) -> Option<Vec<DoctorFinding>> {
        tokio::time::timeout(self.per_component, probe.doctor())
            .await
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use atlas_core::ops::readiness::ReadinessDeadline;

    struct Immediate(ReadinessStatus);

    #[async_trait]
    impl Readiness for Immediate {
        async fn readiness(&self) -> ReadinessStatus {
            self.0.clone()
        }
    }

    struct NeverResolves;

    #[async_trait]
    impl Readiness for NeverResolves {
        async fn readiness(&self) -> ReadinessStatus {
            std::future::pending::<()>().await;
            unreachable!("never resolves")
        }
    }

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    #[tokio::test]
    async fn returns_the_real_outcome_when_the_probe_resolves_inside_the_budget() {
        let deadline = TokioDeadline {
            per_component: Duration::from_secs(5),
        };
        let probe = Immediate(ReadinessStatus::Ready);

        let outcome = deadline.bounded(&component("custos"), &probe).await;

        assert_eq!(outcome, Some(ReadinessStatus::Ready));
    }

    #[tokio::test(start_paused = true)]
    async fn returns_none_when_the_bound_elapses() {
        let deadline = TokioDeadline {
            per_component: Duration::from_millis(10),
        };
        let probe = NeverResolves;

        let outcome = deadline.bounded(&component("custos"), &probe).await;

        assert_eq!(outcome, None);
    }
}

/// Isolated from `tests` above for the same reason noted on the trait impls:
/// this submodule imports only `DoctorDeadline`, never `ReadinessDeadline`.
#[cfg(test)]
mod doctor_tests {
    use super::*;
    use async_trait::async_trait;
    use atlas_core::capabilities::Severity;
    use atlas_core::ops::doctor::DoctorDeadline;

    struct FixedDoctor(Vec<DoctorFinding>);

    #[async_trait]
    impl Doctor for FixedDoctor {
        async fn doctor(&self) -> Vec<DoctorFinding> {
            self.0.clone()
        }
    }

    struct NeverResolves;

    #[async_trait]
    impl Doctor for NeverResolves {
        async fn doctor(&self) -> Vec<DoctorFinding> {
            std::future::pending::<()>().await;
            unreachable!("never resolves")
        }
    }

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn finding(component: &str, text: &str) -> DoctorFinding {
        DoctorFinding {
            component: super::ComponentId::new(component).expect("valid component id"),
            severity: Severity::Warning,
            finding: text.to_string(),
            action: "investigate".to_string(),
        }
    }

    #[tokio::test]
    async fn returns_the_real_findings_when_the_probe_resolves_inside_the_budget() {
        let deadline = TokioDeadline {
            per_component: Duration::from_secs(5),
        };
        let probe = FixedDoctor(vec![finding("custos", "zero enabled admins")]);

        let outcome = deadline.bounded(&component("custos"), &probe).await;

        assert_eq!(
            outcome,
            Some(vec![finding("custos", "zero enabled admins")])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn returns_none_when_the_bound_elapses() {
        let deadline = TokioDeadline {
            per_component: Duration::from_millis(10),
        };
        let probe = NeverResolves;

        let outcome = deadline.bounded(&component("custos"), &probe).await;

        assert_eq!(outcome, None);
    }
}
