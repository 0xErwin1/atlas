use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::registry::{ComponentId, WorkerId};

/// A worker's own lifecycle state, reported through `WorkerStateHandle`. The
/// supervisor's own `Starting` transition is not added (E11-S3b design D3):
/// with no readiness barrier, a worker marks `Running` as its first act, so
/// `Starting` would never be observable long enough to read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkerState {
    /// The worker is running normally.
    Running,
    /// The worker has stopped, without error.
    Stopped,
    /// The worker stopped because of an error.
    Failed {
        /// Why the worker failed.
        cause: String,
    },
    /// The worker never ran because a required capability is not configured
    /// (E11-S3b design D3), e.g. the semantic index worker with no
    /// embedding provider. Distinct from `Failed`: an absent optional
    /// capability is not a degradation (SHELL-OPS-5).
    Inactive {
        /// Why the worker is inactive.
        reason: String,
    },
}

impl WorkerState {
    fn describe(&self) -> String {
        match self {
            WorkerState::Running => "running".to_string(),
            WorkerState::Stopped => "stopped".to_string(),
            WorkerState::Failed { cause } => format!("failed ({cause})"),
            WorkerState::Inactive { reason } => format!("inactive ({reason})"),
        }
    }
}

/// A worker's current state, plus the identity a component's `Readiness`
/// implementation needs to compose it into `component_readiness`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerStatus {
    pub id: WorkerId,
    pub component: ComponentId,
    pub critical: bool,
    pub state: WorkerState,
    pub last_heartbeat: Option<DateTime<Utc>>,
}

/// A handle a worker updates and diagnostics reads. `std::sync::Mutex`, not
/// an async mutex: every critical section is a field write with no `await`
/// held across it (E11-S2 design D3).
#[derive(Clone)]
pub struct WorkerStateHandle(Arc<Mutex<WorkerStatus>>);

impl WorkerStateHandle {
    /// Creates a handle starting in `WorkerState::Stopped`, for `id` on
    /// `component`, with the criticality it was declared with.
    pub fn new(id: WorkerId, component: ComponentId, critical: bool) -> Self {
        Self(Arc::new(Mutex::new(WorkerStatus {
            id,
            component,
            critical,
            state: WorkerState::Stopped,
            last_heartbeat: None,
        })))
    }

    pub fn running(&self) {
        self.set_state(WorkerState::Running);
    }

    pub fn stopped(&self) {
        self.set_state(WorkerState::Stopped);
    }

    pub fn failed(&self, cause: impl Into<String>) {
        self.set_state(WorkerState::Failed {
            cause: cause.into(),
        });
    }

    pub fn inactive(&self, reason: impl Into<String>) {
        self.set_state(WorkerState::Inactive {
            reason: reason.into(),
        });
    }

    pub fn heartbeat(&self, at: DateTime<Utc>) {
        self.lock().last_heartbeat = Some(at);
    }

    pub fn status(&self) -> WorkerStatus {
        self.lock().clone()
    }

    fn set_state(&self, state: WorkerState) {
        self.lock().state = state;
    }

