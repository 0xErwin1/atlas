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

#[derive(FromQueryResult)]
struct DivergenceCount {
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
                return findings;
            }
        }

        match self.principal_divergence_count().await {
            Ok(count) if count > 0 => {
                findings.push(DoctorFinding {
                    component: component.clone(),
                    severity: Severity::Warning,
                    finding: "users.display_name/disabled_at diverge from custos.principals"
                        .to_string(),
                    action: "re-sync custos.principals so both rows agree".to_string(),
                });
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    target: "ops.custos",
                    event = "doctor_failed",
                    error_kind = db_error_kind(&error),
                    "custos doctor could not probe principals divergence"
                );
            }
        }

        findings
    }
}

impl CustosDoctor {
    /// Counts user rows whose principal mirror disagrees on `kind`,
    /// `display_name` or `deactivated_at`. E4-S1 makes `custos.principals`
    /// the source of truth for those fields, so any drift means a write path
    /// stopped moving both rows in one transaction.
    async fn principal_divergence_count(&self) -> Result<i64, sea_orm::DbErr> {
        let row = self
            .db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT count(*) AS count FROM custos.users u \
                 JOIN custos.principals p ON p.id = u.principal_id \
                 WHERE p.kind <> 'user' \
                    OR p.display_name IS DISTINCT FROM u.display_name \
                    OR p.deactivated_at IS DISTINCT FROM u.disabled_at",
            ))
            .await?
            .ok_or_else(|| sea_orm::DbErr::Custom("divergence probe returned no row".to_owned()))?;

        let counted = DivergenceCount::from_query_result(&row, "")?;
        Ok(counted.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unreachable_db() -> DatabaseConnection {
        DatabaseConnection::default()
    }

    /// Seeds one user and its principal. When `diverge` is true, the
    /// principal's display_name is corrupted afterwards so the two rows
    /// disagree.
    async fn seed_user_and_principal(db: &atlas_test_db::TestDb, diverge: bool) {
        use sea_orm::ConnectionTrait;

        let conn = db.conn();
        conn.execute_unprepared(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at) \
             VALUES ('11111111-1111-1111-1111-111111111111', 'user', 'Ada', NULL)",
        )
        .await
        .expect("seed principal");
        conn.execute_unprepared(
            "INSERT INTO custos.users \
                (id, username, display_name, email, password_hash, is_root, is_system_admin, \
                 disabled_at, activated_at, created_at, updated_at, principal_id) \
             VALUES ('11111111-1111-1111-1111-111111111111', 'ada', 'Ada', NULL, NULL, \
                 false, false, NULL, now(), now(), now(), \
                 '11111111-1111-1111-1111-111111111111')",
        )
        .await
        .expect("seed user");

        if diverge {
            conn.execute_unprepared(
                "UPDATE custos.principals SET display_name = 'Drifted' \
                 WHERE id = '11111111-1111-1111-1111-111111111111'",
            )
            .await
            .expect("diverge principal");
        }
    }

    #[tokio::test]
    async fn doctor_reports_a_warning_when_user_and_principal_rows_diverge() {
        let db = atlas_test_db::TestDb::create()
            .await
            .expect("TestDb::create");
        seed_user_and_principal(&db, true).await;

        let doctor = CustosDoctor::new(Arc::new(db.conn().clone()));
        let findings = doctor.doctor().await;

        assert!(
            findings
                .iter()
                .any(|f| f.severity == Severity::Warning && f.finding.contains("principals")),
            "expected a principals divergence warning, got: {findings:?}"
        );

        db.teardown().await.expect("teardown");
    }

    #[tokio::test]
    async fn doctor_reports_no_principals_warning_when_user_and_principal_rows_agree() {
        let db = atlas_test_db::TestDb::create()
            .await
            .expect("TestDb::create");
        seed_user_and_principal(&db, false).await;

        let doctor = CustosDoctor::new(Arc::new(db.conn().clone()));
        let findings = doctor.doctor().await;

        assert!(
            !findings.iter().any(|f| f.finding.contains("principals")),
            "in-sync rows must not raise a principals finding, got: {findings:?}"
        );

        db.teardown().await.expect("teardown");
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
