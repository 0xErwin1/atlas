//! `custos`'s diagnostics implementer (design D1): database reachability on
//! the same shared pool `AppState.db` already holds — no second connection
//! pool is constructed (INV-BOUNDED-PROBE).

use std::sync::Arc;

use async_trait::async_trait;
use atlas_core::capabilities::{Health, HealthStatus, Readiness, ReadinessStatus};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

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
}
