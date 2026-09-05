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
use atlas_core::registry::{BoundWorkers, ComponentEntry, Registry, Worker, build};
use std::io::Write;
use std::sync::Arc;

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

/// Runs the E11-S3b worker-bind gate (spec "Startup refuses when a declared
/// worker has no bound implementation").
///
/// Reconciles `registry`'s declared workers against `workers` via
/// `BoundWorkers::bind`, called after `AppState::new` and before
/// `start_workers`. Returns the bound table on success, so the caller can
/// proceed to start workers and serve traffic. On any drift — a declared
/// worker with no implementation, or an implementation matching no
/// declaration — writes one line per violation naming the offending
/// `WorkerId` to `sink` and returns `Some(exit_code)`; the caller exits with
/// that code before any worker starts or the listener serves.
pub fn run_worker_bind_gate(
    registry: &Registry,
    workers: Vec<Arc<dyn Worker>>,
    sink: &mut dyn Write,
) -> Result<BoundWorkers, i32> {
    match BoundWorkers::bind(registry, workers) {
        Ok(bound) => Ok(bound),
        Err(errors) => {
            let _ = writeln!(sink, "worker binding failed:");

            for error in &errors {
                let _ = writeln!(sink, "  - {error}");
            }

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

    #[test]
    fn worker_bind_gate_refuses_startup_naming_the_unbound_worker() {
        use atlas_core::ops::test_support::FakeWorker;
        use atlas_core::registry::WorkerId;

        let entries = crate::reg5::reg5_component_entries(crate::reg5::StorageBackend::Filesystem);
        let registry = build(entries).expect("valid REG-5 registry");

        // Five of the six REG-5-declared workers are bound; the sixth
        // (`acta.webhook_dispatcher`) is deliberately missing, exercising
        // the real gate `main.rs` calls (T1.38), not only
        // `reg5_registry_build.rs`'s synthetic call.
        let bound_ids = [
            "acta.attachment_reconciler",
            "acta.live_listener",
            "acta.presence_sweeper",
            "acta.presence_agent",
            "search.pgvector_embeddings.index_worker",
        ];
        let workers: Vec<Arc<dyn Worker>> = bound_ids
            .into_iter()
            .map(|id| {
                Arc::new(FakeWorker::new(WorkerId::new(id).expect("valid worker id")))
                    as Arc<dyn Worker>
            })
            .collect();

        let mut output = Vec::new();
        let exit_code = run_worker_bind_gate(&registry, workers, &mut output)
            .expect_err("a missing implementation must refuse startup");

        assert_eq!(exit_code, 1);

        let message = String::from_utf8(output).expect("gate output must be valid utf-8");
        assert!(
            message.contains("acta.webhook_dispatcher"),
            "message must name the unbound worker: {message}"
        );
    }
}
