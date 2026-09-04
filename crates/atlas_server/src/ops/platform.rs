//! `platform`'s diagnostics implementer (design D1): a `SELECT 1` on the
//! shared pool, moved verbatim from `routes/health.rs`'s pre-S3a `ready`
//! handler.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_core::capabilities::{Health, HealthStatus, Readiness, ReadinessStatus};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use super::db_error_kind;

/// `health()` never touches the pool (SHELL-OPS-1): the process answering
/// HTTP at all is the only signal. `readiness()` issues one `SELECT 1` on
/// the shared pool, mapping any error to `NotReady` with a fixed reason —
/// never the raw `sea_orm::DbErr::Display`, which can carry the connection
/// string (INV-NO-SECRET).
pub struct PlatformDiagnostics {
    db: Arc<DatabaseConnection>,
}

impl PlatformDiagnostics {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

impl Health for PlatformDiagnostics {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for PlatformDiagnostics {
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
                    target: "ops.platform",
                    event = "readiness_failed",
                    error_kind = db_error_kind(&error),
                    "platform readiness probe failed: database unreachable"
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

    #[test]
    fn health_is_ok_synchronously_with_no_await_reachable() {
        // The absence of `.await` on this call is the assertion: `Health::health`
        // is not `async` (SHELL-OPS-1), so this line would not compile if it were.
        let diagnostics = PlatformDiagnostics::new(Arc::new(unreachable_db()));
        assert_eq!(diagnostics.health(), HealthStatus::Ok);
    }

    /// A `DatabaseConnection` that is never actually queried by this test:
    /// `health()` performs no I/O, so constructing one only proves the type
    /// compiles without a live database. The `readiness()` SELECT-1 path is
    /// covered by the container-backed `tests/api_readiness.rs` suite instead,
    /// since a real pool is required to exercise it meaningfully.
    fn unreachable_db() -> DatabaseConnection {
        DatabaseConnection::default()
    }

    #[tokio::test]
    async fn readiness_maps_a_pool_error_to_not_ready_without_a_raw_error_string() {
        let diagnostics = PlatformDiagnostics::new(Arc::new(unreachable_db()));

        let status = diagnostics.readiness().await;

        assert_eq!(
            status,
            ReadinessStatus::NotReady {
                reason: "database is unreachable".to_string()
            }
        );
    }
}
