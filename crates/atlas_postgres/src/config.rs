//! Postgres connection and pool-sizing configuration
//! ([`ComponentConfig`](atlas_core::config::ComponentConfig)).
//!
//! Numeric env vars use V1's lenient parsing: an unparseable value falls back
//! to the default rather than failing startup. Tightening that behavior is
//! deferred to a later slice (E3/SHELL-CFG).

use atlas_core::config::{ComponentConfig, ConfigError, EnvSource, Secret};

/// Postgres connection-pool sizing for the shared `sea_orm` connection.
///
/// The server holds one connection permanently for the `LISTEN` consumer, up
/// to the webhook dispatcher's concurrency limit more for in-flight
/// deliveries, plus request and SSE-auth queries. Left at the driver default
/// (10 connections, no acquire timeout) that baseline can saturate the pool
/// and then block new acquisitions forever, so both bounds are configurable
/// and the acquire wait is capped to fail fast instead of hanging silently.
#[derive(Clone, Debug)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_secs: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 20,
            min_connections: 1,
            acquire_timeout_secs: 10,
        }
    }
}

impl ComponentConfig for PoolConfig {
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
        let defaults = Self::default();

        let config = Self {
            max_connections: read_u32_lenient(
                source,
                "ATLAS_DB_MAX_CONNECTIONS",
                defaults.max_connections,
            ),
            min_connections: read_u32_lenient(
                source,
                "ATLAS_DB_MIN_CONNECTIONS",
                defaults.min_connections,
            ),
            acquire_timeout_secs: read_u64_lenient(
                source,
                "ATLAS_DB_ACQUIRE_TIMEOUT_SECS",
                defaults.acquire_timeout_secs,
            ),
        };

        config.validate()?;

        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.max_connections < 1 {
            return Err(ConfigError::invalid(
                "ATLAS_DB_MAX_CONNECTIONS",
                "must be >= 1",
            ));
        }

        if self.min_connections > self.max_connections {
            return Err(ConfigError::composition(
                "ATLAS_DB_MIN_CONNECTIONS must not exceed ATLAS_DB_MAX_CONNECTIONS",
            ));
        }

        Ok(())
    }
}

/// Postgres connection configuration: the database URL plus pool sizing.
#[derive(Clone, Debug)] // `Debug` is safe: `Secret` redacts `database_url`.
pub struct PostgresConfig {
    pub database_url: Secret<String>,
    pub pool: PoolConfig,
}

impl ComponentConfig for PostgresConfig {
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
        let database_url = source
            .get("DATABASE_URL")
            .ok_or_else(|| ConfigError::missing("DATABASE_URL"))?;

        let config = Self {
            database_url: Secret::new(database_url),
            pool: PoolConfig::from_env(source)?,
        };

        config.validate()?;

        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        self.pool.validate()
    }
}

/// Reads a `u32` env var, falling back to `default` when the variable is
/// absent OR unparseable (V1 lenient semantics — see module docs).
fn read_u32_lenient(source: &dyn EnvSource, key: &str, default: u32) -> u32 {
    source
        .get(key)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// Reads a `u64` env var, falling back to `default` when the variable is
/// absent OR unparseable (V1 lenient semantics — see module docs).
fn read_u64_lenient(source: &dyn EnvSource, key: &str, default: u64) -> u64 {
    source
        .get(key)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

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
    fn pool_config_defaults_match_v1() {
        let cfg = PoolConfig::default();

        assert_eq!(cfg.max_connections, 20);
        assert_eq!(cfg.min_connections, 1);
        assert_eq!(cfg.acquire_timeout_secs, 10);
    }

    #[test]
    fn postgres_config_missing_database_url_is_missing_error() {
        let source = env(&[]);

        let error = PostgresConfig::from_env(&source).expect_err("expected Err");

        assert_eq!(error, ConfigError::missing("DATABASE_URL"));
    }

    #[test]
    fn postgres_config_defaults_pool_when_unset() {
        let source = env(&[("DATABASE_URL", "postgres://user:pw@localhost/db")]);

        let cfg = PostgresConfig::from_env(&source).expect("expected Ok");

        assert_eq!(cfg.pool.max_connections, 20);
        assert_eq!(cfg.pool.min_connections, 1);
        assert_eq!(cfg.pool.acquire_timeout_secs, 10);
    }

    #[test]
    fn pool_config_zero_max_connections_is_invalid() {
        let source = env(&[("ATLAS_DB_MAX_CONNECTIONS", "0")]);

        let error = PoolConfig::from_env(&source).expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::invalid("ATLAS_DB_MAX_CONNECTIONS", "must be >= 1")
        );
    }

    #[test]
    fn pool_config_min_greater_than_max_is_composition_error_without_echoing_values() {
        let source = env(&[
            ("ATLAS_DB_MIN_CONNECTIONS", "50"),
            ("ATLAS_DB_MAX_CONNECTIONS", "10"),
        ]);

        let error = PoolConfig::from_env(&source).expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::composition(
                "ATLAS_DB_MIN_CONNECTIONS must not exceed ATLAS_DB_MAX_CONNECTIONS"
            )
        );
        assert!(!error.to_string().contains('5'));
        assert!(!error.to_string().contains("10"));
    }

    #[test]
    fn pool_config_unparsable_value_falls_back_to_default() {
        let source = env(&[("ATLAS_DB_MAX_CONNECTIONS", "not-a-number")]);

        let cfg = PoolConfig::from_env(&source).expect("expected Ok, lenient parse falls back");

        assert_eq!(cfg.max_connections, PoolConfig::default().max_connections);
    }

    #[test]
    fn postgres_config_debug_never_contains_the_url_password() {
        let cfg = PostgresConfig {
            database_url: Secret::new("postgres://user:supersecretpassword@localhost/db".into()),
            pool: PoolConfig::default(),
        };

        let output = format!("{cfg:?}");

        assert!(!output.contains("supersecretpassword"));
        assert!(output.contains("Secret(<redacted>)"));
    }
}
