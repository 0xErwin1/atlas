//! Bidirectional link between the registry's `ConfigDeclaration`s and the
//! typed structs `AtlasConfig` composes (design D2.2). `ConfigDeclaration`
//! carries no loader (`atlas_core::config::component` is deliberately not
//! object-safe), so this test is what makes the declaration non-inert: every
//! declared struct must have a composed loader, and every composed loader
//! must be declared by some entry — checked for both `StorageBackend`
//! values, since the active storage entry differs between them.

use atlas_core::registry::ComponentEntry;
use atlas_server::config::COMPOSED_STRUCT_NAMES;
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

fn declared_struct_names(entries: &[ComponentEntry]) -> Vec<&str> {
    entries
        .iter()
        .filter_map(|entry| entry.config.as_ref().map(|config| config.struct_name()))
        .collect()
}

fn assert_bidirectional_link(backend: StorageBackend) {
    let entries = reg5_component_entries(backend);
    let declared = declared_struct_names(&entries);

    for name in &declared {
        assert!(
            COMPOSED_STRUCT_NAMES.contains(name),
            "entry declares config struct `{name}` but AtlasConfig composes no such struct \
             (backend {backend:?})"
        );
    }

    for name in COMPOSED_STRUCT_NAMES {
        assert!(
            declared.contains(name),
            "AtlasConfig composes `{name}` but no registry entry declares it (backend {backend:?})"
        );
    }
}

#[test]
fn every_declared_struct_has_a_composed_loader_on_the_filesystem_backend() {
    assert_bidirectional_link(StorageBackend::Filesystem);
}

#[test]
fn every_declared_struct_has_a_composed_loader_on_the_s3_backend() {
    assert_bidirectional_link(StorageBackend::S3);
}

#[test]
fn composed_struct_names_is_non_empty() {
    // R1-style non-vacuity gate: a bidirectional check over an empty list on
    // both sides would pass trivially.
    assert!(!COMPOSED_STRUCT_NAMES.is_empty());
}
