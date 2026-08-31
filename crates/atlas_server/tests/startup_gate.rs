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
