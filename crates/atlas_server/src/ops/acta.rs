//! `acta`'s diagnostics implementer (design D1): database reachability plus
//! the active storage Module's (`storage.filesystem` or `storage.s3`) own
//! readiness, composed directly (never through that Module's HTTP surface,
//! since Modules have none).

use std::sync::Arc;

use async_trait::async_trait;
use atlas_core::capabilities::{Health, HealthStatus, Readiness, ReadinessStatus};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use super::db_error_kind;

/// `acta`'s own database probe: one `SELECT 1` on the shared pool, mapping
/// any error to a fixed reason (never the raw `DbErr` `Display`,
/// INV-NO-SECRET).
struct DatabaseProbe {
    db: Arc<DatabaseConnection>,
}

#[async_trait]
impl Readiness for DatabaseProbe {
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
                    target: "ops.acta",
                    event = "readiness_failed",
                    error_kind = db_error_kind(&error),
                    "acta readiness probe failed: database unreachable"
                );
                ReadinessStatus::NotReady {
                    reason: "database is unreachable".to_string(),
                }
            }
        }
    }
}

/// Composes two injected `Readiness` probes: the database first, then the
/// storage Module. The database's own reason passes through; a not-ready
/// Module is reported under a fixed reason and its own reason is logged.
pub struct ActaDiagnostics {
    database: Arc<dyn Readiness>,
    storage: Arc<dyn Readiness>,
}

impl ActaDiagnostics {
    pub fn new(db: Arc<DatabaseConnection>, storage: Arc<dyn Readiness>) -> Self {
        Self {
            database: Arc::new(DatabaseProbe { db }),
            storage,
        }
    }

    #[cfg(test)]
    fn from_probes(database: Arc<dyn Readiness>, storage: Arc<dyn Readiness>) -> Self {
        Self { database, storage }
    }
}

impl Health for ActaDiagnostics {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for ActaDiagnostics {
    async fn readiness(&self) -> ReadinessStatus {
        if let ReadinessStatus::NotReady { reason } = self.database.readiness().await {
            return ReadinessStatus::NotReady { reason };
        }

        match self.storage.readiness().await {
            ReadinessStatus::Ready => ReadinessStatus::Ready,
            ReadinessStatus::NotReady { reason } => {
                tracing::warn!(
                    target: "ops.acta",
                    event = "readiness_failed",
                    module_reason = %reason,
                    "acta readiness probe failed: mandatory storage module is not ready"
                );
                ReadinessStatus::NotReady {
                    reason: "mandatory storage module is not ready".to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedReadiness(ReadinessStatus);

    #[async_trait]
    impl Readiness for FixedReadiness {
        async fn readiness(&self) -> ReadinessStatus {
            self.0.clone()
        }
    }

    fn fixed(status: ReadinessStatus) -> Arc<dyn Readiness> {
        Arc::new(FixedReadiness(status))
    }

    fn not_ready(reason: &str) -> ReadinessStatus {
        ReadinessStatus::NotReady {
            reason: reason.to_string(),
        }
    }

    fn unreachable_db() -> DatabaseConnection {
        DatabaseConnection::default()
    }

    #[test]
    fn health_is_ok_synchronously() {
        let diagnostics =
            ActaDiagnostics::new(Arc::new(unreachable_db()), fixed(ReadinessStatus::Ready));
        assert_eq!(diagnostics.health(), HealthStatus::Ok);
    }

    #[tokio::test]
    async fn readiness_reports_an_unreachable_database_before_consulting_storage() {
        let diagnostics = ActaDiagnostics::new(
            Arc::new(unreachable_db()),
            fixed(not_ready("disk root missing")),
        );

        let status = diagnostics.readiness().await;

        assert_eq!(status, not_ready("database is unreachable"));
    }

    #[tokio::test]
    async fn readiness_names_the_storage_module_when_it_is_not_ready() {
        let diagnostics = ActaDiagnostics::from_probes(
            fixed(ReadinessStatus::Ready),
            fixed(not_ready("disk root missing")),
        );

        let status = diagnostics.readiness().await;

        assert_eq!(status, not_ready("mandatory storage module is not ready"));
    }

    #[tokio::test]
    async fn readiness_is_ready_when_the_database_and_the_storage_module_are_ready() {
        let diagnostics = ActaDiagnostics::from_probes(
            fixed(ReadinessStatus::Ready),
            fixed(ReadinessStatus::Ready),
        );

        let status = diagnostics.readiness().await;

        assert_eq!(status, ReadinessStatus::Ready);
    }
}
