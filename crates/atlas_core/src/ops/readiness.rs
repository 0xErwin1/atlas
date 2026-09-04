use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::capabilities::{Readiness, ReadinessStatus};
use crate::registry::ComponentId;

/// The aggregate result of `aggregate_readiness`: SH2's `{not_ready: [...]}`
/// shape, naming every not-ready mandatory component, not just the first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReadinessReport {
    Ready,
    NotReady { not_ready: Vec<NotReadyComponent> },
}

/// One not-ready mandatory component and the short cause SHELL-OPS-2
/// requires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotReadyComponent {
    pub component: ComponentId,
    pub reason: String,
}

/// A caller-implemented bound on one component's readiness probe.
/// `atlas_core` owns no timer: `atlas_server` implements this port with
/// `tokio::time::timeout` (E11-S3a), and `ops::test_support` supplies
/// immediate-outcome fakes (`NeverElapses`, `AlwaysElapses`).
#[async_trait]
pub trait ReadinessDeadline: Send + Sync {
    /// Runs `probe`'s readiness check for `component` under the caller's
    /// bound. `None` means the bound elapsed before the probe completed.
    async fn bounded(
        &self,
        component: &ComponentId,
        probe: &dyn Readiness,
    ) -> Option<ReadinessStatus>;
}

/// Aggregates readiness across `components`, sequentially, in the order
/// given (the mandatory set the Shell derives via
/// `Registry::readiness_components()`). Never accepts a `Doctor` value
/// (INV-DOCTOR-NOT-READINESS): the hot-path readiness function and the
/// on-demand doctor path stay two separate entry points.
pub async fn aggregate_readiness(
    components: &[(ComponentId, &dyn Readiness)],
    deadline: &dyn ReadinessDeadline,
) -> ReadinessReport {
    let mut not_ready = Vec::new();

    for (component, probe) in components {
        match deadline.bounded(component, *probe).await {
            Some(ReadinessStatus::Ready) => {}
            Some(ReadinessStatus::NotReady { reason }) => {
                not_ready.push(NotReadyComponent {
                    component: component.clone(),
                    reason,
                });
            }
            None => {
                not_ready.push(NotReadyComponent {
                    component: component.clone(),
                    reason: "readiness check exceeded its deadline".to_string(),
                });
            }
        }
    }

    if not_ready.is_empty() {
        ReadinessReport::Ready
    } else {
        ReadinessReport::NotReady { not_ready }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::test_support::{AlwaysElapses, NeverElapses};

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    struct FixedReadiness(ReadinessStatus);

    #[async_trait]
    impl Readiness for FixedReadiness {
        async fn readiness(&self) -> ReadinessStatus {
            self.0.clone()
        }
    }

    #[test]
    fn readiness_report_serde_round_trips_with_exact_json() {
        let report = ReadinessReport::NotReady {
            not_ready: vec![NotReadyComponent {
                component: component("custos"),
                reason: "warming up".to_string(),
            }],
        };
        let json = serde_json::to_string(&report).expect("serialize");

        assert_eq!(
            json,
            r#"{"status":"not_ready","not_ready":[{"component":"custos","reason":"warming up"}]}"#
        );

        let parsed: ReadinessReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, report);
    }

    #[test]
    fn aggregate_readiness_reports_every_not_ready_component_not_just_the_first() {
        let platform = FixedReadiness(ReadinessStatus::Ready);
        let custos = FixedReadiness(ReadinessStatus::NotReady {
            reason: "warming up".to_string(),
        });
        let acta = FixedReadiness(ReadinessStatus::NotReady {
            reason: "waiting on custos".to_string(),
        });

        let components: Vec<(ComponentId, &dyn Readiness)> = vec![
            (component("platform"), &platform),
            (component("custos"), &custos),
            (component("acta"), &acta),
        ];

        let report =
            crate::ops::test_support::block_on(aggregate_readiness(&components, &NeverElapses));

        assert_eq!(
            report,
            ReadinessReport::NotReady {
                not_ready: vec![
                    NotReadyComponent {
                        component: component("custos"),
                        reason: "warming up".to_string()
                    },
                    NotReadyComponent {
                        component: component("acta"),
                        reason: "waiting on custos".to_string()
                    },
                ]
            }
        );
    }

    #[test]
    fn aggregate_readiness_reports_ready_when_every_component_is_ready() {
        let platform = FixedReadiness(ReadinessStatus::Ready);
        let components: Vec<(ComponentId, &dyn Readiness)> =
            vec![(component("platform"), &platform)];

        let report =
            crate::ops::test_support::block_on(aggregate_readiness(&components, &NeverElapses));

        assert_eq!(report, ReadinessReport::Ready);
    }

    #[test]
    fn a_deadline_exceeded_component_is_not_ready_and_the_rest_still_run() {
        let slow = FixedReadiness(ReadinessStatus::Ready);
        let after = FixedReadiness(ReadinessStatus::Ready);
        let components: Vec<(ComponentId, &dyn Readiness)> =
            vec![(component("custos"), &slow), (component("acta"), &after)];

        let report =
            crate::ops::test_support::block_on(aggregate_readiness(&components, &AlwaysElapses));

        assert_eq!(
            report,
            ReadinessReport::NotReady {
                not_ready: vec![
                    NotReadyComponent {
                        component: component("custos"),
                        reason: "readiness check exceeded its deadline".to_string()
                    },
                    NotReadyComponent {
                        component: component("acta"),
                        reason: "readiness check exceeded its deadline".to_string()
                    },
                ]
            },
            "a timed-out component must not block the remaining components from being checked"
        );
    }

    #[test]
    fn aggregate_readiness_never_accepts_a_doctor_value() {
        // Structural assertion (spec Scenario "Doctor aggregation is a
        // distinct entry point from readiness aggregation"): this call site
        // only compiles with `&dyn Readiness` and `&dyn ReadinessDeadline`
        // values. A stray `&dyn Doctor`/`DoctorReport` parameter on
        // `aggregate_readiness` would fail to compile here.
        let platform = FixedReadiness(ReadinessStatus::Ready);
        let components: Vec<(ComponentId, &dyn Readiness)> =
            vec![(component("platform"), &platform)];

        let _report =
            crate::ops::test_support::block_on(aggregate_readiness(&components, &NeverElapses));
    }
}
