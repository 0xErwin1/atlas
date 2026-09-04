//! Startup-failure test for SHELL-REG-3 / SHELL-CFG-2
//! (`v2-e3-s1-registry-population`, PR2).
//!
//! Feeds an intentionally invalid entry set (two entries sharing a
//! `stable_id`) through `atlas_server::startup::run_registry_gate` and
//! asserts the failure contract: a non-zero exit code, and a message that
//! names the offending `stable_id` and identifies the violated rule
//! category, without pinning exact wording. Also confirms the happy path:
//! the real REG-5 entries produce no exit code and no failure output.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_core::registry::{
    Api, Authorization, Capabilities, ComponentEntry, ComponentId, ComponentKind, ContractVersion,
    Diagnostics, Experience, Identity,
};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use atlas_server::startup::run_registry_gate;

fn component(value: &str) -> ComponentId {
    ComponentId::new(value).expect("valid component id")
}

/// A minimal, otherwise-valid `ComponentEntry` with no capabilities, routes,
/// persistence, or config — only `stable_id` varies between calls. Used to
/// build an intentionally invalid entry set (two entries sharing a
/// `stable_id`) without depending on the real REG-5 shape.
fn minimal_entry(stable_id: &str) -> ComponentEntry {
    ComponentEntry {
        identity: Identity {
            stable_id: component(stable_id),
            kind: ComponentKind::Module,
            contract_version: ContractVersion::new(1),
        },
        dependencies: vec![],
        capabilities: Capabilities {
            provided: vec![],
            required_mandatory: vec![],
            required_optional: vec![],
        },
        api: Api {
            namespace: None,
            routes: vec![],
            dto_owner: None,
        },
        authorization: Authorization {
            resource_kinds: vec![],
            actions: vec![],
            role_definitions: vec![],
            principal_sets: vec![],
            provider: false,
        },
        diagnostics: Diagnostics {
            health: false,
            readiness: false,
            doctor: false,
        },
        experience: Experience {
            navigation_providers: vec![],
            context_providers: vec![],
        },
        persistence: None,
        config: None,
        workers: vec![],
        satellites: vec![],
    }
}

#[test]
fn startup_gate_reports_non_zero_exit_and_names_stable_id_and_rule_on_invalid_registry() {
    let duplicate_stable_id = vec![minimal_entry("acta"), minimal_entry("acta")];

    let mut output = Vec::new();
    let exit_code = run_registry_gate(duplicate_stable_id, &mut output);

    assert_eq!(
        exit_code,
        Some(1),
        "an invalid registry must produce a non-zero exit code"
    );

    let message = String::from_utf8(output).expect("gate output must be valid utf-8");
    assert!(
        message.contains("acta"),
        "message must name the offending stable_id: {message}"
    );
    assert!(
        message.to_lowercase().contains("duplicate"),
        "message must identify the violated rule category: {message}"
    );
}

#[test]
fn startup_gate_proceeds_when_the_real_reg5_registry_is_valid() {
    let mut output = Vec::new();
    let exit_code = run_registry_gate(
        reg5_component_entries(StorageBackend::Filesystem),
        &mut output,
    );

    assert_eq!(
        exit_code, None,
        "the real REG-5 registry must pass validation and let startup proceed"
    );
    assert!(
        output.is_empty(),
        "no failure output should be written on a valid registry"
    );
}

/// SHELL-CFG-1: the second startup gate, `AtlasConfig::from_registry`, runs
/// after `run_registry_gate` and refuses when mandatory config for a present
/// component is missing, naming the variable and never its value.
/// `atlas_server::startup::run_config_gate` wraps it for `main.rs` (design
/// D2.5) and is unit-tested in its own module; these tests exercise the
/// loader directly since `ConfigError` already asserts value-freedom in its
/// own module tests (`atlas_core::config::error`).
mod config_gate {
    use atlas_core::config::{ConfigError, EnvSource};
    use atlas_server::config::AtlasConfig;
    use atlas_server::reg5::{StorageBackend, reg5_component_entries};
    use base64::{Engine, engine::general_purpose::STANDARD};

    fn env(pairs: Vec<(&'static str, String)>) -> impl EnvSource {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        }
    }

    fn valid_key() -> String {
        STANDARD.encode([0xAB_u8; 32])
    }

    #[test]
    fn missing_webhook_enc_key_refuses_startup_naming_the_variable() {
        let entries = reg5_component_entries(StorageBackend::Filesystem);
        let source = env(vec![(
            "DATABASE_URL",
            "postgres://set-value/db".to_string(),
        )]);

        let error = AtlasConfig::from_registry(&entries, &source).expect_err("expected Err");

        assert_eq!(error, ConfigError::missing("ATLAS_WEBHOOK_ENC_KEY"));
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn s3_backend_missing_bucket_refuses_startup_naming_the_variable() {
        let entries = reg5_component_entries(StorageBackend::S3);
        let source = env(vec![
            ("DATABASE_URL", "postgres://set-value/db".to_string()),
            ("ATLAS_WEBHOOK_ENC_KEY", valid_key()),
            ("ATLAS_ATTACHMENT_BACKEND", "s3".to_string()),
            ("ATLAS_S3_ENDPOINT", "endpoint".to_string()),
            ("ATLAS_S3_ACCESS_KEY_ID", "access-key".to_string()),
            ("ATLAS_S3_SECRET_ACCESS_KEY", "secret-key".to_string()),
        ]);

        let error = AtlasConfig::from_registry(&entries, &source).expect_err("expected Err");

        assert_eq!(error, ConfigError::missing("ATLAS_S3_BUCKET"));
    }

    #[test]
    fn every_mandatory_variable_present_starts_successfully() {
        let entries = reg5_component_entries(StorageBackend::Filesystem);
        let source = env(vec![
            ("DATABASE_URL", "postgres://set-value/db".to_string()),
            ("ATLAS_WEBHOOK_ENC_KEY", valid_key()),
        ]);

        AtlasConfig::from_registry(&entries, &source).expect("expected Ok");
    }
}
