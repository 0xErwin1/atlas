//! The env-loadable, self-validating configuration contract components
//! implement (SHELL-CFG-2). `ComponentConfig` is a compile-time convention:
//! `atlas_core` never links it to `ConfigDeclaration.struct_name` at
//! runtime.

use super::{ConfigError, EnvSource};

/// Loads and validates one component's configuration from an [`EnvSource`].
///
/// `from_env` requires `Self: Sized`, which makes `ComponentConfig`
/// intentionally not object-safe: it has no receiver to dispatch through, so
/// `dyn ComponentConfig` cannot be named. Callers depend on it only through a
/// generic parameter or a concrete type.
///
/// Implementers MUST call [`ComponentConfig::validate`] before returning
/// `Ok` from `from_env`. This invariant is enforced per implementation, not
/// by the trait: a nested sub-config typically loads its children with their
/// own `from_env`, then runs its own `validate` for cross-field invariants.
pub trait ComponentConfig: Sized {
    /// Loads configuration values from `source`, returning a validated
    /// instance or the first [`ConfigError`] encountered.
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError>;

    /// Checks cross-field invariants that cannot be expressed while parsing
    /// individual values. The default implementation accepts any instance.
    fn validate(&self) -> Result<(), ConfigError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EnvSource;

    #[derive(Debug)]
    struct Demo {
        name: String,
    }

    impl ComponentConfig for Demo {
        fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
            let name = source
                .get("DEMO_NAME")
                .ok_or_else(|| ConfigError::missing("DEMO_NAME"))?;

            Ok(Self { name })
        }
    }

    #[derive(Debug)]
    struct Port {
        value: u16,
    }

    impl ComponentConfig for Port {
        fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
            let raw = source
                .get("DEMO_PORT")
                .ok_or_else(|| ConfigError::missing("DEMO_PORT"))?;
            let value = raw
                .parse::<u16>()
                .map_err(|_| ConfigError::invalid("DEMO_PORT", "must be a valid u16"))?;

            Ok(Self { value })
        }
    }

    #[derive(Debug)]
    struct Pool {
        min_connections: u32,
        max_connections: u32,
    }

    impl ComponentConfig for Pool {
        fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
            let min_connections = source
                .get("POOL_MIN_CONNECTIONS")
                .ok_or_else(|| ConfigError::missing("POOL_MIN_CONNECTIONS"))?
                .parse::<u32>()
                .map_err(|_| ConfigError::invalid("POOL_MIN_CONNECTIONS", "must be a valid u32"))?;
            let max_connections = source
                .get("POOL_MAX_CONNECTIONS")
                .ok_or_else(|| ConfigError::missing("POOL_MAX_CONNECTIONS"))?
                .parse::<u32>()
                .map_err(|_| ConfigError::invalid("POOL_MAX_CONNECTIONS", "must be a valid u32"))?;

            let config = Self {
                min_connections,
                max_connections,
            };
            config.validate()?;

            Ok(config)
        }

        fn validate(&self) -> Result<(), ConfigError> {
            if self.min_connections > self.max_connections {
                return Err(ConfigError::composition(
                    "min_connections must not exceed max_connections",
                ));
            }

            Ok(())
        }
    }

    #[derive(Debug)]
    struct Server {
        pool: Pool,
    }

    impl ComponentConfig for Server {
        fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
            let pool = Pool::from_env(source)?;

            Ok(Self { pool })
        }

        fn validate(&self) -> Result<(), ConfigError> {
            self.pool.validate()
        }
    }

    #[test]
    fn demo_config_loads_from_env_source_happy_path() {
        let source = |k: &str| {
            if k == "DEMO_NAME" {
                Some("atlas".into())
            } else {
                None
            }
        };

        let demo = Demo::from_env(&source).expect("expected Ok");

        assert_eq!(demo.name, "atlas");
    }

    #[test]
    fn demo_config_missing_key_returns_missing_error() {
        let source = |_: &str| None;

        let error = Demo::from_env(&source).expect_err("expected Err");

        assert_eq!(error, ConfigError::missing("DEMO_NAME"));
    }

    #[test]
    fn port_config_loads_from_env_source_happy_path() {
        let source = |k: &str| {
            if k == "DEMO_PORT" {
                Some("8080".into())
            } else {
                None
            }
        };

        let port = Port::from_env(&source).expect("expected Ok");

        assert_eq!(port.value, 8080);
    }

    #[test]
    fn demo_config_unparsable_value_returns_invalid_error() {
        let source = |k: &str| {
            if k == "DEMO_PORT" {
                Some("not-a-port".into())
            } else {
                None
            }
        };

        let error = Port::from_env(&source).expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::invalid("DEMO_PORT", "must be a valid u16")
        );
        assert!(!error.to_string().contains("not-a-port"));
    }

    #[test]
    fn nested_sub_config_composes_via_validate() {
        let source = |k: &str| match k {
            "POOL_MIN_CONNECTIONS" => Some("10".into()),
            "POOL_MAX_CONNECTIONS" => Some("5".into()),
            _ => None,
        };

        let error = Server::from_env(&source).expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::composition("min_connections must not exceed max_connections")
        );
    }

    #[test]
    fn default_validate_is_a_noop() {
        let demo = Demo {
            name: "atlas".into(),
        };

        assert_eq!(demo.validate(), Ok(()));
    }
}
