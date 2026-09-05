//! `acta`'s diagnostics implementer (design D1): database reachability plus
//! the active storage Module's (`storage.filesystem` or `storage.s3`) own
//! readiness, composed directly (never through that Module's HTTP surface,
//! since Modules have none). Also `ActaDoctor` (design D5, this is SH3):
//! database reachability, the storage Module's readiness, every REG-5
//! worker's state, and a pending-webhook-backlog check.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_core::capabilities::{
    Doctor, DoctorFinding, Health, HealthStatus, Readiness, ReadinessStatus, Severity,
};
use atlas_core::ops::{WorkerState, WorkerStatus};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};

use super::db_error_kind;
use super::workers::WorkerStates;

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

#[derive(FromQueryResult)]
struct PendingBacklogCount {
    count: i64,
}

/// `acta`'s doctor (design D5): database reachability, the storage
/// Module's readiness, every REG-5 worker's state (this is SH3 — a
/// non-`Running` worker becomes a finding regardless of its `critical`
/// flag), and a count of webhook deliveries pending past the dispatcher's
/// poll interval.
pub struct ActaDoctor {
    db: Arc<DatabaseConnection>,
    database: Arc<dyn Readiness>,
    storage: Arc<dyn Readiness>,
    workers: Arc<WorkerStates>,
    dispatcher_poll_interval: Duration,
}

impl ActaDoctor {
    pub fn new(
        db: Arc<DatabaseConnection>,
        storage: Arc<dyn Readiness>,
        workers: Arc<WorkerStates>,
        dispatcher_poll_interval: Duration,
    ) -> Self {
        let database = Arc::new(DatabaseProbe { db: db.clone() });

        Self {
            db,
            database,
            storage,
            workers,
            dispatcher_poll_interval,
        }
    }

    #[cfg(test)]
    fn from_probes(
        db: Arc<DatabaseConnection>,
        database: Arc<dyn Readiness>,
        storage: Arc<dyn Readiness>,
        workers: Arc<WorkerStates>,
        dispatcher_poll_interval: Duration,
    ) -> Self {
        Self {
            db,
            database,
            storage,
            workers,
            dispatcher_poll_interval,
        }
    }

    fn worker_finding(
        component: &atlas_core::registry::ComponentId,
        status: &WorkerStatus,
    ) -> Option<DoctorFinding> {
        match &status.state {
            WorkerState::Running => None,
            WorkerState::Inactive { reason } => Some(DoctorFinding {
                component: component.clone(),
                severity: Severity::Info,
                finding: format!("worker `{}` is inactive: {reason}", status.id),
                action: "no action required — an optional capability is not configured".to_string(),
            }),
            WorkerState::Stopped => Some(DoctorFinding {
                component: component.clone(),
                severity: Severity::Warning,
                finding: format!("worker `{}` is stopped", status.id),
                action: "investigate the worker and restart the process if it does not recover"
                    .to_string(),
            }),
            WorkerState::Failed { cause } => Some(DoctorFinding {
                component: component.clone(),
                severity: Severity::Warning,
                finding: format!("worker `{}` failed: {cause}", status.id),
                action: "investigate the worker and restart the process if it does not recover"
                    .to_string(),
            }),
        }
    }

    async fn backlog_finding(
        &self,
        component: &atlas_core::registry::ComponentId,
    ) -> Option<DoctorFinding> {
        let seconds = self.dispatcher_poll_interval.as_secs_f64().max(0.0);
        let statement = Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM acta.events_outbox \
             WHERE status = 'pending' AND next_attempt_at < now() - make_interval(secs => $1)",
            vec![seconds.into()],
        );

        match self.db.query_one_raw(statement).await.and_then(|row| {
            row.ok_or_else(|| sea_orm::DbErr::Custom("backlog probe returned no row".to_owned()))
                .and_then(|row| PendingBacklogCount::from_query_result(&row, ""))
        }) {
            Ok(PendingBacklogCount { count }) if count > 0 => Some(DoctorFinding {
                component: component.clone(),
                severity: Severity::Warning,
                finding: format!(
                    "{count} webhook deliveries are pending past the dispatcher's poll interval"
                ),
                action: "check the webhook dispatcher worker for failures or a slow target"
                    .to_string(),
            }),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(
                    target: "ops.acta",
                    event = "doctor_failed",
                    error_kind = db_error_kind(&error),
                    "acta doctor backlog probe failed: database unreachable"
                );
                None
            }
        }
    }
}

