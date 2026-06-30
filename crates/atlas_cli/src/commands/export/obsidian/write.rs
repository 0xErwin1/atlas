#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::path::Path;

use crate::error::CliError;

use super::plan::ExportPlan;

/// Writes an `ExportPlan` to `root`, creating all directories and files.
///
/// Directories are created depth-first via `create_dir_all`, which handles
/// nested paths in any order. Each file's parent directory is also ensured so
/// callers do not need to guarantee that every ancestor dir is in `plan.dirs`.
pub(crate) fn materialize(plan: &ExportPlan, root: &Path) -> Result<(), CliError> {
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
}
