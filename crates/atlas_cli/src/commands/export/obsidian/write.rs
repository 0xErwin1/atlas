#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::path::{Component, Path};

use crate::error::CliError;

use super::plan::ExportPlan;

/// Rejects any plan path that could escape the export root when joined onto it.
///
/// Server-supplied names feed these relative paths, so an absolute component
/// would replace the root entirely and a `..` component would climb out of it.
fn ensure_contained_rel_path(rel_path: &Path) -> Result<(), CliError> {
    for component in rel_path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(CliError::Validation(format!(
                    "refusing to write unsafe export path '{}': absolute or parent-directory \
                     components are not allowed",
                    rel_path.display()
                )));
            }
        }
    }

    Ok(())
}

/// Writes an `ExportPlan` to `root`, creating all directories and files.
///
/// Every relative path is validated to stay inside `root` before anything is
/// written. Directories are created depth-first via `create_dir_all`, which
/// handles nested paths in any order. Each file's parent directory is also
/// ensured so callers do not need to guarantee that every ancestor dir is in
/// `plan.dirs`.
pub(crate) fn materialize(plan: &ExportPlan, root: &Path) -> Result<(), CliError> {
    for dir_op in &plan.dirs {
        ensure_contained_rel_path(&dir_op.rel_path)?;
    }
    for file_op in &plan.files {
        ensure_contained_rel_path(&file_op.rel_path)?;
    }

    for dir_op in &plan.dirs {
        std::fs::create_dir_all(root.join(&dir_op.rel_path))?;
    }

    for file_op in &plan.files {
        let dest = root.join(&file_op.rel_path);

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&dest, &file_op.content)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::export::obsidian::plan::{DirOp, ExportPlan, FileOp};
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn materialize_creates_directories_and_files() {
        let root = tempdir().unwrap();
        let root_path = root.path();

        let mut plan = ExportPlan::new();
        plan.dirs.push(DirOp {
            rel_path: PathBuf::from("subfolder"),
        });
        plan.files.push(FileOp {
            rel_path: PathBuf::from("subfolder/note.md"),
            content: "# Hello".to_string(),
        });
        plan.files.push(FileOp {
            rel_path: PathBuf::from("root.md"),
            content: "Root content".to_string(),
        });

        materialize(&plan, root_path).unwrap();

        assert!(root_path.join("subfolder").is_dir());
        let note = std::fs::read_to_string(root_path.join("subfolder/note.md")).unwrap();
        assert_eq!(note, "# Hello");
        let root_file = std::fs::read_to_string(root_path.join("root.md")).unwrap();
        assert_eq!(root_file, "Root content");
    }

    #[test]
    fn materialize_file_without_prior_dir_op_still_works() {
        let root = tempdir().unwrap();
        let root_path = root.path();

        let mut plan = ExportPlan::new();
        plan.files.push(FileOp {
            rel_path: PathBuf::from("deep/nested/file.md"),
            content: "deep".to_string(),
        });

        materialize(&plan, root_path).unwrap();

        let contents = std::fs::read_to_string(root_path.join("deep/nested/file.md")).unwrap();
        assert_eq!(contents, "deep");
    }

    #[test]
    fn materialize_empty_plan_is_noop() {
        let root = tempdir().unwrap();
        let plan = ExportPlan::new();
        materialize(&plan, root.path()).unwrap();
    }

    #[test]
    fn materialize_rejects_parent_dir_file_path_without_writing() {
        let root = tempdir().unwrap();

        let mut plan = ExportPlan::new();
        plan.files.push(FileOp {
            rel_path: PathBuf::from("../escape.md"),
            content: "evil".to_string(),
        });

        let err = materialize(&plan, root.path()).unwrap_err();

        assert!(matches!(err, CliError::Validation(_)));
        assert!(
            !root.path().parent().unwrap().join("escape.md").exists(),
            "no file may be written outside the root"
        );
    }

    #[test]
    fn materialize_rejects_absolute_file_path() {
        let root = tempdir().unwrap();

        let mut plan = ExportPlan::new();
        plan.files.push(FileOp {
            rel_path: PathBuf::from("/etc/atlas-export-escape.md"),
            content: "evil".to_string(),
        });

        let err = materialize(&plan, root.path()).unwrap_err();
        assert!(matches!(err, CliError::Validation(_)));
    }

    #[test]
    fn materialize_rejects_nested_parent_dir_traversal() {
        let root = tempdir().unwrap();

        let mut plan = ExportPlan::new();
        plan.files.push(FileOp {
            rel_path: PathBuf::from("safe/../../escape.md"),
            content: "evil".to_string(),
        });

        let err = materialize(&plan, root.path()).unwrap_err();
        assert!(matches!(err, CliError::Validation(_)));
    }

    #[test]
    fn materialize_rejects_parent_dir_dir_op_before_any_write() {
        let root = tempdir().unwrap();

        let mut plan = ExportPlan::new();
        plan.dirs.push(DirOp {
            rel_path: PathBuf::from("../escaped-dir"),
        });
        plan.files.push(FileOp {
            rel_path: PathBuf::from("safe.md"),
            content: "ok".to_string(),
        });

        let err = materialize(&plan, root.path()).unwrap_err();

        assert!(matches!(err, CliError::Validation(_)));
        assert!(
            !root.path().join("safe.md").exists(),
            "validation must reject the whole plan before writing anything"
        );
    }
}
