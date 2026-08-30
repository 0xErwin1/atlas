#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod actor;
pub mod entities;
pub mod error;
pub mod ids;
pub mod permissions;
pub mod ports;

/// Acta-owned top-level modules relocated to `atlas_acta` (S2d). Re-exported
/// here to keep every existing `crate::*` import path compiling.
pub use atlas_acta::{document_lines, frontmatter, revision, search, semantic_search, wikilink};

pub use atlas_core::position;
pub use atlas_core::slug;
pub use atlas_core::slug::{resolve_collision, slugify};
pub use frontmatter::{parse_frontmatter_yaml, strip_frontmatter};
pub use ports::attachment_store::AttachmentStore;
pub use wikilink::{
    ParsedWikilink, WikilinkTarget, classify_wikilink, parse_wikilink_target, parse_wikilinks,
    rename_file_links,
};

pub use actor::{Actor, ApiKeyAttributionId, UserAttributionId, WorkspaceCtx};
pub use error::{DomainError, RevisionConflict};
pub use ids::{
    ApiKeyId, AttachmentId, BoardId, ChecklistItemId, ColumnId, CommentDraftId, CommentId,
    DocumentId, FolderId, GroupId, MembershipId, PlatformStatusTemplateId, ProjectId,
    PropertyDefinitionId, RevisionId, SavedSearchId, SecurityAuditId, SessionId, StatusTemplateId,
    TagId, TaskActivityId, TaskId, TaskReferenceId, TaskViewId, UserId, WorkspaceId,
};

pub trait HealthProbe {
    fn ping(&self) -> Result<(), DomainError>;
}
