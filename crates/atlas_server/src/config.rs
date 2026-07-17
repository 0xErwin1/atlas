use base64::{Engine, engine::general_purpose::STANDARD};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddingProviderKind {
    Deterministic,
    OpenAiCompatible,
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub enabled: bool,
    pub provider: EmbeddingProviderKind,
    pub model: String,
    pub dimensions: usize,
    pub api_key: Option<String>,
    pub base_url: String,
    pub batch_size: usize,
    pub timeout_ms: u64,
    pub retry_attempts: u32,
}

impl EmbeddingConfig {
    pub fn from_env() -> Result<Self, String> {
        Self::from_env_vars(|name| std::env::var(name).ok())
    }

    pub fn from_env_vars<F>(get: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let read = |name: &str| get(name);
        let enabled = read_bool(read("ATLAS_EMBEDDINGS_ENABLED"), false);
        let provider = match read("ATLAS_EMBEDDINGS_PROVIDER")
            .unwrap_or_else(|| "deterministic".to_owned())
            .as_str()
        {
            "deterministic" | "test" => EmbeddingProviderKind::Deterministic,
            "openai_compatible" => EmbeddingProviderKind::OpenAiCompatible,
            other => return Err(format!("unsupported ATLAS_EMBEDDINGS_PROVIDER: {other}")),
        };
        let model =
            read("ATLAS_EMBEDDINGS_MODEL").unwrap_or_else(|| "atlas-test-embedding".to_owned());
        if model.trim().is_empty() {
            return Err("ATLAS_EMBEDDINGS_MODEL must not be empty".to_owned());
        }
        let dimensions = read("ATLAS_EMBEDDINGS_DIMENSIONS")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1536);
        if dimensions == 0 {
            return Err("ATLAS_EMBEDDINGS_DIMENSIONS must be greater than zero".to_owned());
        }
        let config = Self {
            enabled,
            provider,
            model,
            dimensions,
            api_key: read("ATLAS_EMBEDDINGS_API_KEY"),
            base_url: read("ATLAS_EMBEDDINGS_BASE_URL")
                .unwrap_or_else(|| "https://api.openai.com/v1".to_owned()),
            batch_size: read("ATLAS_EMBEDDINGS_BATCH_SIZE")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(64),
            timeout_ms: read("ATLAS_EMBEDDINGS_TIMEOUT_MS")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30_000),
            retry_attempts: read("ATLAS_EMBEDDINGS_RETRY_ATTEMPTS")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(2),
        };
        config.validate_for_provider()?;
        Ok(config)
    }

    pub fn validate_for_provider(&self) -> Result<(), String> {
        if self.dimensions == 0 {
            return Err("ATLAS_EMBEDDINGS_DIMENSIONS must be greater than zero".to_owned());
        }
        if self.model.trim().is_empty() {
            return Err("ATLAS_EMBEDDINGS_MODEL must not be empty".to_owned());
        }
        if matches!(self.provider, EmbeddingProviderKind::OpenAiCompatible)
            && self.enabled
            && self
                .api_key
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(
                "ATLAS_EMBEDDINGS_API_KEY is required for openai_compatible embeddings".to_owned(),
            );
        }
        Ok(())
    }
}

impl fmt::Debug for EmbeddingConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbeddingConfig")
            .field("enabled", &self.enabled)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("base_url", &self.base_url)
            .field("batch_size", &self.batch_size)
            .field("timeout_ms", &self.timeout_ms)
            .field("retry_attempts", &self.retry_attempts)
            .finish()
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: EmbeddingProviderKind::Deterministic,
            model: "atlas-test-embedding".to_owned(),
            dimensions: 1536,
            api_key: None,
            base_url: "https://api.openai.com/v1".to_owned(),
            batch_size: 64,
            timeout_ms: 30_000,
            retry_attempts: 2,
        }
    }
}

