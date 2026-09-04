use atlas_core::config::EnvSource;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, Statement};
use std::collections::HashSet;
use std::sync::Arc;

use atlas_acta::ports::attachment_store::AttachmentStore;
use atlas_acta::semantic_search::EmbeddingProvider;

use crate::config::{
    AtlasConfig, DEFAULT_MAX_ATTACHMENT_BYTES, DispatcherConfig, EmbeddingProviderKind,
    SearchSemanticConfig, StorageConfig, env_var_nonempty, read_env,
};
use crate::crypto::WebhookCrypto;
use crate::embeddings::{DeterministicEmbeddingProvider, OpenAiCompatibleEmbeddingProvider};
use crate::live::{DEFAULT_HUB_CAPACITY, LiveEventHub};
use crate::middleware::rate_limit::PrincipalRateLimiter;
use crate::persistence::repos::{
    DiskAttachmentStore, PgCommentAttachmentDraftRepo, S3AttachmentStore, S3Config,
};
use crate::presence::PresenceRegistry;
use crate::services::{CommentService, DocumentService, TaskService};

/// The default per-component readiness bound root `/ready` enforces
/// (design D3): small enough that the worst case (every mandatory
/// component elapsing, sequentially) stays well under a typical
/// orchestrator probe interval.
pub const DEFAULT_READINESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Shared application state injected into every route handler.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DatabaseConnection>,
    pub session_ttl_hours: i64,
    pub session_max_ttl_hours: i64,
    /// Response-retention TTL for the `Idempotency-Key` store (D7, `v2-e3-s3`
    /// PR4), alongside `session_ttl_hours`. Default 24h.
    pub idempotency_retention_hours: i64,
    pub cookie_secure: bool,
    /// Build identifier surfaced by `/api/meta` (`ATLAS_BUILD`, `PlatformConfig`).
    pub build: Option<String>,
    /// Public base URL surfaced by `/api/meta` and used in activation links
    /// (`ATLAS_SERVER_URL`, `PlatformConfig`).
    pub server_url: Option<String>,
    pub anchor_interval: u32,
    pub attachments: Arc<dyn AttachmentStore>,
    pub max_attachment_bytes: u64,
    /// Configurable allow-list of upload file extensions. `None` means no
    /// positive extension gate is applied (only the built-in blocklist and the
    /// content allowlist run).
    pub upload_allowed_extensions: Option<Arc<HashSet<String>>>,
    pub webhook_crypto: Arc<WebhookCrypto>,
    pub dispatcher_config: DispatcherConfig,
    pub allow_private_webhook_targets: bool,
    /// Per-principal rate limiter, or `None` when rate limiting is disabled.
    pub rate_limiter: Option<Arc<PrincipalRateLimiter>>,
    /// In-process fan-out hub for live events streamed to clients.
    pub live: LiveEventHub,
    /// In-memory board presence registry (who is currently viewing each board).
    pub presence: Arc<PresenceRegistry>,
    pub embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    pub search_semantic: SearchSemanticConfig,
    /// Bound `Health`/`Readiness` implementers, one per diagnostics-bearing
    /// component plus the active storage/search Modules (E11-S3a design D1).
    /// Built once by [`crate::ops::default_registry`] in both `new` and
    /// `for_test` (D1.2), so the table the server boots with and the table
    /// ~300 tests construct never drift apart.
    pub diagnostics: Arc<crate::ops::DiagnosticsRegistry>,
    /// The per-component bound root `/ready` enforces via `TokioDeadline`
    /// (design D3): small enough that the worst case (every mandatory
    /// component elapsing, sequentially) stays well under a typical
    /// orchestrator probe interval. Overridden in tests via
    /// [`Self::with_readiness_timeout`] to prove the budget path without a
    /// multi-second real wait.
    pub readiness_timeout: std::time::Duration,
}

