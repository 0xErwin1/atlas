#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
// B1a fields: `save` (atomic write), `is_unchanged` (content-hash), `id`,
// `content_hash`, `folders`, `boards`, `tasks` are populated by the execute
// phase and not yet read in B0b.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::CliError;

/// A single document entry persisted in the import manifest.
///
/// `slug` and `id` are the server-side values returned after a successful
/// create or update. `content_hash` (SHA-256 of the content sent to Atlas)
/// enables SKIP-if-unchanged on re-import.
///
/// B1a: `id` and `content_hash` are written and checked in the execute phase.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub(crate) struct ManifestDocEntry {
    pub slug: String,
    pub id: String,
    pub content_hash: String,
}

/// The import manifest stored at `{vault}/.atlas-import.json`.
///
/// Maps vault-relative paths to server-side identifiers so re-imports can
/// take the update path and skip unchanged files.
///
/// B1a: Atomic write (`save`) and `is_unchanged` content-hash logic are not
/// yet implemented.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub version: u32,
    /// `{rel_path} → ManifestDocEntry` for successfully imported documents.
    pub documents: HashMap<String, ManifestDocEntry>,
    /// `{rel_dir} → folder_uuid` for successfully created folders.
    pub folders: HashMap<String, String>,
    /// `{rel_path} → board_uuid` for successfully created boards.
    pub boards: HashMap<String, String>,
    /// `{rel_path} → readable_id` (e.g. `ATL-42`) for successfully created tasks.
    pub tasks: HashMap<String, String>,
}

impl Manifest {
    /// Creates an empty manifest with `version = 1` and no entries.
    pub(crate) fn empty() -> Self {
        Self {
            version: 1,
            ..Default::default()
        }
    }

    /// Loads the manifest from `path`.
    ///
    /// Returns an empty manifest when the file is absent. Returns
    /// `CliError::Validation` when the file is present but malformed JSON.
    ///
    /// B1a: Atomic write (`save`) and `is_unchanged` content-hash comparison
    /// are not yet implemented.
    pub(crate) fn load(path: &Path) -> Result<Self, CliError> {
        if !path.exists() {
            return Ok(Self::empty());
        }

        let text = std::fs::read_to_string(path).map_err(CliError::from)?;

        serde_json::from_str(&text)
            .map_err(|e| CliError::Validation(format!("malformed .atlas-import.json: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn empty_manifest_has_version_1_and_no_entries() {
        let m = Manifest::empty();
        assert_eq!(m.version, 1);
        assert!(m.documents.is_empty());
        assert!(m.folders.is_empty());
        assert!(m.boards.is_empty());
        assert!(m.tasks.is_empty());
    }

    #[test]
    fn load_absent_file_returns_empty_manifest() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".atlas-import.json");

        let m = Manifest::load(&path).unwrap();
        assert_eq!(m.version, 1);
        assert!(m.documents.is_empty());
    }

    #[test]
    fn load_valid_json_round_trips() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".atlas-import.json");

        let mut m = Manifest::empty();
        m.documents.insert(
            "a.md".to_string(),
            ManifestDocEntry {
                slug: "a".to_string(),
                id: "uuid-1".to_string(),
                content_hash: "deadbeef".to_string(),
            },
        );

        fs::write(&path, serde_json::to_string(&m).unwrap()).unwrap();

        let loaded = Manifest::load(&path).unwrap();
        assert_eq!(loaded.documents.len(), 1);
        assert_eq!(loaded.documents["a.md"].slug, "a");
    }

    #[test]
    fn load_malformed_json_returns_validation_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".atlas-import.json");
        fs::write(&path, "not valid json { at all").unwrap();

        let result = Manifest::load(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            CliError::Validation(msg) => assert!(msg.contains("malformed")),
            other => panic!("expected Validation, got {other:?}"),
        }
    }
}