#[async_trait]
impl Doctor for ActaDoctor {
    async fn doctor(&self) -> Vec<DoctorFinding> {
        let component = super::component("acta");
        let mut findings = Vec::new();

        if let ReadinessStatus::NotReady { reason } = self.database.readiness().await {
            findings.push(DoctorFinding {
                component: component.clone(),
                severity: Severity::Critical,
                finding: reason,
                action: "restore database connectivity".to_string(),
            });
        }

        if let ReadinessStatus::NotReady { reason } = self.storage.readiness().await {
            tracing::warn!(
                target: "ops.acta",
                event = "doctor_failed",
                module_reason = %reason,
                "acta doctor probe failed: mandatory storage module is not ready"
            );
            findings.push(DoctorFinding {
                component: component.clone(),
                severity: Severity::Warning,
                finding: "mandatory storage module is not ready".to_string(),
                action: "investigate the active storage backend".to_string(),
            });
        }

        for status in self.workers.statuses_for(&component) {
            if let Some(finding) = Self::worker_finding(&component, &status) {
                findings.push(finding);
            }
        }

        if let Some(finding) = self.backlog_finding(&component).await {
            findings.push(finding);
        }

        findings
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

    use atlas_core::registry::{
        Api, Authorization, Capabilities, ComponentEntry, ComponentId, ComponentKind,
        ContractVersion, Diagnostics, Experience, Identity, WorkerDeclaration, WorkerId, build,
    };

    fn component_id(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn worker_id(value: &str) -> WorkerId {
        WorkerId::new(value).expect("valid worker id")
    }

    fn acta_registry_with_workers(worker_ids: &[&str]) -> atlas_core::registry::Registry {
        let workers = worker_ids
            .iter()
            .map(|id| WorkerDeclaration {
                id: worker_id(id),
                critical: false,
            })
            .collect();

        let entry = ComponentEntry {
            identity: Identity {
                stable_id: component_id("acta"),
                kind: ComponentKind::Product,
                contract_version: ContractVersion::new(1),
            },
            dependencies: vec![],
            capabilities: Capabilities {
                provided: vec![],
                required_mandatory: vec![],
                required_optional: vec![],
            },
            api: Api {
                namespace: None,
                routes: vec![],
                dto_owner: None,
            },
            authorization: Authorization {
                resource_kinds: vec![],
                actions: vec![],
                role_definitions: vec![],
                principal_sets: vec![],
                provider: false,
            },
            diagnostics: Diagnostics {
                health: false,
                readiness: false,
                doctor: false,
            },
            experience: Experience {
                navigation_providers: vec![],
                context_providers: vec![],
            },
            persistence: None,
            config: None,
            workers,
            satellites: vec![],
        };

        build(vec![entry]).expect("valid registry")
    }

    #[tokio::test]
    async fn doctor_reports_the_database_and_storage_and_worker_and_backlog_findings() {
        let registry = acta_registry_with_workers(&[
            "acta.webhook_dispatcher",
            "acta.attachment_reconciler",
            "acta.presence_sweeper",
        ]);
        let workers = Arc::new(WorkerStates::from_registry(&registry));
        workers
            .handle(&worker_id("acta.webhook_dispatcher"))
            .expect("declared")
            .failed("panicked");
        workers
            .handle(&worker_id("acta.attachment_reconciler"))
            .expect("declared")
            .running();
        workers
            .handle(&worker_id("acta.presence_sweeper"))
            .expect("declared")
            .inactive("no embedding provider configured");

        let doctor = ActaDoctor::from_probes(
            Arc::new(unreachable_db()),
            fixed(not_ready("database is unreachable")),
            fixed(ReadinessStatus::Ready),
            workers,
            Duration::from_secs(60),
        );

        let findings = doctor.doctor().await;

        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[0].finding, "database is unreachable");

        let dispatcher = findings
            .iter()
            .find(|finding| finding.finding.contains("acta.webhook_dispatcher"))
            .expect("the failed worker is named — this is SH3");
        assert_eq!(dispatcher.severity, Severity::Warning);
        assert!(dispatcher.finding.contains("panicked"));

        let sweeper = findings
            .iter()
            .find(|finding| finding.finding.contains("acta.presence_sweeper"))
            .expect("the inactive worker is named");
        assert_eq!(sweeper.severity, Severity::Info);

        assert!(
            !findings
                .iter()
                .any(|finding| finding.finding.contains("acta.attachment_reconciler")),
            "a running worker never produces a finding"
        );

        assert!(
            findings
                .iter()
                .all(|finding| !finding.finding.contains("://")),
            "no finding may carry a raw connection string"
        );
    }

    #[tokio::test]
    async fn doctor_names_the_storage_module_when_it_is_not_ready() {
        let registry = acta_registry_with_workers(&[]);
        let workers = Arc::new(WorkerStates::from_registry(&registry));

        let doctor = ActaDoctor::new(
            Arc::new(unreachable_db()),
            fixed(not_ready("disk root missing")),
            workers,
            Duration::from_secs(60),
        );

        let findings = doctor.doctor().await;

        let storage = findings
            .iter()
            .find(|finding| finding.finding.contains("storage module"))
            .expect("the not-ready storage module is named");
        assert_eq!(storage.severity, Severity::Warning);
        assert!(!storage.finding.contains("disk root missing"));
    }
}