impl AppState {
    pub async fn new(db: DatabaseConnection, cfg: &AtlasConfig) -> Result<Self, anyhow::Error> {
        let attachments = build_attachment_store(&cfg.modules.storage).await?;
        let webhook_crypto = Arc::new(WebhookCrypto::new(cfg.acta.webhook_enc_key.expose()));

        let rate_limiter = cfg.platform.rate_limit.enabled.then(|| {
            Arc::new(PrincipalRateLimiter::new(
                cfg.platform.rate_limit.per_second,
                cfg.platform.rate_limit.burst,
            ))
        });

        let embedding_provider = build_embedding_provider(&cfg.modules.search_semantic)?;

        let upload_allowed_extensions =
            parse_upload_allowed_extensions(cfg.acta.upload_allowed_extensions.clone());

        let diagnostics = Arc::new(crate::ops::default_registry(
            Arc::new(db.clone()),
            attachments.clone(),
            embedding_provider.clone(),
            &cfg.modules.storage,
        )?);

        Ok(Self {
            db: Arc::new(db),
            session_ttl_hours: cfg.platform.session_ttl_hours,
            session_max_ttl_hours: cfg.platform.session_max_ttl_hours,
            idempotency_retention_hours: cfg.platform.idempotency_retention_hours,
            cookie_secure: cfg.platform.cookie_secure,
            build: cfg.platform.build.clone(),
            server_url: cfg.platform.server_url.clone(),
            anchor_interval: cfg.acta.anchor_interval,
            attachments,
            max_attachment_bytes: cfg.acta.max_attachment_bytes,
            upload_allowed_extensions,
            webhook_crypto,
            dispatcher_config: cfg.acta.dispatcher.clone(),
            allow_private_webhook_targets: cfg.acta.allow_private_webhook_targets,
            rate_limiter,
            live: LiveEventHub::new(DEFAULT_HUB_CAPACITY),
            presence: Arc::new(PresenceRegistry::default()),
            embedding_provider,
            search_semantic: cfg.modules.search_semantic.clone(),
            diagnostics,
            readiness_timeout: DEFAULT_READINESS_TIMEOUT,
        })
    }

    /// Creates a test-mode state with reduced session TTLs and `cookie_secure=false`.
    ///
    /// Uses a freshly generated random AES key so tests do not need
    /// `ATLAS_WEBHOOK_ENC_KEY` set. The attachment store uses a temp directory
    /// unless `ATLAS_ATTACHMENT_ROOT` is set.
    pub async fn for_test(db: DatabaseConnection) -> Result<Self, anyhow::Error> {
        let source = atlas_core::config::ProcessEnv;
        let anchor_interval = read_env::<u32>(&source, "ATLAS_ANCHOR_INTERVAL", 50).max(2);

        let attachment_root =
            env_var_nonempty(&source, "ATLAS_ATTACHMENT_ROOT").unwrap_or_else(|| {
                std::env::temp_dir()
                    .join("atlas-test-attachments")
                    .to_string_lossy()
                    .to_string()
            });

        let attachments: Arc<dyn AttachmentStore> = Arc::new(
            DiskAttachmentStore::new(&attachment_root)
                .await
                .map_err(|e| anyhow::anyhow!("test attachment store: {e:?}"))?,
        );

        let embedding_provider: Option<Arc<dyn EmbeddingProvider>> = Some(Arc::new(
            DeterministicEmbeddingProvider::new("atlas-test-embedding", 1536)?,
        ));

        let db = Arc::new(db);

        let diagnostics = Arc::new(crate::ops::default_registry(
            db.clone(),
            attachments.clone(),
            embedding_provider.clone(),
            &crate::config::StorageConfig::Disk {
                root: attachment_root,
            },
        )?);

        Ok(Self {
            db,
            session_ttl_hours: 24,
            session_max_ttl_hours: 72,
            idempotency_retention_hours: 24,
            cookie_secure: false,
            build: None,
            server_url: None,
            anchor_interval,
            attachments,
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
            upload_allowed_extensions: None,
            webhook_crypto: Arc::new(WebhookCrypto::generate_for_test()),
            dispatcher_config: DispatcherConfig::default(),
            allow_private_webhook_targets: true,
            rate_limiter: None,
            live: LiveEventHub::new(DEFAULT_HUB_CAPACITY),
            presence: Arc::new(PresenceRegistry::default()),
            embedding_provider,
            search_semantic: SearchSemanticConfig::default(),
            diagnostics,
            readiness_timeout: DEFAULT_READINESS_TIMEOUT,
        })
    }

    /// Consumes this state and returns it with the diagnostics table replaced —
    /// the test seam a container test uses to force one component's
    /// `Health`/`Readiness` result via `atlas_core::ops::test_support::FakeDiagnostics`
    /// (design R8): never by poisoning the shared pool, which would fail
    /// every component at once and prove nothing about aggregation.
    pub fn with_diagnostics(mut self, diagnostics: crate::ops::DiagnosticsRegistry) -> Self {
        self.diagnostics = Arc::new(diagnostics);
        self
    }

