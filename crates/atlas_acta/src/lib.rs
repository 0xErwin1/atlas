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
pub mod document_lines;
pub mod entities;
pub mod frontmatter;
pub mod ids;
pub mod permissions;
pub mod ports;
pub mod revision;
pub mod search;
pub mod semantic_search;
pub mod wikilink;

pub use frontmatter::{parse_frontmatter_yaml, strip_frontmatter};
pub use ports::attachment_store::AttachmentStore;
pub use wikilink::{
    ParsedWikilink, WikilinkTarget, classify_wikilink, parse_wikilink_target, parse_wikilinks,
    rename_file_links,
};

pub use actor::{Actor, ApiKeyAttributionId, UserAttributionId, WorkspaceCtx};
pub use atlas_core::error::{DomainError, RevisionConflict};
pub use ids::{
    AttachmentId, BoardId, ChecklistItemId, ColumnId, CommentDraftId, CommentId,
    CommentLinkEventId, CommentLinkId, DocumentId, FolderId, MembershipId,
    PlatformStatusTemplateId, ProjectId, PropertyDefinitionId, PurgeOperationId, RevisionId,
    SavedSearchId, StatusTemplateId, TagId, TaskActivityId, TaskId, TaskReferenceId, TaskViewId,
    WorkspaceId,
};
