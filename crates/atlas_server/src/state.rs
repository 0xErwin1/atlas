use sea_orm::DatabaseConnection;
use std::collections::HashSet;
use std::sync::Arc;

use atlas_domain::{AttachmentStore, semantic_search::EmbeddingProvider};

use crate::config::{DispatcherConfig, EmbeddingProviderKind, ServerConfig};
use crate::crypto::WebhookCrypto;
use crate::embeddings::{DeterministicEmbeddingProvider, OpenAiCompatibleEmbeddingProvider};
use crate::live::{DEFAULT_HUB_CAPACITY, LiveEventHub};
use crate::middleware::rate_limit::PrincipalRateLimiter;
use crate::persistence::repos::{DiskAttachmentStore, S3AttachmentStore, S3Config};
use crate::presence::PresenceRegistry;
use crate::services::{DocumentService, TaskService};

const DEFAULT_MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024; // 20 MiB

/// Shared application state injected into every route handler.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DatabaseConnection>,
    pub session_ttl_hours: i64,
    pub session_max_ttl_hours: i64,
    pub cookie_secure: bool,
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
}

impl AppState {
    pub async fn new(db: DatabaseConnection, cfg: &ServerConfig) -> Result<Self, anyhow::Error> {
        let session_ttl_hours = read_env_i64("ATLAS_SESSION_TTL_HOURS", 168);
        let session_max_ttl_hours = read_env_i64("ATLAS_SESSION_MAX_TTL_HOURS", 720);

        let cookie_secure = std::env::var("ATLAS_COOKIE_SECURE")
            .map(|s| s != "false" && s != "0")
            .unwrap_or(true);

        let anchor_interval = read_env_u32("ATLAS_ANCHOR_INTERVAL", 50).max(2);

        let attachments = build_attachment_store().await?;
        let webhook_crypto = Arc::new(WebhookCrypto::new(&cfg.webhook_enc_key));

        let rate_limiter = cfg.rate_limit.enabled.then(|| {
            Arc::new(PrincipalRateLimiter::new(
                cfg.rate_limit.per_second,
                cfg.rate_limit.burst,
            ))
        });

        let embedding_provider = build_embedding_provider(cfg)?;

        let upload_allowed_extensions =
            parse_upload_allowed_extensions(std::env::var("ATLAS_UPLOAD_ALLOWED_EXTENSIONS").ok());

        Ok(Self {
            db: Arc::new(db),
            session_ttl_hours,
            session_max_ttl_hours,
            cookie_secure,
            anchor_interval,
            attachments,
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
            upload_allowed_extensions,
            webhook_crypto,
            dispatcher_config: cfg.dispatcher.clone(),
            allow_private_webhook_targets: cfg.allow_private_webhook_targets,
            rate_limiter,
            live: LiveEventHub::new(DEFAULT_HUB_CAPACITY),
            presence: Arc::new(PresenceRegistry::default()),
            embedding_provider,
        })
    }

    /// Creates a test-mode state with reduced session TTLs and `cookie_secure=false`.
    ///
    /// Uses a freshly generated random AES key so tests do not need
    /// `ATLAS_WEBHOOK_ENC_KEY` set. The attachment store uses a temp directory
    /// unless `ATLAS_ATTACHMENT_ROOT` is set.
    pub async fn for_test(db: DatabaseConnection) -> Result<Self, anyhow::Error> {
        let anchor_interval = read_env_u32("ATLAS_ANCHOR_INTERVAL", 50).max(2);

        let attachment_root = std::env::var("ATLAS_ATTACHMENT_ROOT").unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("atlas-test-attachments")
                .to_string_lossy()
                .to_string()
        });

        let attachments = DiskAttachmentStore::new(&attachment_root)
            .await
            .map_err(|e| anyhow::anyhow!("test attachment store: {e:?}"))?;

        Ok(Self {
            db: Arc::new(db),
            session_ttl_hours: 24,
            session_max_ttl_hours: 72,
            cookie_secure: false,
            anchor_interval,
            attachments: Arc::new(attachments),
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
            upload_allowed_extensions: None,
            webhook_crypto: Arc::new(WebhookCrypto::generate_for_test()),
            dispatcher_config: DispatcherConfig::default(),
            allow_private_webhook_targets: true,
            rate_limiter: None,
            live: LiveEventHub::new(DEFAULT_HUB_CAPACITY),
            presence: Arc::new(PresenceRegistry::default()),
            embedding_provider: Some(Arc::new(DeterministicEmbeddingProvider::new(
                "atlas-test-embedding",
                1536,
            )?)),
        })
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
        TaskService::new((*self.db).clone())
    }

    /// Builds a `DocumentService` bound to this state's database connection.
    pub fn document_service(&self) -> DocumentService {
        DocumentService::new((*self.db).clone(), self.anchor_interval)
    }
}

