//! `custos`'s diagnostics implementer (design D1): database reachability on
//! the same shared pool `AppState.db` already holds — no second connection
//! pool is constructed (INV-BOUNDED-PROBE). Also `CustosDoctor` (design D5):
//! database reachability plus a count of enabled root/system-admin users,
//! on the same shared pool.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_core::capabilities::{
    Doctor, DoctorFinding, Health, HealthStatus, Readiness, ReadinessStatus, Severity,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};

use super::db_error_kind;

pub struct CustosDiagnostics {
    db: Arc<DatabaseConnection>,
}

impl CustosDiagnostics {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

impl Health for CustosDiagnostics {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for CustosDiagnostics {
    async fn readiness(&self) -> ReadinessStatus {
        let probe = self
            .db
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 1",
            ))
            .await;

        match probe {
            Ok(_) => ReadinessStatus::Ready,
            Err(error) => {
                tracing::warn!(
                    target: "ops.custos",
                    event = "readiness_failed",
                    error_kind = db_error_kind(&error),
                    "custos readiness probe failed: database unreachable"
                );
                ReadinessStatus::NotReady {
                    reason: "database is unreachable".to_string(),
                }
            }
        }
    }
}

/// `custos`'s doctor (design D5): database reachability, then a count of
/// enabled `is_root || is_system_admin` users on the same shared connection
/// — no second pool, INV-BOUNDED-DOCTOR.
pub struct CustosDoctor {
    db: Arc<DatabaseConnection>,
}

#[derive(FromQueryResult)]
struct EnabledAdminCount {
    count: i64,
}

impl CustosDoctor {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Doctor for CustosDoctor {
    async fn doctor(&self) -> Vec<DoctorFinding> {
        let component = super::component("custos");
        let mut findings = Vec::new();

        let count = self
            .db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT count(*) AS count FROM custos.users \
                 WHERE (is_root OR is_system_admin) AND disabled_at IS NULL",
            ))
            .await
            .and_then(|row| {
                row.ok_or_else(|| {
                    sea_orm::DbErr::Custom("admin count probe returned no row".to_owned())
                })
                .and_then(|row| EnabledAdminCount::from_query_result(&row, ""))
            });

        match count {
            Ok(EnabledAdminCount { count: 0 }) => {
                findings.push(DoctorFinding {
                    component: component.clone(),
                    severity: Severity::Critical,
                    finding: "no enabled root or system-admin user exists".to_string(),
                    action: "enable at least one root or system-admin user".to_string(),
                });
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    target: "ops.custos",
                    event = "doctor_failed",
                    error_kind = db_error_kind(&error),
                    "custos doctor probe failed: database unreachable"
                );
                findings.push(DoctorFinding {
                    component,
                    severity: Severity::Critical,
                    finding: "database is unreachable".to_string(),
                    action: "restore database connectivity".to_string(),
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unreachable_db() -> DatabaseConnection {
        DatabaseConnection::default()
    }

    #[test]
    fn health_is_ok_synchronously() {
        let diagnostics = CustosDiagnostics::new(Arc::new(unreachable_db()));
        assert_eq!(diagnostics.health(), HealthStatus::Ok);
    }

    #[tokio::test]
    async fn readiness_maps_a_pool_error_to_not_ready_with_a_fixed_reason() {
        let diagnostics = CustosDiagnostics::new(Arc::new(unreachable_db()));

        let status = diagnostics.readiness().await;

        assert_eq!(
            status,
            ReadinessStatus::NotReady {
                reason: "database is unreachable".to_string()
            }
        );
    }

    #[tokio::test]
    async fn doctor_reports_a_critical_finding_with_a_fixed_reason_when_the_database_is_unreachable()
     {
        let doctor = CustosDoctor::new(Arc::new(unreachable_db()));

        let findings = doctor.doctor().await;

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[0].finding, "database is unreachable");
        assert!(
            !findings[0].finding.contains("://"),
            "the finding must never carry a raw connection string"
        );
    }
}
