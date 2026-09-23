//! `custos` component configuration (registry declaration:
//! `ConfigDeclaration::new("CustosConfig", "ATLAS_CUSTOS_", true)`,
//! `reg5.rs`).

use atlas_core::config::{ComponentConfig, ConfigError, EnvSource, Secret};

use super::env_var_nonempty;

const EXPLICIT_DENY_MODE_VAR: &str = "ATLAS_EXPLICIT_DENY_MODE";

/// How explicit deny rules take part in authorization
/// (`ATLAS_EXPLICIT_DENY_MODE`). Deny rows persist regardless of the mode.
///
/// `Enforced` is a recognized spelling but not an accepted configuration:
/// the runtime does not evaluate deny rules until E7, so loading it fails
/// rather than silently running without the enforcement it promises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DenyModeConfig {
    #[default]
    Disabled,
    Audit,
    Enforced,
}

impl std::str::FromStr for DenyModeConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "disabled" => Ok(DenyModeConfig::Disabled),
            "audit" => Ok(DenyModeConfig::Audit),
            "enforced" => Ok(DenyModeConfig::Enforced),
            other => Err(format!("unknown deny mode: {other}")),
        }
    }
}

/// Typed configuration owned by the `custos` component.
#[derive(Debug)] // safe: `root_password` is `Secret<String>`.
pub struct CustosConfig {
    /// First-boot root password (`ATLAS_ROOT_PASSWORD`). `None` when bootstrap
    /// should not create a root user from this variable.
    pub root_password: Option<Secret<String>>,
    /// Explicit deny mode (`ATLAS_EXPLICIT_DENY_MODE`), `disabled` when unset.
    pub explicit_deny_mode: DenyModeConfig,
}

impl ComponentConfig for CustosConfig {
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
        Ok(Self {
            root_password: env_var_nonempty(source, "ATLAS_ROOT_PASSWORD").map(Secret::new),
            explicit_deny_mode: read_explicit_deny_mode(source)?,
        })
    }
}

fn read_explicit_deny_mode(source: &dyn EnvSource) -> Result<DenyModeConfig, ConfigError> {
    let Some(raw) = env_var_nonempty(source, EXPLICIT_DENY_MODE_VAR) else {
        return Ok(DenyModeConfig::default());
    };

    let mode = raw.parse::<DenyModeConfig>().map_err(|_| {
        ConfigError::invalid(
            EXPLICIT_DENY_MODE_VAR,
            "must be 'disabled', 'audit' or 'enforced'",
        )
    })?;

    if mode == DenyModeConfig::Enforced {
        return Err(ConfigError::invalid(
            EXPLICIT_DENY_MODE_VAR,
            "the runtime does not enforce explicit deny rules until E7; \
             only 'disabled' or 'audit' are accepted until then",
        ));
    }

    Ok(mode)
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

    #[test]
    fn deny_mode_defaults_to_disabled_when_unset() {
        let cfg = CustosConfig::from_env(&env(&[])).expect("expected Ok");

        assert_eq!(cfg.explicit_deny_mode, DenyModeConfig::Disabled);
    }

    #[test]
    fn deny_mode_parses_disabled_and_audit() {
        let disabled = CustosConfig::from_env(&env(&[("ATLAS_EXPLICIT_DENY_MODE", "disabled")]))
            .expect("expected Ok");
        assert_eq!(disabled.explicit_deny_mode, DenyModeConfig::Disabled);

        let audit = CustosConfig::from_env(&env(&[("ATLAS_EXPLICIT_DENY_MODE", "audit")]))
            .expect("expected Ok");
        assert_eq!(audit.explicit_deny_mode, DenyModeConfig::Audit);
    }

    #[test]
    fn deny_mode_enforced_is_rejected_until_e7() {
        let error = CustosConfig::from_env(&env(&[("ATLAS_EXPLICIT_DENY_MODE", "enforced")]))
            .expect_err("enforced must not load");

        assert!(
            matches!(&error, ConfigError::Invalid { name, .. } if name == "ATLAS_EXPLICIT_DENY_MODE"),
            "got: {error:?}"
        );
        assert!(error.to_string().contains("E7"), "got: {error}");
    }

    #[test]
    fn deny_mode_garbage_is_rejected() {
        let error = CustosConfig::from_env(&env(&[("ATLAS_EXPLICIT_DENY_MODE", "sometimes")]))
            .expect_err("garbage must not load");

        assert!(
            matches!(&error, ConfigError::Invalid { name, .. } if name == "ATLAS_EXPLICIT_DENY_MODE"),
            "got: {error:?}"
        );
        assert!(!error.to_string().contains("sometimes"), "got: {error}");
    }

    #[test]
    fn deny_mode_from_str_recognizes_every_spelling() {
        assert_eq!(
            "disabled".parse::<DenyModeConfig>().unwrap(),
            DenyModeConfig::Disabled
        );
        assert_eq!(
            "audit".parse::<DenyModeConfig>().unwrap(),
            DenyModeConfig::Audit
        );
        assert_eq!(
            "enforced".parse::<DenyModeConfig>().unwrap(),
            DenyModeConfig::Enforced
        );
        assert!("Enforced".parse::<DenyModeConfig>().is_err());
    }
}
