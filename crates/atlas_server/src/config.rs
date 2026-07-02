use base64::{Engine, engine::general_purpose::STANDARD};
use std::fmt;

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
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required".to_string())?;

        let root_password = std::env::var("ATLAS_ROOT_PASSWORD").ok();

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

        Ok(Self {
            database_url,
            root_password,
            anchor_interval,
            webhook_enc_key,
            dispatcher,
            allow_private_webhook_targets,
            rate_limit,
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
    std::env::var(var)
        .map(|s| matches!(s.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(default)
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
    fn dispatcher_config_has_sane_defaults() {
        let cfg = DispatcherConfig::default();
        assert_eq!(cfg.poll_interval_ms, 1_000);
        assert_eq!(cfg.max_attempts, 5);
        assert_eq!(cfg.delivery_timeout_ms, 10_000);
        assert_eq!(cfg.max_concurrent, 16);
        assert_eq!(cfg.batch_size, 32);
        assert_eq!(cfg.lease_secs, 30);
    }
}
