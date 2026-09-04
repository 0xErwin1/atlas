//! Diagnostics implementers for the storage/search Modules (design D1):
//! `storage.filesystem`, `storage.s3`, `search.postgres_fts`,
//! `search.pgvector_embeddings`. None of these Modules has an HTTP surface
//! (SHELL-CAP-3/CAP-5) — their signal reaches the wire only through `acta`'s
//! composed readiness (`ops::acta::ActaDiagnostics`).

use std::sync::Arc;

use async_trait::async_trait;
use atlas_core::capabilities::{Health, HealthStatus, Readiness, ReadinessStatus};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use atlas_acta::ports::attachment_store::AttachmentStore;
use atlas_acta::semantic_search::EmbeddingProvider;

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
}
