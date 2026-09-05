//! Diagnostics implementers for the storage/search Modules (design D1):
//! `storage.filesystem`, `storage.s3`, `search.postgres_fts`,
//! `search.pgvector_embeddings`. None of these Modules has an HTTP surface
//! (SHELL-CAP-3/CAP-5) — their signal reaches the wire only through `acta`'s
//! composed readiness (`ops::acta::ActaDiagnostics`). Also their four
//! `Doctor` implementers (design D5), reported the same way: each Module has
//! no HTTP surface of its own, so its doctor findings reach
//! `POST /api/v2/platform/doctor` through `doctor_set` naming the Module's
//! own `ComponentId`, independent of `acta`'s doctor.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_core::capabilities::{
    Doctor, DoctorFinding, Health, HealthStatus, Readiness, ReadinessStatus, Severity,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, FromQueryResult, Statement};

use atlas_acta::ports::attachment_store::AttachmentStore;
use atlas_acta::semantic_search::{EmbeddingInput, EmbeddingProvider};

use super::db_error_kind;
use crate::state::probe_semantic_search_schema;

/// The SHA-256 hex digest of the empty input. The S3 probe asks the store
/// whether an object with this digest exists: either `Ok` answer proves the
/// store is reachable and only `Err` means unreachable, so an existing
/// empty attachment under this digest is harmless. Used only for
/// `AttachmentStore::exists`'s bounded HeadObject.
pub const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// `storage.filesystem`'s readiness: the configured root exists and is
/// writable, checked via `std::fs::metadata` and permission bits
/// (orchestrator correction, 2026-09-04, superseding design D1.4's original
/// create+remove-probe-file text) — never a file write on every `/ready`
/// poll.
pub struct DiskStorageDiagnostics {
    root: String,
}

impl DiskStorageDiagnostics {
    pub fn new(root: String) -> Self {
        Self { root }
    }
}

impl Health for DiskStorageDiagnostics {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for DiskStorageDiagnostics {
    async fn readiness(&self) -> ReadinessStatus {
        let root = self.root.clone();

        let metadata = match tokio::task::spawn_blocking(move || std::fs::metadata(root)).await {
            Ok(Ok(metadata)) => metadata,
            Ok(Err(_)) => {
                return ReadinessStatus::NotReady {
                    reason: "storage root does not exist".to_string(),
                };
            }
            Err(_) => {
                return ReadinessStatus::NotReady {
                    reason: "storage root check failed".to_string(),
                };
            }
        };

        if !metadata.is_dir() {
            return ReadinessStatus::NotReady {
                reason: "storage root is not a directory".to_string(),
            };
        }

        if metadata.permissions().readonly() {
            return ReadinessStatus::NotReady {
                reason: "storage root is read-only".to_string(),
            };
        }

        ReadinessStatus::Ready
    }
}

/// `storage.s3`'s readiness: one bounded HeadObject
/// (`AttachmentStore::exists`) against a fixed digest that never collides
/// with real content — the store has no dedicated probe method (design
/// §0.4).
pub struct S3StorageDiagnostics {
    attachments: Arc<dyn AttachmentStore>,
}

impl S3StorageDiagnostics {
    pub fn new(attachments: Arc<dyn AttachmentStore>) -> Self {
        Self { attachments }
    }
}

impl Health for S3StorageDiagnostics {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for S3StorageDiagnostics {
    async fn readiness(&self) -> ReadinessStatus {
        match self.attachments.exists(EMPTY_SHA256).await {
            Ok(_) => ReadinessStatus::Ready,
            Err(_) => {
                tracing::warn!(
                    target: "ops.storage.s3",
                    event = "readiness_failed",
                    "storage.s3 readiness probe failed: object store unreachable"
                );
                ReadinessStatus::NotReady {
                    reason: "object store is unreachable".to_string(),
                }
            }
        }
    }
}

/// `search.postgres_fts`'s readiness: database reachability only, on the
/// same shared pool.
pub struct SearchLexicalDiagnostics {
    db: Arc<DatabaseConnection>,
}

impl SearchLexicalDiagnostics {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

impl Health for SearchLexicalDiagnostics {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for SearchLexicalDiagnostics {
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
                    target: "ops.search.postgres_fts",
                    event = "readiness_failed",
                    error_kind = db_error_kind(&error),
                    "search.postgres_fts readiness probe failed: database unreachable"
                );
                ReadinessStatus::NotReady {
                    reason: "database is unreachable".to_string(),
                }
            }
        }
    }
}

