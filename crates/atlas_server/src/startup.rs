//! Server startup gates (SHELL-REG-3, SHELL-CFG-1, SHELL-CFG-2).
//!
//! `main.rs` calls [`run_registry_gate`] with the REG-5 component entries
//! and then [`run_config_gate`] with the same entries, both before the
//! server binds and starts accepting connections. On success the server
//! proceeds to serve traffic unchanged. On failure each gate writes its
//! diagnostic to the given sink and returns a non-zero exit code for the
//! caller to act on. The registry gate writes one line per
//! `RegistryBuildError`, each naming the offending `stable_id` and the
//! violated rule via `RegistryBuildError`'s own `Display` impl
//! (`atlas_core::registry::error`). The config gate writes the single
//! `ConfigError`, whose `Display` names the variable and never its value
//! (`atlas_core::config::error`).

use crate::config::AtlasConfig;
use atlas_core::config::EnvSource;
use atlas_core::registry::{ComponentEntry, build};
use std::io::Write;

/// Runs the REG-5 registry validation gate.
///
/// Returns `None` when the registry builds successfully, meaning the caller
/// should proceed to serve traffic. Returns `Some(exit_code)` on any
/// validation failure; the caller is responsible for exiting the process
/// with that code before the server binds, so an invalid registry never
/// gets the chance to serve traffic.
pub fn run_registry_gate(entries: Vec<ComponentEntry>, sink: &mut dyn Write) -> Option<i32> {
    match build(entries) {
        Ok(_) => None,
        Err(errors) => {
            let _ = writeln!(sink, "registry validation failed:");

            for error in &errors {
                let _ = writeln!(sink, "  - {error}");
            }

            Some(1)
        }
    }
}

/// Runs the SHELL-CFG-1 configuration gate.
///
/// Loads [`AtlasConfig`] for the components present in `entries` through
/// `source`. Returns the config on success so the caller can proceed to
/// serve traffic. On failure writes one `configuration error:` line to
/// `sink` and returns `Err(exit_code)`; the caller is responsible for
/// exiting the process with that code before the server binds.
pub fn run_config_gate(
    entries: &[ComponentEntry],
    source: &dyn EnvSource,
    sink: &mut dyn Write,
) -> Result<AtlasConfig, i32> {
    match AtlasConfig::from_registry(entries, source) {
        Ok(cfg) => Ok(cfg),
        Err(error) => {
            let _ = writeln!(sink, "configuration error: {error}");

            Err(1)
        }
    }
}

/// Reads `ATLAS_PORT` through `source`, defaulting to `default` when the
/// variable is absent or fails to parse as a `u16`.
pub fn read_port(source: &dyn EnvSource, default: u16) -> u16 {
    source
        .get("ATLAS_PORT")
        .and_then(|p| p.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OneVar(&'static str, &'static str);

    impl EnvSource for OneVar {
        fn get(&self, key: &str) -> Option<String> {
            (key == self.0).then(|| self.1.to_string())
        }
    }

    struct NoVars;

    impl EnvSource for NoVars {
        fn get(&self, _key: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn read_port_binds_atlas_port_through_the_injected_source() {
        let source = OneVar("ATLAS_PORT", "9090");

        assert_eq!(read_port(&source, 8080), 9090);
    }

    #[test]
    fn read_port_falls_back_to_the_default_when_unset() {
        assert_eq!(read_port(&NoVars, 8080), 8080);
    }

    #[test]
    fn config_gate_refuses_on_missing_mandatory_variable_naming_it_and_not_a_value() {
        let entries = crate::reg5::reg5_component_entries(crate::reg5::StorageBackend::Filesystem);
        let source = OneVar("DATABASE_URL", "postgres://set-value/db");

        let mut output = Vec::new();
        let exit_code = run_config_gate(&entries, &source, &mut output)
            .expect_err("missing ATLAS_WEBHOOK_ENC_KEY must refuse startup");

        assert_eq!(exit_code, 1);

        let message = String::from_utf8(output).expect("gate output must be valid utf-8");
        assert!(
            message.contains("ATLAS_WEBHOOK_ENC_KEY"),
            "message must name the missing variable: {message}"
        );
        assert!(
            !message.contains("set-value"),
            "message must not leak any configured value: {message}"
        );
    }
}
