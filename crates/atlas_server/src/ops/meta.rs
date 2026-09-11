//! Registry-derived identity fields shared by `GET /version` and
//! `GET /api/v2/platform/meta` (E11-S3a design D4): release metadata only,
//! never a runtime/config value (SHELL-OPS-7).

use atlas_api::dtos::ComponentSummaryDto;
use atlas_core::registry::Registry;

/// Lists every registry-present component as a summary, in the registry's
/// own order. Derived from `registry.entries()`, so an absent component
/// (e.g. a disabled storage/search Module) never appears (design threat
/// matrix row, INV-NON-VACUOUS).
pub fn component_summaries(registry: &Registry) -> Vec<ComponentSummaryDto> {
    registry
        .entries()
        .iter()
        .map(|entry| ComponentSummaryDto {
            stable_id: entry.identity.stable_id.as_str().to_string(),
            kind: entry.identity.kind.to_string(),
            contract_version: entry.identity.contract_version.value(),
            navigation_providers: entry.experience.navigation_providers.clone(),
        })
        .collect()
}

/// The fields shared by `VersionDto` and `ServerMetaDto` (design D4): built
/// once so the two payloads cannot drift apart on the same process.
pub fn shared_identity(
    registry: &Registry,
    build: Option<String>,
) -> (String, Option<String>, Vec<ComponentSummaryDto>) {
    (
        env!("CARGO_PKG_VERSION").to_string(),
        build,
        component_summaries(registry),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reg5::{StorageBackend, reg5_component_entries};
    use atlas_core::registry::build;

    #[test]
    fn lists_every_present_component_for_both_storage_backends() {
        for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
            let registry = build(reg5_component_entries(backend)).expect("valid registry");

            let summaries = component_summaries(&registry);

            assert!(
                !summaries.is_empty(),
                "component_summaries must walk a non-zero component count"
            );
            assert_eq!(summaries.len(), registry.entries().len());

            let has_platform = summaries.iter().any(|s| s.stable_id == "platform");
            assert!(has_platform, "platform must be among the summaries");
        }
    }

    /// Design threat matrix: "an absent component appears in `components`".
    /// The list is derived from `registry.entries()`, so the storage backend
    /// not selected for this process is never named.
    #[test]
    fn an_inactive_storage_backend_never_appears_in_the_summaries() {
        let disk_registry =
            build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");
        let disk_summaries = component_summaries(&disk_registry);
        assert!(
            disk_summaries
                .iter()
                .any(|s| s.stable_id == "storage.filesystem")
        );
        assert!(!disk_summaries.iter().any(|s| s.stable_id == "storage.s3"));

        let s3_registry =
            build(reg5_component_entries(StorageBackend::S3)).expect("valid registry");
        let s3_summaries = component_summaries(&s3_registry);
        assert!(s3_summaries.iter().any(|s| s.stable_id == "storage.s3"));
        assert!(
            !s3_summaries
                .iter()
                .any(|s| s.stable_id == "storage.filesystem")
        );
    }

    /// E11-S8 design D-S8-4: acta and custos each declare one navigation
    /// provider id in `reg5.rs`, and `component_summaries` must carry that
    /// declaration through verbatim so the web shell can resolve it.
    #[test]
    fn component_summaries_carry_the_registry_declared_navigation_providers() {
        let registry =
            build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

        let summaries = component_summaries(&registry);

        let acta = summaries
            .iter()
            .find(|s| s.stable_id == "acta")
            .expect("acta must be present");
        assert_eq!(
            acta.navigation_providers,
            vec!["acta.workspace".to_string()]
        );

        let custos = summaries
            .iter()
            .find(|s| s.stable_id == "custos")
            .expect("custos must be present");
        assert_eq!(
            custos.navigation_providers,
            vec!["custos.admin".to_string()]
        );

        let platform = summaries
            .iter()
            .find(|s| s.stable_id == "platform")
            .expect("platform must be present");
        assert!(platform.navigation_providers.is_empty());
    }

    #[test]
    fn shared_identity_carries_the_given_build_and_the_crate_version() {
        let registry =
            build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

        let (version, build_id, components) =
            shared_identity(&registry, Some("abc123".to_string()));

        assert_eq!(version, env!("CARGO_PKG_VERSION"));
        assert_eq!(build_id, Some("abc123".to_string()));
        assert!(!components.is_empty());
    }
}
