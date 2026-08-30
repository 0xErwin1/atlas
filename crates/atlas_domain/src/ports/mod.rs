pub mod group_repo;
pub mod identity;
pub mod security_audit;

/// Acta-owned ports relocated to `atlas_acta::ports` (S2d). Re-exported here
/// to keep every existing `crate::ports::*` import path compiling.
pub use atlas_acta::ports::{
    CommentAttachmentDraftRepo, attachment_store, boards_tasks, comments, documents, lifecycle,
    saved_searches, search, status_templates, tags, task_views, workspace_core,
};
