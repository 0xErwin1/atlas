use sea_orm::DatabaseConnection;
use std::sync::Arc;

use atlas_domain::AttachmentStore;

use crate::persistence::repos::DiskAttachmentStore;

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
}

impl AppState {
    pub async fn new(db: DatabaseConnection) -> Result<Self, anyhow::Error> {
        let session_ttl_hours = read_env_i64("ATLAS_SESSION_TTL_HOURS", 168);
        let session_max_ttl_hours = read_env_i64("ATLAS_SESSION_MAX_TTL_HOURS", 720);

        let cookie_secure = std::env::var("ATLAS_COOKIE_SECURE")
            .map(|s| s != "false" && s != "0")
            .unwrap_or(true);

        let anchor_interval = read_env_u32("ATLAS_ANCHOR_INTERVAL", 50).max(1);

        let attachment_root = std::env::var("ATLAS_ATTACHMENT_ROOT")
            .unwrap_or_else(|_| "./data/attachments".to_string());

        let attachments = DiskAttachmentStore::new(&attachment_root)
            .await
            .map_err(|e| {
                anyhow::anyhow!("cannot initialise attachment store at {attachment_root}: {e:?}")
            })?;

        Ok(Self {
            db: Arc::new(db),
            session_ttl_hours,
            session_max_ttl_hours,
            cookie_secure,
            anchor_interval,
            attachments: Arc::new(attachments),
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
        })
    }

    /// Creates a test-mode state with reduced session TTLs and `cookie_secure=false`.
    ///
    /// The attachment store uses a temp directory unless `ATLAS_ATTACHMENT_ROOT` is set.
    /// Returns `Err` only if the attachment root directory cannot be created.
    pub async fn for_test(db: DatabaseConnection) -> Result<Self, anyhow::Error> {
        let anchor_interval = read_env_u32("ATLAS_ANCHOR_INTERVAL", 50).max(1);

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