    /// Consumes this state and returns it with the readiness timeout replaced —
    /// the test seam that proves `/ready`'s budget (design D3, T1.28)
    /// without waiting out the real default on every stalling-component
    /// scenario.
    pub fn with_readiness_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.readiness_timeout = timeout;
        self
    }

    pub async fn semantic_search_enabled_now(&self) -> Result<bool, DbErr> {
        if self.embedding_provider.is_none() {
            return Ok(false);
        }

        probe_semantic_search_schema(&self.db).await
    }

    /// Returns a clone of this state with a custom attachment size cap.
    ///
    /// Intended for integration tests that need to trigger the oversize path
    /// without uploading a real 20 MiB body.
    pub fn with_max_attachment_bytes(mut self, cap: u64) -> Self {
        self.max_attachment_bytes = cap;
        self
    }

    /// Returns a clone of this state with a custom upload extension allow-list.
    ///
    /// Intended for integration tests that exercise the positive extension gate
    /// without setting `ATLAS_UPLOAD_ALLOWED_EXTENSIONS` in the process
    /// environment. Each entry is normalized like the env var; an empty iterator
    /// yields `None` (no positive gate).
    pub fn with_upload_allowed_extensions(
        mut self,
        exts: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let set: HashSet<String> = exts
            .into_iter()
            .filter_map(|e| normalize_extension(&e.into()))
            .collect();

        self.upload_allowed_extensions = (!set.is_empty()).then(|| Arc::new(set));
        self
    }

    /// Returns this state with per-principal rate limiting enabled at the given
    /// quota. Intended for integration tests that exercise the 429 path; the
    /// default `for_test` state leaves the limiter disabled so unrelated tests
    /// are never throttled.
    pub fn with_rate_limit(mut self, per_second: u32, burst: u32) -> Self {
        self.rate_limiter = Some(Arc::new(PrincipalRateLimiter::new(per_second, burst)));
        self
    }

    /// Builds a `TaskService` bound to this state's database connection.
    pub fn task_service(&self) -> TaskService {
        TaskService::with_comment_service((*self.db).clone(), self.comment_service())
    }

    /// Builds a `DocumentService` bound to this state's database connection.
    pub fn document_service(&self) -> DocumentService {
        DocumentService::with_comment_service(
            (*self.db).clone(),
            self.anchor_interval,
            self.comment_service(),
        )
    }

    pub fn comment_attachment_draft_repo(&self) -> PgCommentAttachmentDraftRepo {
        PgCommentAttachmentDraftRepo::new((*self.db).clone())
    }

    fn comment_service(&self) -> CommentService {
        CommentService::with_attachment_store((*self.db).clone(), self.attachments.clone())
    }
}

pub(crate) async fn probe_semantic_search_schema(db: &DatabaseConnection) -> Result<bool, DbErr> {
    let row = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector') AS vector_extension, \
             to_regclass('acta.search_embeddings') IS NOT NULL AS embeddings_table",
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("semantic search schema probe returned no row".to_owned()))?;

    let vector_extension: bool = row.try_get("", "vector_extension")?;
    let embeddings_table: bool = row.try_get("", "embeddings_table")?;

    Ok(vector_extension && embeddings_table)
}

/// Builds the semantic embedding provider selected by
/// `cfg.provider`, or `None` when semantic search is disabled.
fn build_embedding_provider(
    cfg: &SearchSemanticConfig,
) -> Result<Option<Arc<dyn EmbeddingProvider>>, anyhow::Error> {
    if !cfg.enabled {
        return Ok(None);
    }

    let provider: Arc<dyn EmbeddingProvider> = match cfg.provider {
        EmbeddingProviderKind::Deterministic => Arc::new(DeterministicEmbeddingProvider::new(
            cfg.model.clone(),
            cfg.dimensions,
        )?),
        EmbeddingProviderKind::OpenAiCompatible => {
            Arc::new(OpenAiCompatibleEmbeddingProvider::new(cfg.clone())?)
        }
    };
    Ok(Some(provider))
}

/// Resolves `ATLAS_COOKIE_SECURE` through `source`, matching V1's exact
/// truthiness rule: any value other than `"false"`/`"0"` (including a set but
/// empty string) counts as secure, and an unset variable defaults to secure.
pub fn resolve_cookie_secure(source: &dyn EnvSource) -> bool {
    source
        .get("ATLAS_COOKIE_SECURE")
        .map(|s| s != "false" && s != "0")
        .unwrap_or(true)
}

/// The resolved attachment-backend choice and its settings, read from
/// `ATLAS_ATTACHMENT_BACKEND` and its dependent variables.
///
/// Split out of [`build_attachment_store`] so each variable's binding is
/// provable — by the by-name environment-binding characterization test
/// (`tests/env_binding.rs`) — without touching the filesystem or building a
/// network client.
#[derive(Debug, PartialEq, Eq)]
pub enum AttachmentBackendChoice {
    Disk {
        root: String,
    },
    S3 {
        bucket: String,
        endpoint: String,
        access_key_id: String,
        secret_access_key: String,
        region: String,
    },
}

