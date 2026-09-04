//! `TokioDeadline`: the `atlas_server`-side implementation of
//! `atlas_core::ops::readiness::ReadinessDeadline` (design D3), enforcing
//! root `/ready`'s per-component timeout with `tokio::time::timeout`.
//! `atlas_core` owns no timer (S2's runtime-free orchestrator decision); this
//! is the one place the bound actually runs.

use std::time::Duration;

use async_trait::async_trait;
use atlas_core::capabilities::{Readiness, ReadinessStatus};
use atlas_core::ops::readiness::ReadinessDeadline;
use atlas_core::registry::ComponentId;

/// Bounds each component's readiness probe to `per_component`. An elapsed
/// timer maps to `None` (never a hang, never an `Err`): `aggregate_readiness`
/// turns that into a fixed `NotReady` reason on the `atlas_core` side.
pub struct TokioDeadline {
    pub per_component: Duration,
}

#[async_trait]
impl ReadinessDeadline for TokioDeadline {
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

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
