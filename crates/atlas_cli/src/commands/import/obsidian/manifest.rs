#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    pub(crate) fn load(path: &Path) -> Result<Self, CliError> {
        if !path.exists() {
            return Ok(Self::empty());
        }

        let text = std::fs::read_to_string(path).map_err(CliError::from)?;

        serde_json::from_str(&text)
            .map_err(|e| CliError::Validation(format!("malformed .atlas-import.json: {e}")))
    }

    /// Writes the manifest to `path` atomically.
    ///
    /// Writes to a `NamedTempFile` in the same directory as `path`, then
    /// renames it into place. An interrupted write never corrupts an existing
    /// valid manifest file.
    pub(crate) fn save(&self, path: &Path) -> Result<(), CliError> {
        let dir = path
            .parent()
            .ok_or_else(|| CliError::Validation("manifest path has no parent directory".into()))?;

        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;

        serde_json::to_writer_pretty(&mut tmp, self)
            .map_err(|e| CliError::Validation(format!("failed to serialize manifest: {e}")))?;

        tmp.persist(path).map_err(|e| CliError::Io(e.error))?;

        Ok(())
    }

    /// Returns `true` when `rel_path` is in the manifest and its stored
    /// `content_hash` matches `new_hash`. Returns `false` for unknown paths
    /// and on hash mismatch.
    pub(crate) fn is_unchanged(&self, rel_path: &str, new_hash: &str) -> bool {
        self.documents
            .get(rel_path)
            .map(|e| e.content_hash == new_hash)
            .unwrap_or(false)
    }
}

/// Computes a SHA-256 digest of `content` and returns it as a 64-character
/// lowercase hex string.
pub(crate) fn content_hash(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
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

    // -- content_hash -----------------------------------------------------------

    #[test]
    fn content_hash_is_64_char_lowercase_hex() {
        let h = content_hash("hello");
        assert_eq!(h.len(), 64, "SHA-256 digest must be 64 hex chars");
        assert!(
            h.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "must be lowercase hex; got: {h}"
        );
    }

    #[test]
    fn content_hash_is_stable_for_same_input() {
        assert_eq!(content_hash("same text"), content_hash("same text"));
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        assert_ne!(content_hash("foo"), content_hash("bar"));
    }

    #[test]
    fn content_hash_known_vector() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let h = content_hash("");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    // -- is_unchanged -----------------------------------------------------------

    #[test]
    fn is_unchanged_returns_true_when_hash_matches() {
        let mut m = Manifest::empty();
        let hash = content_hash("some content");
        m.documents.insert(
            "a.md".into(),
            ManifestDocEntry {
                slug: "a".into(),
                id: "id-1".into(),
                content_hash: hash.clone(),
            },
        );
        assert!(m.is_unchanged("a.md", &hash));
    }

    #[test]
    fn is_unchanged_returns_false_when_hash_differs() {
        let mut m = Manifest::empty();
        m.documents.insert(
            "a.md".into(),
            ManifestDocEntry {
                slug: "a".into(),
                id: "id-1".into(),
                content_hash: content_hash("original"),
            },
        );
        assert!(!m.is_unchanged("a.md", &content_hash("modified")));
    }

    #[test]
    fn is_unchanged_returns_false_for_unknown_path() {
        let m = Manifest::empty();
        assert!(!m.is_unchanged("not-in-manifest.md", "anyhash"));
    }

    // -- save (atomic write) ---------------------------------------------------

    #[test]
    fn save_and_load_round_trip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".atlas-import.json");

        let mut m = Manifest::empty();
        m.documents.insert(
            "note.md".into(),
            ManifestDocEntry {
                slug: "note".into(),
                id: "uuid-abc".into(),
                content_hash: content_hash("content"),
            },
        );
        m.folders.insert("notes/".into(), "folder-uuid".into());

        m.save(&path).unwrap();

        let loaded = Manifest::load(&path).unwrap();
        assert_eq!(loaded.version, 1);
        assert_eq!(loaded.documents.len(), 1);
        assert_eq!(loaded.documents["note.md"].slug, "note");
        assert_eq!(loaded.folders.len(), 1);
        assert_eq!(loaded.folders["notes/"], "folder-uuid");
    }

    #[test]
    fn save_atomic_no_temp_file_left_on_success() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".atlas-import.json");

        Manifest::empty().save(&path).unwrap();

        let entries: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();

        assert_eq!(entries.len(), 1, "only the manifest file should remain");
        assert_eq!(
            entries[0].file_name(),
            ".atlas-import.json",
            "the file must be the manifest"
        );
    }
}
