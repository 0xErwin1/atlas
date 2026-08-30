pub mod groups;
pub mod identity;
pub mod security_audit;

/// Acta-owned entities relocated to `atlas_acta::entities` (S2d). Re-exported
/// here to keep every existing `crate::entities::*` import path compiling.
pub use atlas_acta::entities::{
    boards_tasks, comments, documents, events, lifecycle, saved_searches, status_templates, tags,
    task_views, workspace_core,
};