/// `search.pgvector_embeddings`'s readiness: **presence only** (design D1.3)
/// — the provider is configured and the schema is present, never a call to
/// the embeddings endpoint itself (SHELL-OPS-2's "no ejecuta diagnósticos
/// costosos").
pub struct SearchSemanticDiagnostics {
    db: Arc<DatabaseConnection>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl SearchSemanticDiagnostics {
    pub fn new(
        db: Arc<DatabaseConnection>,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Self {
        Self {
            db,
            embedding_provider,
        }
    }
}

impl Health for SearchSemanticDiagnostics {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for SearchSemanticDiagnostics {
    async fn readiness(&self) -> ReadinessStatus {
        if self.embedding_provider.is_none() {
            return ReadinessStatus::NotReady {
                reason: "no embedding provider is configured".to_string(),
            };
        }

        match probe_semantic_search_schema(&self.db).await {
            Ok(true) => ReadinessStatus::Ready,
            Ok(false) => ReadinessStatus::NotReady {
                reason: "semantic search schema is not present".to_string(),
            },
            Err(error) => {
                tracing::warn!(
                    target: "ops.search.pgvector_embeddings",
                    event = "readiness_failed",
                    error_kind = db_error_kind(&error),
                    "search.pgvector_embeddings readiness probe failed: schema check failed"
                );
                ReadinessStatus::NotReady {
                    reason: "semantic search schema readiness check failed".to_string(),
                }
            }
        }
    }
}

/// The prefix of the probe file a disk-storage doctor run creates and
/// removes inside the configured root. Doctor **writes** where S3a's
/// readiness deliberately only stats (design D5) — a metadata check alone
/// cannot prove the root is writable. Each run appends its process id and
/// a fresh UUID so concurrent runs, in one process or across replicas
/// sharing the root, never touch the same path.
const DISK_PROBE_FILE_PREFIX: &str = ".atlas-doctor-probe";

/// Creates and removes one uniquely named probe file under `root`. A probe
/// removed underneath us by another cleaner still proves the root writable.
fn write_and_remove_probe(root: &str) -> std::io::Result<()> {
    let name = format!(
        "{DISK_PROBE_FILE_PREFIX}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    let path = std::path::Path::new(root).join(name);

    std::fs::write(&path, b"atlas doctor probe")?;

    match std::fs::remove_file(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

/// `storage.filesystem`'s doctor (design D5): the root exists, is a
/// directory, and a uniquely named probe file can be created and removed.
pub struct DiskStorageDoctor {
    root: String,
}

impl DiskStorageDoctor {
    pub fn new(root: String) -> Self {
        Self { root }
    }
}

#[async_trait]
impl Doctor for DiskStorageDoctor {
    async fn doctor(&self) -> Vec<DoctorFinding> {
        let component = super::component("storage.filesystem");
        let root = self.root.clone();

        let probe = tokio::task::spawn_blocking(move || write_and_remove_probe(&root)).await;

        match probe {
            Ok(Ok(())) => vec![],
            _ => vec![DoctorFinding {
                component,
                severity: Severity::Critical,
                finding: "the storage root cannot be written to and read back".to_string(),
                action: "check the storage root's existence and write permissions".to_string(),
            }],
        }
    }
}

/// `storage.s3`'s doctor (design D5): one bounded `exists` HeadObject, plus
/// confirming the configured bucket name is not empty. Never echoes the
/// endpoint, key id, or bucket credentials (INV-NO-SECRET).
pub struct S3StorageDoctor {
    attachments: Arc<dyn AttachmentStore>,
    bucket: String,
}

impl S3StorageDoctor {
    pub fn new(attachments: Arc<dyn AttachmentStore>, bucket: String) -> Self {
        Self {
            attachments,
            bucket,
        }
    }
}

#[async_trait]
impl Doctor for S3StorageDoctor {
    async fn doctor(&self) -> Vec<DoctorFinding> {
        let component = super::component("storage.s3");
        let mut findings = Vec::new();

        if self.bucket.trim().is_empty() {
            findings.push(DoctorFinding {
                component: component.clone(),
                severity: Severity::Critical,
                finding: "no bucket is configured".to_string(),
                action: "set the object store's bucket name".to_string(),
            });
        }

        if let Err(_error) = self.attachments.exists(EMPTY_SHA256).await {
            tracing::warn!(
                target: "ops.storage.s3",
                event = "doctor_failed",
                "storage.s3 doctor probe failed: object store unreachable"
            );
            findings.push(DoctorFinding {
                component,
                severity: Severity::Critical,
                finding: "object store is unreachable".to_string(),
                action: "check object store connectivity and credentials".to_string(),
            });
        }

        findings
    }
}

#[derive(FromQueryResult)]
struct FtsIndexPresent {
    present: bool,
}

/// `search.postgres_fts`'s doctor (design D5): database reachability, then
/// whether any index on `acta.documents` covers `search_vector`.
pub struct SearchLexicalDoctor {
    db: Arc<DatabaseConnection>,
}

impl SearchLexicalDoctor {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl Doctor for SearchLexicalDoctor {
    async fn doctor(&self) -> Vec<DoctorFinding> {
        let component = super::component("search.postgres_fts");

        let probe = self
            .db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = 'acta' \
                 AND tablename = 'documents' AND indexdef ILIKE '%search_vector%') AS present",
            ))
            .await
            .and_then(|row| {
                row.ok_or_else(|| {
                    sea_orm::DbErr::Custom("fts index probe returned no row".to_owned())
                })
                .and_then(|row| FtsIndexPresent::from_query_result(&row, ""))
            });

        match probe {
            Ok(FtsIndexPresent { present: true }) => vec![],
            Ok(FtsIndexPresent { present: false }) => vec![DoctorFinding {
                component,
                severity: Severity::Warning,
                finding: "no full-text search index covers `acta.documents`".to_string(),
                action: "run the pending migration that adds the full-text search index"
                    .to_string(),
            }],
            Err(error) => {
                tracing::warn!(
                    target: "ops.search.postgres_fts",
                    event = "doctor_failed",
                    error_kind = db_error_kind(&error),
                    "search.postgres_fts doctor probe failed: database unreachable"
                );
                vec![DoctorFinding {
                    component,
                    severity: Severity::Critical,
                    finding: "database is unreachable".to_string(),
                    action: "restore database connectivity".to_string(),
                }]
            }
        }
    }
}

#[derive(FromQueryResult)]
struct IndexQueueBacklogCount {
    count: i64,
}

/// `search.pgvector_embeddings`'s doctor (design D5): absent provider is one
/// `Info` (SHELL-OPS-5's "absence is not degradation"), never Critical or
/// Warning; a configured provider gets one bounded embed round trip plus a
/// `search_index_queue` backlog count. The backlog query never runs when no
/// provider is configured — nothing drains the queue in that case, so
/// counting it would be noise, not a finding (mirrors S3a's readiness
/// discipline of never running a costly check for an absent capability).
pub struct SearchSemanticDoctor {
    db: Arc<DatabaseConnection>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl SearchSemanticDoctor {
    pub fn new(
        db: Arc<DatabaseConnection>,
        embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    ) -> Self {
        Self {
            db,
            embedding_provider,
        }
    }
}

#[async_trait]
impl Doctor for SearchSemanticDoctor {
    async fn doctor(&self) -> Vec<DoctorFinding> {
        let component = super::component("search.pgvector_embeddings");

        let Some(provider) = &self.embedding_provider else {
            return vec![DoctorFinding {
                component,
                severity: Severity::Info,
                finding: "no embedding provider is configured".to_string(),
                action: "no action required — optional capability not configured".to_string(),
            }];
        };

        let mut findings = Vec::new();

        let probe = provider
            .embed(&[EmbeddingInput {
                text: "atlas doctor probe".to_string(),
            }])
            .await;

        if probe.is_err() {
            tracing::warn!(
                target: "ops.search.pgvector_embeddings",
                event = "doctor_failed",
                "search.pgvector_embeddings doctor probe failed: embedding round trip failed"
            );
            findings.push(DoctorFinding {
                component: component.clone(),
                severity: Severity::Warning,
                finding: "the embedding provider round trip failed".to_string(),
                action: "check the embedding provider's connectivity and credentials".to_string(),
            });
        }

        let backlog = self
            .db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT count(*) AS count FROM acta.search_index_queue",
            ))
            .await
            .and_then(|row| {
                row.ok_or_else(|| {
                    sea_orm::DbErr::Custom("index queue probe returned no row".to_owned())
                })
                .and_then(|row| IndexQueueBacklogCount::from_query_result(&row, ""))
            });

        match backlog {
            Ok(IndexQueueBacklogCount { count }) if count > 0 => {
                findings.push(DoctorFinding {
                    component,
                    severity: Severity::Warning,
                    finding: format!(
                        "{count} resources are waiting in the semantic search index queue"
                    ),
                    action: "check the search index worker for failures".to_string(),
                });
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    target: "ops.search.pgvector_embeddings",
                    event = "doctor_failed",
                    error_kind = db_error_kind(&error),
                    "search.pgvector_embeddings doctor backlog probe failed: database unreachable"
                );
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::error::DomainError;
    use bytes::Bytes;

    #[tokio::test]
    async fn disk_storage_is_ready_when_the_root_exists_and_is_writable() {
        let dir =
            std::env::temp_dir().join(format!("atlas-disk-readiness-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create test root");

        let diagnostics = DiskStorageDiagnostics::new(dir.to_string_lossy().to_string());
        let status = diagnostics.readiness().await;

        assert_eq!(status, ReadinessStatus::Ready);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn disk_storage_is_not_ready_naming_a_missing_root() {
        let missing = std::env::temp_dir().join(format!(
            "atlas-disk-readiness-missing-{}",
            uuid::Uuid::new_v4()
        ));

        let diagnostics = DiskStorageDiagnostics::new(missing.to_string_lossy().to_string());
        let status = diagnostics.readiness().await;

        assert_eq!(
            status,
            ReadinessStatus::NotReady {
                reason: "storage root does not exist".to_string()
            }
        );
    }

    #[tokio::test]
    async fn disk_storage_readiness_never_writes_a_probe_file() {
        // Orchestrator correction (2026-09-04): a metadata/permission check
        // never creates a file. Proven by asserting the directory is empty
        // after the check runs.
        let dir = std::env::temp_dir().join(format!(
            "atlas-disk-readiness-no-write-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create test root");

        let diagnostics = DiskStorageDiagnostics::new(dir.to_string_lossy().to_string());
        diagnostics.readiness().await;

        let entries: Vec<_> = std::fs::read_dir(&dir).expect("read test root").collect();
        assert!(
            entries.is_empty(),
            "readiness must not leave any file behind in the storage root"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    struct StubAttachmentStore {
        exists_result: Result<bool, DomainError>,
        exists_calls: std::sync::Mutex<Vec<String>>,
    }

    impl StubAttachmentStore {
        fn new(exists_result: Result<bool, DomainError>) -> Self {
            Self {
                exists_result,
                exists_calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn exists_calls(&self) -> Vec<String> {
            self.exists_calls.lock().expect("unpoisoned").clone()
        }
    }

    #[async_trait]
    impl AttachmentStore for StubAttachmentStore {
        async fn put(&self, _data: &[u8]) -> Result<String, DomainError> {
            unreachable!("not exercised by this test")
        }

        async fn get(&self, _digest: &str) -> Result<Bytes, DomainError> {
            unreachable!("not exercised by this test")
        }

        async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
            self.exists_calls
                .lock()
                .expect("unpoisoned")
                .push(digest.to_string());

            match &self.exists_result {
                Ok(value) => Ok(*value),
                Err(_) => Err(DomainError::Internal {
                    message: "stub failure".to_string(),
                }),
            }
        }

        async fn delete(&self, _digest: &str) -> Result<(), DomainError> {
            unreachable!("not exercised by this test")
        }
    }

    #[test]
    fn empty_sha256_is_a_well_formed_lowercase_hex_digest() {
        assert_eq!(EMPTY_SHA256.len(), 64);
        assert!(
            EMPTY_SHA256
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
    }

    #[tokio::test]
    async fn s3_storage_readiness_calls_exists_exactly_once_with_the_fixed_digest() {
        let store = Arc::new(StubAttachmentStore::new(Ok(true)));
        let diagnostics = S3StorageDiagnostics::new(store.clone());

        let status = diagnostics.readiness().await;

        assert_eq!(status, ReadinessStatus::Ready);
        assert_eq!(store.exists_calls(), vec![EMPTY_SHA256.to_string()]);
    }

    #[tokio::test]
    async fn s3_storage_is_ready_even_when_an_empty_attachment_exists() {
        let store = Arc::new(StubAttachmentStore::new(Ok(false)));
        let diagnostics = S3StorageDiagnostics::new(store);

        let status = diagnostics.readiness().await;

        assert_eq!(status, ReadinessStatus::Ready);
    }

    #[tokio::test]
    async fn s3_storage_is_not_ready_when_the_head_object_fails() {
        let store = Arc::new(StubAttachmentStore::new(Err(DomainError::Internal {
            message: "unreachable".to_string(),
        })));
        let diagnostics = S3StorageDiagnostics::new(store);

        let status = diagnostics.readiness().await;

        assert_eq!(
            status,
            ReadinessStatus::NotReady {
                reason: "object store is unreachable".to_string()
            }
        );
    }

    #[tokio::test]
    async fn semantic_search_is_not_ready_with_no_embedding_provider_configured() {
        let diagnostics =
            SearchSemanticDiagnostics::new(Arc::new(DatabaseConnection::default()), None);

        let status = diagnostics.readiness().await;

        assert_eq!(
            status,
            ReadinessStatus::NotReady {
                reason: "no embedding provider is configured".to_string()
            }
        );
    }

    #[tokio::test]
    async fn concurrent_disk_doctor_runs_on_one_root_never_collide_and_leave_it_empty() {
        let root = tempfile::tempdir().expect("tempdir");
        let doctor = DiskStorageDoctor::new(root.path().to_string_lossy().into_owned());

        let (first, second, third) =
            tokio::join!(doctor.doctor(), doctor.doctor(), doctor.doctor());

        assert!(
            first.is_empty() && second.is_empty() && third.is_empty(),
            "a healthy root must yield no finding from any concurrent run: {first:?} {second:?} {third:?}"
        );
        let leftovers = std::fs::read_dir(root.path()).expect("read root").count();
        assert_eq!(leftovers, 0, "every probe file must be removed");
    }

    #[tokio::test]
    async fn disk_doctor_reports_a_missing_root_as_critical() {
        let root = tempfile::tempdir().expect("tempdir");
        let missing = root.path().join("absent");
        let doctor = DiskStorageDoctor::new(missing.to_string_lossy().into_owned());

        let findings = doctor.doctor().await;

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
        assert_eq!(findings[0].component.as_str(), "storage.filesystem");
    }
}
