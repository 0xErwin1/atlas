use std::path::PathBuf;

/// The complete plan for materializing an Atlas project as an Obsidian vault.
///
/// Built during the read phase (Atlas API calls) and then either printed
/// for `--dry-run` or passed to `materialize` for disk writes.
pub(crate) struct ExportPlan {
    pub dirs: Vec<DirOp>,
    pub files: Vec<FileOp>,
}

impl ExportPlan {
    pub(crate) fn new() -> Self {
        Self {
            dirs: Vec::new(),
            files: Vec::new(),
        }
    }
}

/// A directory to create at `<root> / rel_path`.
pub(crate) struct DirOp {
    pub rel_path: PathBuf,
}

/// A file to write at `<root> / rel_path` with the given `content`.
pub(crate) struct FileOp {
    pub rel_path: PathBuf,
    pub content: String,
}
