//! Neutral, dyn-safe primitives for env-backed component configuration
//! (SHELL-CFG-1, SHELL-CFG-2, SHELL-CFG-3): loading, a validation contract,
//! typed errors that never echo values, and secret redaction. This module
//! defines no concrete config struct; components implement `ComponentConfig`
//! for their own types.

mod component;
mod env_source;
mod error;
mod secret;

pub use component::ComponentConfig;
pub use env_source::{EnvSource, ProcessEnv};
pub use error::ConfigError;
pub use secret::Secret;

#[cfg(test)]
mod tests {
    #[test]
    fn config_module_is_reachable_from_crate_root() {
        use crate::config::{ComponentConfig, ConfigError, EnvSource, ProcessEnv, Secret};

        fn assert_reachable<T: ComponentConfig>() {}
        assert_reachable::<NoopConfig>();

        let _ = ConfigError::missing("X");
        let _ = ProcessEnv;
        let _: Box<dyn EnvSource> = Box::new(|_: &str| -> Option<String> { None });
        let _ = Secret::new("x");
    }

    #[derive(Debug)]
    struct NoopConfig;

    impl super::ComponentConfig for NoopConfig {
        fn from_env(_source: &dyn super::EnvSource) -> Result<Self, super::ConfigError> {
            Ok(Self)
        }
    }
}
