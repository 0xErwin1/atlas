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
pub mod frontmatter;
pub mod ids;
pub mod permissions;
pub mod ports;
pub mod position;
pub mod revision;
pub mod search;
pub mod slug;
pub mod wikilink;

pub use frontmatter::{parse_frontmatter_yaml, strip_frontmatter};
pub use ports::attachment_store::AttachmentStore;
pub use slug::{resolve_collision, slugify};
pub use wikilink::{parse_wikilink_target, parse_wikilinks};

pub use actor::{Actor, WorkspaceCtx};
pub use error::{DomainError, RevisionConflict};
pub use ids::{
    ApiKeyId, AttachmentId, BoardId, ChecklistItemId, ColumnId, DocumentId, FolderId, GroupId,
    MembershipId, ProjectId, PropertyDefinitionId, RevisionId, SavedSearchId, SecurityAuditId,
    SessionId, StatusTemplateId, TagId, TaskActivityId, TaskId, TaskReferenceId, TaskViewId,
    UserId, WorkspaceId,
};

pub trait HealthProbe {
    fn ping(&self) -> Result<(), DomainError>;
}
