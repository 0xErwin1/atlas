//! `platform` component configuration (registry declaration:
//! `ConfigDeclaration::new("PlatformConfig", "ATLAS_PLATFORM_", true)`,
//! `reg5.rs`).
//!
//! Owns the database connection/pool, per-principal rate limiting, graceful
//! shutdown, the HTTP bind port, session/cookie/idempotency lifetimes, and
//! (per the orchestrator's E11-S1 decision) the two per-request build/URL
//! variables `routes/health.rs` and `routes/users.rs` read.

use std::net::SocketAddr;

use atlas_core::config::{ComponentConfig, ConfigError, EnvSource};
use atlas_postgres::PostgresConfig;

use super::{RateLimitConfig, load_rate_limit_config, read_env};

/// The environment variable naming the Prometheus exposition bind (design
/// D4, E11-S7 D-S7-1). A *new* variable, so it takes `platform`'s declared
/// `ATLAS_PLATFORM_` prefix (`shell-requirements.md` S3) rather than a bare
/// V1-style name. `pub(crate)`: read only by this module's own tests and by
/// `ConfigError::invalid`'s message; nothing outside the crate names it.
pub(crate) const METRICS_BIND_VAR: &str = "ATLAS_PLATFORM_METRICS_BIND";

/// Prometheus exposition bind for the second, independently bound listener
/// (design D4, D-S7-1). `None` on [`PlatformConfig::metrics`] means the
/// feature is off: no listener binds and `/metrics` is unreachable on every
/// port (INV-METRICS-DISABLED-BY-DEFAULT).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricsConfig {
    pub bind: SocketAddr,
}

/// Typed configuration owned by the `platform` component.
#[derive(Debug)] // safe: `postgres.database_url` is `Secret<String>`.
pub struct PlatformConfig {
    pub postgres: PostgresConfig,
    pub rate_limit: RateLimitConfig,
    /// Upper bound, in seconds, on the post-signal graceful drain before the
    /// process forces termination. Guards against long-lived SSE streams
    /// blocking shutdown indefinitely.
    pub shutdown_timeout_secs: u64,
    pub port: u16,
    /// Build identifier surfaced by `/api/meta` (`ATLAS_BUILD`).
    pub build: Option<String>,
    /// Public base URL surfaced by `/api/meta` and used in activation links
    /// (`ATLAS_SERVER_URL`).
    pub server_url: Option<String>,
    pub cookie_secure: bool,
    pub session_ttl_hours: i64,
    pub session_max_ttl_hours: i64,
    /// Response-retention TTL for the `Idempotency-Key` store.
    pub idempotency_retention_hours: i64,
    /// The second listener's bind, or `None` when the feature is off
    /// (design D4, E11-S7 D-S7-1).
    pub metrics: Option<MetricsConfig>,
}

impl ComponentConfig for PlatformConfig {
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
        let postgres = PostgresConfig::from_env(source)?;
        let rate_limit = load_rate_limit_config(source);
        let shutdown_timeout_secs = read_env(source, "ATLAS_SHUTDOWN_TIMEOUT_SECS", 20);
        let port = crate::startup::read_port(source, 8080);
        let build = source.get("ATLAS_BUILD");
        let server_url = source.get("ATLAS_SERVER_URL");
        let cookie_secure = crate::state::resolve_cookie_secure(source);
        let session_ttl_hours = read_env(source, "ATLAS_SESSION_TTL_HOURS", 168);
        let session_max_ttl_hours = read_env(source, "ATLAS_SESSION_MAX_TTL_HOURS", 720);
        let idempotency_retention_hours = read_env(source, "ATLAS_IDEMPOTENCY_RETENTION_HOURS", 24);
        let metrics = parse_metrics_config(source)?;

        let config = Self {
            postgres,
            rate_limit,
            shutdown_timeout_secs,
            port,
            build,
            server_url,
            cookie_secure,
            session_ttl_hours,
            session_max_ttl_hours,
            idempotency_retention_hours,
            metrics,
        };

        config.validate()?;

        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        self.postgres.validate()
    }
}