/// Runtime parameters for the webhook dispatcher.
#[derive(Clone, Debug)]
pub struct DispatcherConfig {
    /// Milliseconds between successive poll cycles when there is no work.
    pub poll_interval_ms: u64,
    /// Maximum delivery attempts before an outbox row transitions to `dead`.
    pub max_attempts: i32,
    /// Per-delivery HTTP request timeout in milliseconds.
    pub delivery_timeout_ms: u64,
    /// Maximum number of concurrent deliveries per poll cycle.
    pub max_concurrent: usize,
    /// Maximum number of outbox rows to claim per poll cycle.
    pub batch_size: i64,
    /// Seconds a claimed row is locked before the recovery sweep reclaims it.
    pub lease_secs: i64,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1_000,
            max_attempts: 5,
            delivery_timeout_ms: 10_000,
            max_concurrent: 16,
            batch_size: 32,
            lease_secs: 30,
        }
    }
}

/// Postgres connection-pool sizing for the shared `sea_orm` connection.
///
/// The server holds one connection permanently for the `LISTEN` consumer, up to
/// `DispatcherConfig::max_concurrent` more for in-flight webhook deliveries, plus
/// request and SSE-auth queries. Left at the driver default (10 connections, no
/// acquire timeout) that baseline can saturate the pool and then block new
/// acquisitions forever, so both bounds are configurable and the acquire wait is
/// capped to fail fast instead of hanging silently.
#[derive(Clone, Debug)]
pub struct DbPoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_secs: u64,
}

impl Default for DbPoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 20,
            min_connections: 1,
            acquire_timeout_secs: 10,
        }
    }
}

/// Per-principal rate-limit parameters for the authenticated API surface.
///
/// The limiter keys by the authenticated caller (user or API key), not by IP:
/// the abuse vector the limit guards against is programmatic clients (the MCP
/// server and CLI) driving high request volume, and those are always
/// authenticated. `per_second` is the steady-state refill rate and `burst` is
/// the maximum number of requests allowed in an instantaneous spike.
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub per_second: u32,
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            per_second: 20,
            burst: 40,
        }
    }
}

pub struct ServerConfig {
    pub database_url: String,
    pub root_password: Option<String>,
    pub anchor_interval: u32,
    /// Raw 32-byte AES-256-GCM key bytes decoded from `ATLAS_WEBHOOK_ENC_KEY`.
    pub webhook_enc_key: [u8; 32],
    pub dispatcher: DispatcherConfig,
    pub allow_private_webhook_targets: bool,
    pub rate_limit: RateLimitConfig,
    pub db_pool: DbPoolConfig,
    pub embeddings: EmbeddingConfig,
    /// Upper bound, in seconds, on the post-signal graceful drain before the
    /// process forces termination. Guards against long-lived SSE streams
    /// blocking shutdown indefinitely.
    pub shutdown_timeout_secs: u64,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required".to_string())?;

        let root_password = env_var_nonempty("ATLAS_ROOT_PASSWORD");

        let anchor_interval = std::env::var("ATLAS_ANCHOR_INTERVAL")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(50);

        if anchor_interval < 2 {
            return Err(format!(
                "ATLAS_ANCHOR_INTERVAL must be >= 2, got {anchor_interval}"
            ));
        }

        let webhook_enc_key = load_webhook_enc_key()?;
        let dispatcher = load_dispatcher_config();
        let allow_private_webhook_targets =
            read_env_bool("ATLAS_ALLOW_PRIVATE_WEBHOOK_TARGETS", false);
        let rate_limit = load_rate_limit_config();

        let db_pool = load_db_pool_config();
        let embeddings = EmbeddingConfig::from_env()?;

        if db_pool.max_connections < 1 {
            return Err("ATLAS_DB_MAX_CONNECTIONS must be >= 1".to_string());
        }