/// Reads `ATLAS_ATTACHMENT_BACKEND` and its dependent variables through
/// `source`, without touching the filesystem or a network client.
pub fn resolve_attachment_backend(
    source: &dyn EnvSource,
) -> Result<AttachmentBackendChoice, anyhow::Error> {
    let backend =
        env_var_nonempty(source, "ATLAS_ATTACHMENT_BACKEND").unwrap_or_else(|| "disk".to_string());

    match backend.as_str() {
        "disk" => Ok(AttachmentBackendChoice::Disk {
            root: env_var_nonempty(source, "ATLAS_ATTACHMENT_ROOT")
                .unwrap_or_else(|| "./data/attachments".to_string()),
        }),
        "s3" => Ok(AttachmentBackendChoice::S3 {
            bucket: require_env(source, "ATLAS_S3_BUCKET")?,
            endpoint: require_env(source, "ATLAS_S3_ENDPOINT")?,
            access_key_id: require_env(source, "ATLAS_S3_ACCESS_KEY_ID")?,
            secret_access_key: require_env(source, "ATLAS_S3_SECRET_ACCESS_KEY")?,
            region: env_var_nonempty(source, "ATLAS_S3_REGION")
                .unwrap_or_else(|| "auto".to_string()),
        }),
        other => Err(anyhow::anyhow!(
            "unknown ATLAS_ATTACHMENT_BACKEND '{other}'; expected 'disk' or 's3'"
        )),
    }
}

/// Builds the attachment store selected by `cfg` (the composed
/// `StorageConfig`, `ATLAS_ATTACHMENT_BACKEND`'s discriminator).
///
/// Defaults to the filesystem backend (`disk`) so an unconfigured deployment keeps
/// working. The `s3` backend targets any S3-compatible object store (e.g. Cloudflare
/// R2) and requires its connection variables; `StorageConfig::from_env` already
/// refused startup with a value-free message if one was missing, so this
/// function only builds the client.
async fn build_attachment_store(
    cfg: &StorageConfig,
) -> Result<Arc<dyn AttachmentStore>, anyhow::Error> {
    match cfg {
        StorageConfig::Disk { root } => {
            let store = DiskAttachmentStore::new(root).await.map_err(|e| {
                anyhow::anyhow!("cannot initialise attachment store at {root}: {e:?}")
            })?;

            Ok(Arc::new(store))
        }
        StorageConfig::S3 {
            bucket,
            endpoint,
            access_key_id,
            secret_access_key,
            region,
        } => {
            let config = S3Config {
                bucket: bucket.clone(),
                endpoint: endpoint.clone(),
                access_key_id: access_key_id.clone(),
                secret_access_key: secret_access_key.clone(),
                region: region.clone(),
            };

            let store = S3AttachmentStore::new(config)
                .map_err(|e| anyhow::anyhow!("cannot initialise S3 attachment store: {e:?}"))?;

            Ok(Arc::new(store))
        }
    }
}

/// Reads a required environment variable through `source`, failing with a
/// message that names the variable. The variable's value is never included
/// in the error so a missing secret cannot leak through startup logs.
fn require_env(source: &dyn EnvSource, var: &str) -> Result<String, anyhow::Error> {
    source
        .get(var)
        .ok_or_else(|| anyhow::anyhow!("ATLAS_ATTACHMENT_BACKEND=s3 requires {var} to be set"))
}

/// Normalizes a single extension entry: trims surrounding whitespace, strips a
/// single leading `.`, and lowercases ASCII. Returns `None` for an entry that is
/// empty after normalization.
fn normalize_extension(raw: &str) -> Option<String> {
    let trimmed = raw.trim().strip_prefix('.').unwrap_or(raw.trim());

    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

/// Parses `ATLAS_UPLOAD_ALLOWED_EXTENSIONS` into a normalized set of extensions.
///
/// Splits on `,`, normalizes each entry (trim, strip one leading `.`, lowercase),
/// and drops empties. Returns `None` when the raw value is absent or the
/// resulting set is empty, so an unset or blank value applies no positive gate.
pub fn parse_upload_allowed_extensions(raw: Option<String>) -> Option<Arc<HashSet<String>>> {
    let raw = raw?;

    let set: HashSet<String> = raw.split(',').filter_map(normalize_extension).collect();

    (!set.is_empty()).then(|| Arc::new(set))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_interval_floor_clamps_1_to_2() {
        let raw: u32 = 1;
        let effective = raw.max(2);
        assert_eq!(effective, 2, "interval of 1 must be clamped to floor of 2");
    }

    #[test]
    fn parses_and_normalizes_allowed_extensions() {
        let parsed =
            parse_upload_allowed_extensions(Some("PNG, .jpg ,pdf,".to_string())).expect("some set");

        let expected: HashSet<String> = ["png", "jpg", "pdf"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        assert_eq!(*parsed, expected);
    }

    #[test]
    fn empty_or_unset_allowed_extensions_is_none() {
        assert!(parse_upload_allowed_extensions(Some(String::new())).is_none());
        assert!(parse_upload_allowed_extensions(None).is_none());
        assert!(parse_upload_allowed_extensions(Some("   ,  , ".to_string())).is_none());
    }
}
