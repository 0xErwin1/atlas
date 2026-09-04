//! `custos` component configuration (registry declaration:
//! `ConfigDeclaration::new("CustosConfig", "ATLAS_CUSTOS_", true)`,
//! `reg5.rs`).

use atlas_core::config::{ComponentConfig, ConfigError, EnvSource, Secret};

use super::env_var_nonempty;

/// Typed configuration owned by the `custos` component.
#[derive(Debug)] // safe: `root_password` is `Secret<String>`.
pub struct CustosConfig {
    /// First-boot root password (`ATLAS_ROOT_PASSWORD`). `None` when bootstrap
    /// should not create a root user from this variable.
    pub root_password: Option<Secret<String>>,
}

impl ComponentConfig for CustosConfig {
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
        Ok(Self {
            root_password: env_var_nonempty(source, "ATLAS_ROOT_PASSWORD").map(Secret::new),
        })
    }
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
    fn from_env_binds_root_password() {
        let cfg = CustosConfig::from_env(&env(&[("ATLAS_ROOT_PASSWORD", "s3cr3t")]))
            .expect("expected Ok");

        assert_eq!(
            cfg.root_password.map(|s| s.expose().clone()),
            Some("s3cr3t".to_string())
        );
    }

    #[test]
    fn from_env_defaults_to_none_when_unset() {
        let cfg = CustosConfig::from_env(&env(&[])).expect("expected Ok");

        assert!(cfg.root_password.is_none());
    }
}
