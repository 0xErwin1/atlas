use sea_orm::DatabaseConnection;
use std::sync::Arc;

/// Shared application state injected into every route handler.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<DatabaseConnection>,
    pub session_ttl_hours: i64,
    pub session_max_ttl_hours: i64,
    pub cookie_secure: bool,
}

impl AppState {
    pub fn new(db: DatabaseConnection) -> Self {
        let session_ttl_hours = std::env::var("ATLAS_SESSION_TTL_HOURS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(168); // 7 days

        let session_max_ttl_hours = std::env::var("ATLAS_SESSION_MAX_TTL_HOURS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(720); // 30 days

        let cookie_secure = std::env::var("ATLAS_COOKIE_SECURE")
            .map(|s| s != "false" && s != "0")
            .unwrap_or(true);

        Self {
            db: Arc::new(db),
            session_ttl_hours,
            session_max_ttl_hours,
            cookie_secure,
        }
    }

    /// Creates a test-mode state with reduced session TTLs and cookie_secure=false.
    pub fn for_test(db: DatabaseConnection) -> Self {
        Self {
            db: Arc::new(db),
            session_ttl_hours: 24,
            session_max_ttl_hours: 72,
            cookie_secure: false,
        }
    }
}
