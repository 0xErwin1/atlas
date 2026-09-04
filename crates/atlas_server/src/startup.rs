//! Server startup registry validation gate (SHELL-REG-3, SHELL-CFG-2).
//!
//! `main.rs` calls [`run_registry_gate`] with the REG-5 component entries
//! before the server binds and starts accepting connections. On success the
//! server proceeds to serve traffic unchanged. On failure the gate writes
//! one line per `RegistryBuildError` to the given sink — each already
//! naming the offending `stable_id` and the specific violated rule via
//! `RegistryBuildError`'s own `Display` impl (`atlas_core::registry::error`),
//! with no secret or configuration value in any variant — and returns a
//! non-zero exit code for the caller to act on.

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
}