        if db_pool.min_connections > db_pool.max_connections {
            return Err(format!(
                "ATLAS_DB_MIN_CONNECTIONS ({}) must be <= ATLAS_DB_MAX_CONNECTIONS ({})",
                db_pool.min_connections, db_pool.max_connections
            ));
        }

        let shutdown_timeout_secs = read_env_u64("ATLAS_SHUTDOWN_TIMEOUT_SECS", 20);

        Ok(Self {
            database_url,
            root_password,
            anchor_interval,
            webhook_enc_key,
            dispatcher,
            allow_private_webhook_targets,
            rate_limit,
            db_pool,
            embeddings,
            shutdown_timeout_secs,
        })
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("database_url", &"[REDACTED]")
            .field("root_password", &"[REDACTED]")
            .field("anchor_interval", &self.anchor_interval)
            .field("webhook_enc_key", &"[REDACTED]")
            .field("dispatcher", &self.dispatcher)
            .field(
                "allow_private_webhook_targets",
                &self.allow_private_webhook_targets,
            )
            .field("rate_limit", &self.rate_limit)
            .field("db_pool", &self.db_pool)
            .field("embeddings", &self.embeddings)
            .field("shutdown_timeout_secs", &self.shutdown_timeout_secs)
            .finish()
    }
}

/// Reads and validates `ATLAS_WEBHOOK_ENC_KEY`.
///
/// The variable must contain a standard-base64-encoded value that decodes to
/// exactly 32 bytes. The error message never echoes the value so a misconfigured
/// key cannot leak through startup logs.
fn load_webhook_enc_key() -> Result<[u8; 32], String> {
    let raw = std::env::var("ATLAS_WEBHOOK_ENC_KEY")
        .map_err(|_| "ATLAS_WEBHOOK_ENC_KEY is required but not set".to_string())?;

    let bytes = STANDARD
        .decode(raw.trim())
        .map_err(|e| format!("ATLAS_WEBHOOK_ENC_KEY is not valid base64: {e}"))?;

    bytes.as_slice().try_into().map_err(|_| {
        format!(
            "ATLAS_WEBHOOK_ENC_KEY must decode to exactly 32 bytes, got {}",
            bytes.len()
        )
    })
}

