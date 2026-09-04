//! `acta` component configuration (registry declaration:
//! `ConfigDeclaration::new("ActaConfig", "ATLAS_ACTA_", true)`, `reg5.rs`).
//!
//! Owns anchor-interval tuning, the webhook dispatcher, webhook target
//! validation, the upload extension allow-list, and the attachment size
//! cap. `anchor_interval` collapses V1's two contradictory readers to one
//! rule (design D3.3): `config.rs`'s strict `>= 2` refusal. The clamp
//! `state.rs:63` used to apply on the production path is deleted here;
//! `AppState::for_test`'s own clamp is kept unchanged (it constructs no
//! config object).

use atlas_core::config::{ComponentConfig, ConfigError, EnvSource, Secret};

use super::{
    DEFAULT_MAX_ATTACHMENT_BYTES, DispatcherConfig, load_dispatcher_config, read_anchor_interval,
    read_env, read_env_bool,
};

/// Typed configuration owned by the `acta` component.
#[derive(Debug)] // safe: `webhook_enc_key` is `Secret<[u8; 32]>`.
pub struct ActaConfig {
    pub anchor_interval: u32,
    pub dispatcher: DispatcherConfig,
    /// Raw 32-byte AES-256-GCM key bytes decoded from `ATLAS_WEBHOOK_ENC_KEY`.
    pub webhook_enc_key: Secret<[u8; 32]>,
    pub allow_private_webhook_targets: bool,
    /// Upper bound on an uploaded attachment's size, in bytes
    /// (`ATLAS_ACTA_MAX_ATTACHMENT_BYTES`, new in this slice — previously a
    /// hardcoded constant with no environment binding).
    pub max_attachment_bytes: u64,
    /// Raw `ATLAS_UPLOAD_ALLOWED_EXTENSIONS` value; `state::AppState::new`
    /// parses it via `state::parse_upload_allowed_extensions`.
    pub upload_allowed_extensions: Option<String>,
}

impl ComponentConfig for ActaConfig {
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
        let anchor_interval = read_anchor_interval(source)
            .map_err(|_| ConfigError::invalid("ATLAS_ANCHOR_INTERVAL", "must be >= 2"))?;

        let webhook_enc_key = load_webhook_enc_key_typed(source)?;
        let dispatcher = load_dispatcher_config(source);
        let allow_private_webhook_targets =
            read_env_bool(source, "ATLAS_ALLOW_PRIVATE_WEBHOOK_TARGETS", false);
        let max_attachment_bytes = read_env(
            source,
            "ATLAS_ACTA_MAX_ATTACHMENT_BYTES",
            DEFAULT_MAX_ATTACHMENT_BYTES,
        );
        let upload_allowed_extensions = source.get("ATLAS_UPLOAD_ALLOWED_EXTENSIONS");

        Ok(Self {
            anchor_interval,
            dispatcher,
            webhook_enc_key,
            allow_private_webhook_targets,
            max_attachment_bytes,
            upload_allowed_extensions,
        })
    }
}

/// Adapts [`super::load_webhook_enc_key`]'s `String`-based errors into typed
/// [`ConfigError`]s that name the variable and never echo its value (its
/// `String` messages already avoid the raw key material — this only tightens
/// the byte-count/base64-detail out of the reason, matching every other
/// `ComponentConfig` implementer in this module).
fn load_webhook_enc_key_typed(source: &dyn EnvSource) -> Result<Secret<[u8; 32]>, ConfigError> {
    if source.get("ATLAS_WEBHOOK_ENC_KEY").is_none() {
        return Err(ConfigError::missing("ATLAS_WEBHOOK_ENC_KEY"));
    }

    let bytes = super::load_webhook_enc_key(source).map_err(|_| {
        ConfigError::invalid(
            "ATLAS_WEBHOOK_ENC_KEY",
            "must be base64 that decodes to exactly 32 bytes",
        )
    })?;

    Ok(Secret::new(bytes))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};

    fn valid_key() -> String {
        STANDARD.encode([0xAB_u8; 32])
    }

    fn env(pairs: Vec<(&'static str, String)>) -> impl EnvSource {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        }
    }

    #[test]
    fn from_env_composes_every_field() {
        let cfg = ActaConfig::from_env(&env(vec![
            ("ATLAS_ANCHOR_INTERVAL", "10".to_string()),
            ("ATLAS_WEBHOOK_ENC_KEY", valid_key()),
            ("ATLAS_ALLOW_PRIVATE_WEBHOOK_TARGETS", "true".to_string()),
            ("ATLAS_ACTA_MAX_ATTACHMENT_BYTES", "5242880".to_string()),
            ("ATLAS_UPLOAD_ALLOWED_EXTENSIONS", "png,jpg".to_string()),
        ]))
        .expect("expected Ok");

        assert_eq!(cfg.anchor_interval, 10);
        assert_eq!(*cfg.webhook_enc_key.expose(), [0xAB_u8; 32]);
        assert!(cfg.allow_private_webhook_targets);
        assert_eq!(cfg.max_attachment_bytes, 5_242_880);
        assert_eq!(cfg.upload_allowed_extensions.as_deref(), Some("png,jpg"));
    }

    #[test]
    fn max_attachment_bytes_defaults_to_20_mib_when_unset() {
        let cfg = ActaConfig::from_env(&env(vec![("ATLAS_WEBHOOK_ENC_KEY", valid_key())]))
            .expect("expected Ok");

        assert_eq!(cfg.max_attachment_bytes, 20 * 1024 * 1024);
    }

    #[test]
    fn max_attachment_bytes_binds_the_set_value() {
        let cfg = ActaConfig::from_env(&env(vec![
            ("ATLAS_WEBHOOK_ENC_KEY", valid_key()),
            ("ATLAS_ACTA_MAX_ATTACHMENT_BYTES", "5242880".to_string()),
        ]))
        .expect("expected Ok");

        assert_eq!(cfg.max_attachment_bytes, 5_242_880);
    }

    #[test]
    fn anchor_interval_below_2_is_invalid_naming_the_variable() {
        let error = ActaConfig::from_env(&env(vec![
            ("ATLAS_ANCHOR_INTERVAL", "1".to_string()),
            ("ATLAS_WEBHOOK_ENC_KEY", valid_key()),
        ]))
        .expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::invalid("ATLAS_ANCHOR_INTERVAL", "must be >= 2")
        );
    }

    #[test]
    fn missing_webhook_enc_key_is_missing_error() {
        let error = ActaConfig::from_env(&env(vec![])).expect_err("expected Err");

        assert_eq!(error, ConfigError::missing("ATLAS_WEBHOOK_ENC_KEY"));
    }

    #[test]
    fn invalid_webhook_enc_key_names_the_variable_without_echoing_it() {
        let error = ActaConfig::from_env(&env(vec![(
            "ATLAS_WEBHOOK_ENC_KEY",
            "not-valid-base64!!".to_string(),
        )]))
        .expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::invalid(
                "ATLAS_WEBHOOK_ENC_KEY",
                "must be base64 that decodes to exactly 32 bytes"
            )
        );
        assert!(!error.to_string().contains("not-valid-base64"));
    }
}