/// Resolves [`PlatformConfig::metrics`] from [`METRICS_BIND_VAR`] (design
/// D4). Unset or empty ⇒ `None`. A full `SocketAddr` string
/// (`127.0.0.1:9464`) is accepted as-is; a bare `u16` port binds
/// `127.0.0.1:<port>` (D-S7-1's "default loopback"). Any other value is a
/// startup error naming the variable, never a lenient fallback — this
/// departs from [`read_env`]'s house behavior on purpose: a typo'd bind
/// that silently falls back either opens a port the operator did not
/// choose, or silently disables the feature they did choose, and both are
/// worse than refusing to start.
fn parse_metrics_config(source: &dyn EnvSource) -> Result<Option<MetricsConfig>, ConfigError> {
    let Some(raw) = source
        .get(METRICS_BIND_VAR)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };

    if let Ok(bind) = raw.parse::<SocketAddr>() {
        return Ok(Some(MetricsConfig { bind }));
    }

    if let Ok(port) = raw.parse::<u16>() {
        return Ok(Some(MetricsConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], port)),
        }));
    }

    Err(ConfigError::invalid(
        METRICS_BIND_VAR,
        "must be a socket address (e.g. `127.0.0.1:9464`) or a bare port number",
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl EnvSource {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn from_env_composes_postgres_rate_limit_and_process_level_fields() {
        let cfg = PlatformConfig::from_env(&env(&[
            ("DATABASE_URL", "postgres://set-value/db"),
            ("ATLAS_PORT", "9090"),
            ("ATLAS_BUILD", "2026.09.01+abc123"),
            ("ATLAS_SERVER_URL", "https://atlas.example.test"),
            ("ATLAS_SHUTDOWN_TIMEOUT_SECS", "45"),
        ]))
        .expect("expected Ok");

        assert_eq!(
            cfg.postgres.database_url.expose(),
            "postgres://set-value/db"
        );
        assert_eq!(cfg.port, 9090);
        assert_eq!(cfg.build.as_deref(), Some("2026.09.01+abc123"));
        assert_eq!(
            cfg.server_url.as_deref(),
            Some("https://atlas.example.test")
        );
        assert_eq!(cfg.shutdown_timeout_secs, 45);
        assert!(cfg.rate_limit.enabled);
        assert_eq!(cfg.session_ttl_hours, 168);
        assert_eq!(cfg.session_max_ttl_hours, 720);
        assert_eq!(cfg.idempotency_retention_hours, 24);
        assert!(cfg.cookie_secure);
    }

    #[test]
    fn from_env_defaults_process_level_fields_when_unset() {
        let cfg = PlatformConfig::from_env(&env(&[("DATABASE_URL", "postgres://set-value/db")]))
            .expect("expected Ok");

        assert_eq!(cfg.port, 8080);
        assert_eq!(cfg.build, None);
        assert_eq!(cfg.server_url, None);
        assert_eq!(cfg.shutdown_timeout_secs, 20);
    }

    #[test]
    fn from_env_missing_database_url_is_missing_error() {
        let error = PlatformConfig::from_env(&env(&[])).expect_err("expected Err");

        assert_eq!(error, ConfigError::missing("DATABASE_URL"));
    }

    #[test]
    fn metrics_is_none_when_unset() {
        let cfg = PlatformConfig::from_env(&env(&[("DATABASE_URL", "postgres://set-value/db")]))
            .expect("expected Ok");

        assert_eq!(cfg.metrics, None);
    }

    #[test]
    fn metrics_parses_a_full_socket_addr() {
        let cfg = PlatformConfig::from_env(&env(&[
            ("DATABASE_URL", "postgres://set-value/db"),
            (METRICS_BIND_VAR, "127.0.0.1:9464"),
        ]))
        .expect("expected Ok");

        assert_eq!(
            cfg.metrics,
            Some(MetricsConfig {
                bind: "127.0.0.1:9464".parse().unwrap(),
            })
        );
    }

    #[test]
    fn metrics_binds_loopback_from_a_bare_port() {
        let cfg = PlatformConfig::from_env(&env(&[
            ("DATABASE_URL", "postgres://set-value/db"),
            (METRICS_BIND_VAR, "9464"),
        ]))
        .expect("expected Ok");

        assert_eq!(
            cfg.metrics,
            Some(MetricsConfig {
                bind: "127.0.0.1:9464".parse().unwrap(),
            })
        );
    }

    #[test]
    fn metrics_rejects_a_garbage_value_naming_the_variable() {
        let error = PlatformConfig::from_env(&env(&[
            ("DATABASE_URL", "postgres://set-value/db"),
            (METRICS_BIND_VAR, "not-a-bind-address"),
        ]))
        .expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::invalid(
                METRICS_BIND_VAR,
                "must be a socket address (e.g. `127.0.0.1:9464`) or a bare port number",
            )
        );
    }
}