    /// Locks the shared status, recovering the guard on poison rather than
    /// panicking: a worker reporting its own state must never bring down an
    /// unrelated caller reading it (SHELL-OPS-1's "never failing" contract
    /// extends to this handle).
    fn lock(&self) -> std::sync::MutexGuard<'_, WorkerStatus> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Composes a component's worker statuses into a readiness cause
/// (SHELL-OPS-5): the first `critical: true` worker not in `Running` state
/// names the reason; a non-critical worker's state never affects the
/// result. A component's own `Readiness` implementation calls this and maps
/// `Some(reason)` to `ReadinessStatus::NotReady { reason }`.
pub fn component_readiness(statuses: &[WorkerStatus]) -> Option<String> {
    statuses
        .iter()
        .find(|status| status.critical && status.state != WorkerState::Running)
        .map(|status| {
            format!(
                "critical worker `{}` is {}",
                status.id,
                status.state.describe()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{Readiness, ReadinessStatus};
    use async_trait::async_trait;

    fn worker_id(value: &str) -> WorkerId {
        WorkerId::new(value).expect("valid worker id")
    }

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn worker_status(id: &str, critical: bool, state: WorkerState) -> WorkerStatus {
        WorkerStatus {
            id: worker_id(id),
            component: component("acta"),
            critical,
            state,
            last_heartbeat: None,
        }
    }

    #[test]
    fn worker_state_serde_round_trips_with_exact_json() {
        let state = WorkerState::Failed {
            cause: "panicked".to_string(),
        };
        let json = serde_json::to_string(&state).expect("serialize");

        assert_eq!(json, r#"{"state":"failed","cause":"panicked"}"#);

        let parsed: WorkerState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, state);
    }

    #[test]
    fn worker_state_has_exactly_running_stopped_failed_and_inactive() {
        let running = serde_json::to_string(&WorkerState::Running).expect("serialize");
        let stopped = serde_json::to_string(&WorkerState::Stopped).expect("serialize");

        assert_eq!(running, r#"{"state":"running"}"#);
        assert_eq!(stopped, r#"{"state":"stopped"}"#);
    }

    #[test]
    fn worker_state_inactive_round_trips_with_exact_json_in_the_same_style_as_failed() {
        let state = WorkerState::Inactive {
            reason: "no embedding provider configured".to_string(),
        };
        let json = serde_json::to_string(&state).expect("serialize");

        assert_eq!(
            json,
            r#"{"state":"inactive","reason":"no embedding provider configured"}"#
        );

        let parsed: WorkerState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, state);
    }

    #[test]
    fn worker_state_handle_reports_running_then_failed_then_heartbeat() {
        let handle = WorkerStateHandle::new(worker_id("acta.reindex"), component("acta"), true);

        handle.running();
        assert_eq!(handle.status().state, WorkerState::Running);

        handle.failed("panicked");
        assert_eq!(
            handle.status().state,
            WorkerState::Failed {
                cause: "panicked".to_string()
            }
        );

        let now = Utc::now();
        handle.heartbeat(now);
        assert_eq!(handle.status().last_heartbeat, Some(now));
    }

    #[test]
    fn worker_state_handle_inactive_transitions_the_handle() {
        let handle = WorkerStateHandle::new(
            worker_id("search.pgvector_embeddings.index_worker"),
            component("search.pgvector_embeddings"),
            false,
        );

        handle.inactive("no embedding provider configured");

        assert_eq!(
            handle.status().state,
            WorkerState::Inactive {
                reason: "no embedding provider configured".to_string()
            }
        );
    }

    struct ComposedReadiness {
        statuses: Vec<WorkerStatus>,
    }

    #[async_trait]
    impl Readiness for ComposedReadiness {
        async fn readiness(&self) -> ReadinessStatus {
            match component_readiness(&self.statuses) {
                Some(reason) => ReadinessStatus::NotReady { reason },
                None => ReadinessStatus::Ready,
            }
        }
    }

    #[test]
    fn a_stopped_critical_worker_makes_its_component_not_ready() {
        let component = ComposedReadiness {
            statuses: vec![worker_status(
                "acta.webhook_dispatcher",
                true,
                WorkerState::Failed {
                    cause: "panicked".to_string(),
                },
            )],
        };

        let status = crate::ops::test_support::block_on(component.readiness());

        match status {
            ReadinessStatus::NotReady { reason } => {
                assert!(reason.contains("acta.webhook_dispatcher"));
                assert!(reason.contains("panicked"));
            }
            ReadinessStatus::Ready => panic!("expected NotReady"),
        }
    }

    #[test]
    fn a_stopped_non_critical_worker_does_not_affect_readiness() {
        let component = ComposedReadiness {
            statuses: vec![worker_status(
                "acta.presence_sweeper",
                false,
                WorkerState::Stopped,
            )],
        };

        let status = crate::ops::test_support::block_on(component.readiness());

        assert_eq!(status, ReadinessStatus::Ready);
    }
}