fn load_dispatcher_config() -> DispatcherConfig {
    DispatcherConfig {
        poll_interval_ms: read_env_u64("ATLAS_WEBHOOK_POLL_INTERVAL_MS", 1_000),
        max_attempts: std::env::var("ATLAS_WEBHOOK_MAX_ATTEMPTS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(5),
        delivery_timeout_ms: read_env_u64("ATLAS_WEBHOOK_DELIVERY_TIMEOUT_MS", 10_000),
        max_concurrent: std::env::var("ATLAS_WEBHOOK_MAX_CONCURRENT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(16),
        batch_size: std::env::var("ATLAS_WEBHOOK_BATCH_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(32),
        lease_secs: std::env::var("ATLAS_WEBHOOK_LEASE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30),
    }
}

fn load_db_pool_config() -> DbPoolConfig {
    let defaults = DbPoolConfig::default();
    DbPoolConfig {
        max_connections: read_env_u32("ATLAS_DB_MAX_CONNECTIONS", defaults.max_connections),
        min_connections: read_env_u32("ATLAS_DB_MIN_CONNECTIONS", defaults.min_connections),
        acquire_timeout_secs: read_env_u64(
            "ATLAS_DB_ACQUIRE_TIMEOUT_SECS",
            defaults.acquire_timeout_secs,
        ),
    }
}

fn load_rate_limit_config() -> RateLimitConfig {
    let defaults = RateLimitConfig::default();
    RateLimitConfig {
        enabled: read_env_bool("ATLAS_RATE_LIMIT_ENABLED", defaults.enabled),
        per_second: read_env_u32("ATLAS_RATE_LIMIT_PER_SECOND", defaults.per_second),
        burst: read_env_u32("ATLAS_RATE_LIMIT_BURST", defaults.burst),
    }
}

fn read_env_u32(var: &str, default: u32) -> u32 {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn read_env_u64(var: &str, default: u64) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn read_env_bool(var: &str, default: bool) -> bool {
    read_bool(std::env::var(var).ok(), default)
}

fn read_bool(value: Option<String>, default: bool) -> bool {
    match nonempty(value) {
        Some(s) => matches!(s.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"),
        None => default,
    }
}

/// Collapses a present-but-empty value to `None`.
///
/// A variable that is defined but empty carries no configuration intent, so it
/// must behave exactly like an absent one; `std::env::var` returns `Ok("")` for
/// such values, which would otherwise bypass the caller's default.
fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// Reads an environment variable, treating a present-but-empty value as absent
/// so the caller's default applies instead of a blank string.
pub(crate) fn env_var_nonempty(key: &str) -> Option<String> {
    nonempty(std::env::var(key).ok())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn debug_does_not_expose_database_url_password() {
        let config = ServerConfig {
            database_url: "postgres://user:supersecretpassword@localhost/db".to_string(),
            root_password: Some("rootsecret".to_string()),
            anchor_interval: 50,
            webhook_enc_key: [0xABu8; 32],
            dispatcher: DispatcherConfig::default(),
            allow_private_webhook_targets: false,
            rate_limit: RateLimitConfig::default(),
            db_pool: DbPoolConfig::default(),
            embeddings: EmbeddingConfig::default(),
            shutdown_timeout_secs: 20,
        };

        let output = format!("{config:?}");

        assert!(
            !output.contains("supersecretpassword"),
            "database_url password must not appear in Debug output: {output}"
        );
        assert!(
            !output.contains("rootsecret"),
            "root_password must not appear in Debug output: {output}"
        );
        assert!(
            !output.contains("0xAB") && !output.contains("171"),
            "webhook_enc_key bytes must not appear in Debug output: {output}"
        );
        assert!(
            output.contains("[REDACTED]"),
            "Debug output must contain [REDACTED]: {output}"
        );
    }

    #[test]
    fn rate_limit_config_has_sane_defaults() {
        let cfg = RateLimitConfig::default();
        assert!(cfg.enabled, "rate limiting is enabled by default");
        assert_eq!(cfg.per_second, 20);
        assert_eq!(cfg.burst, 40);
    }

    #[test]
    fn db_pool_config_has_sane_defaults() {
        let cfg = DbPoolConfig::default();
        assert_eq!(cfg.max_connections, 20);
        assert_eq!(cfg.min_connections, 1);
        assert_eq!(cfg.acquire_timeout_secs, 10);
        assert!(
            cfg.min_connections <= cfg.max_connections,
            "min pool size must not exceed max pool size"
        );
    }

    #[test]
    fn dispatcher_config_has_sane_defaults() {
        let cfg = DispatcherConfig::default();
        assert_eq!(cfg.poll_interval_ms, 1_000);
        assert_eq!(cfg.max_attempts, 5);
        assert_eq!(cfg.delivery_timeout_ms, 10_000);
        assert_eq!(cfg.max_concurrent, 16);
        assert_eq!(cfg.batch_size, 32);
        assert_eq!(cfg.lease_secs, 30);
    }

    #[test]
    fn read_bool_treats_empty_as_absent() {
        assert!(read_bool(Some(String::new()), true));
        assert!(!read_bool(Some(String::new()), false));
    }

    #[test]
    fn read_bool_honors_truthy_and_falsy_tokens() {
        assert!(read_bool(Some("true".to_string()), false));
        assert!(!read_bool(Some("false".to_string()), true));
        assert!(read_bool(None, true));
        assert!(!read_bool(None, false));
    }

    #[test]
    fn nonempty_treats_empty_as_absent() {
        assert_eq!(nonempty(Some(String::new())), None);
        assert_eq!(nonempty(Some("x".to_string())), Some("x".to_string()));
        assert_eq!(nonempty(None), None);
    }
}
