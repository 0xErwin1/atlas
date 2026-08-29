use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::registry::ComponentId;

/// The result of a component's `/health` check. Never returned as an
/// `Err` — a failed internal check degrades the status instead (SHELL-OPS-1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HealthStatus {
    /// The component is fully operational.
    Ok,
    /// The component is operational but impaired.
    Degraded {
        /// Why the component is degraded.
        reason: String,
    },
    /// The component cannot serve requests.
    Down {
        /// Why the component is down.
        reason: String,
    },
}

/// The result of a component's `/ready` check. Never returned as an `Err`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReadinessStatus {
    /// The component is ready to serve traffic.
    Ready,
    /// The component is not yet ready.
    NotReady {
        /// Why the component is not ready.
        reason: String,
    },
}

/// The severity of a `DoctorFinding`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The finding is informational and requires no action.
    Info,
    /// The finding should be reviewed but does not block operation.
    Warning,
    /// The finding blocks correct operation.
    Critical,
}

/// One structured diagnostic finding reported by `Doctor::doctor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoctorFinding {
    /// The component this finding concerns.
    pub component: ComponentId,
    /// How severe the finding is.
    pub severity: Severity,
    /// What was found.
    pub finding: String,
    /// The recommended remedy.
    pub action: String,
}

/// Reports whether a component is operational, never failing (SHELL-OPS-1).
#[async_trait]
pub trait Health: Send + Sync {
    /// Returns the current health status.
    async fn health(&self) -> HealthStatus;
}

/// Reports whether a component is ready to serve traffic, never failing.
#[async_trait]
pub trait Readiness: Send + Sync {
    /// Returns the current readiness status.
    async fn readiness(&self) -> ReadinessStatus;
}

/// Reports structured diagnostic findings for a component, never failing.
#[async_trait]
pub trait Doctor: Send + Sync {
    /// Returns every current diagnostic finding.
    async fn doctor(&self) -> Vec<DoctorFinding>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::test_support::block_on;

    #[test]
    fn health_status_serde_round_trips_with_exact_json() {
        let status = HealthStatus::Degraded {
            reason: "cache miss rate high".to_string(),
        };
        let json = serde_json::to_string(&status).expect("serialize");

        assert_eq!(
            json,
            r#"{"status":"degraded","reason":"cache miss rate high"}"#
        );

        let parsed: HealthStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, status);
    }

    #[test]
    fn readiness_status_serde_round_trips_with_exact_json() {
        let status = ReadinessStatus::NotReady {
            reason: "warming up".to_string(),
        };
        let json = serde_json::to_string(&status).expect("serialize");

        assert_eq!(json, r#"{"status":"not_ready","reason":"warming up"}"#);

        let parsed: ReadinessStatus = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, status);
    }

    #[test]
    fn doctor_finding_shape_matches_spec() {
        let finding = DoctorFinding {
            component: ComponentId::new("storage.filesystem").expect("valid component id"),
            severity: Severity::Warning,
            finding: "disk usage above 80%".to_string(),
            action: "provision more storage".to_string(),
        };

        assert_eq!(finding.component.as_str(), "storage.filesystem");
        assert_eq!(finding.severity, Severity::Warning);
        assert_eq!(finding.finding, "disk usage above 80%");
        assert_eq!(finding.action, "provision more storage");
    }

    struct StubComponent;

    #[async_trait]
    impl Health for StubComponent {
        async fn health(&self) -> HealthStatus {
            HealthStatus::Down {
                reason: "internal check failed".to_string(),
            }
        }
    }

    #[async_trait]
    impl Readiness for StubComponent {
        async fn readiness(&self) -> ReadinessStatus {
            ReadinessStatus::Ready
        }
    }

    #[async_trait]
    impl Doctor for StubComponent {
        async fn doctor(&self) -> Vec<DoctorFinding> {
            vec![
                DoctorFinding {
                    component: ComponentId::new("storage.filesystem").expect("valid component id"),
                    severity: Severity::Warning,
                    finding: "disk usage above 80%".to_string(),
                    action: "provision more storage".to_string(),
                },
                DoctorFinding {
                    component: ComponentId::new("storage.filesystem").expect("valid component id"),
                    severity: Severity::Critical,
                    finding: "disk full".to_string(),
                    action: "free disk space immediately".to_string(),
                },
            ]
        }
    }

    #[test]
    fn health_readiness_doctor_are_object_safe() {
        let _: Option<Box<dyn Health>> = None;
        let _: Option<Box<dyn Readiness>> = None;
        let _: Option<Box<dyn Doctor>> = None;
    }

    #[test]
    fn health_degrades_instead_of_erroring_on_internal_failure() {
        let component: Box<dyn Health> = Box::new(StubComponent);

        let status = block_on(component.health());

        assert_eq!(
            status,
            HealthStatus::Down {
                reason: "internal check failed".to_string()
            }
        );
    }

    #[test]
    fn doctor_reports_one_warning_and_one_critical_finding() {
        let component: Box<dyn Doctor> = Box::new(StubComponent);

        let findings = block_on(component.doctor());

        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert_eq!(findings[1].severity, Severity::Critical);
    }
}