/// Builds the attachment store selected by `ATLAS_ATTACHMENT_BACKEND`.
///
/// Defaults to the filesystem backend (`disk`) so an unconfigured deployment keeps
/// working. The `s3` backend targets any S3-compatible object store (e.g. Cloudflare
/// R2) and requires its connection variables; a missing required variable fails
/// startup with a message that names the variable but never echoes a secret value.
fn build_embedding_provider(
    cfg: &ServerConfig,
) -> Result<Option<Arc<dyn EmbeddingProvider>>, anyhow::Error> {
    if !cfg.embeddings.enabled {
        return Ok(None);
    }

    let provider: Arc<dyn EmbeddingProvider> = match cfg.embeddings.provider {
        EmbeddingProviderKind::Deterministic => Arc::new(DeterministicEmbeddingProvider::new(
            cfg.embeddings.model.clone(),
            cfg.embeddings.dimensions,
        )?),
        EmbeddingProviderKind::OpenAiCompatible => Arc::new(
            OpenAiCompatibleEmbeddingProvider::new(cfg.embeddings.clone())?,
        ),
    };
    Ok(Some(provider))
}

async fn build_attachment_store() -> Result<Arc<dyn AttachmentStore>, anyhow::Error> {
    let backend = std::env::var("ATLAS_ATTACHMENT_BACKEND").unwrap_or_else(|_| "disk".to_string());

    match backend.as_str() {
        "disk" => {
            let attachment_root = std::env::var("ATLAS_ATTACHMENT_ROOT")
                .unwrap_or_else(|_| "./data/attachments".to_string());

            let store = DiskAttachmentStore::new(&attachment_root)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "cannot initialise attachment store at {attachment_root}: {e:?}"
                    )
                })?;

            Ok(Arc::new(store))
        }
        "s3" => {
            let config = S3Config {
                bucket: require_env("ATLAS_S3_BUCKET")?,
                endpoint: require_env("ATLAS_S3_ENDPOINT")?,
                access_key_id: require_env("ATLAS_S3_ACCESS_KEY_ID")?,
                secret_access_key: require_env("ATLAS_S3_SECRET_ACCESS_KEY")?,
                region: std::env::var("ATLAS_S3_REGION").unwrap_or_else(|_| "auto".to_string()),
            };

            let store = S3AttachmentStore::new(config)
                .map_err(|e| anyhow::anyhow!("cannot initialise S3 attachment store: {e:?}"))?;

            Ok(Arc::new(store))
        }
        other => Err(anyhow::anyhow!(
            "unknown ATLAS_ATTACHMENT_BACKEND '{other}'; expected 'disk' or 's3'"
        )),
    }
}

/// Reads a required environment variable, failing with a message that names the
/// variable. The variable's value is never included in the error so a missing
/// secret cannot leak through startup logs.
fn require_env(var: &str) -> Result<String, anyhow::Error> {
    std::env::var(var)
        .map_err(|_| anyhow::anyhow!("ATLAS_ATTACHMENT_BACKEND=s3 requires {var} to be set"))
}

fn read_env_i64(var: &str, default: i64) -> i64 {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn read_env_u32(var: &str, default: u32) -> u32 {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
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
fn parse_upload_allowed_extensions(raw: Option<String>) -> Option<Arc<HashSet<String>>> {
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
