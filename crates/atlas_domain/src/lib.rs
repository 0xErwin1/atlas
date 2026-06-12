#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod actor;
pub mod error;
pub mod ids;
pub mod ports;
pub mod position;
pub mod revision;

pub use actor::{Actor, WorkspaceCtx};
pub use error::{DomainError, RevisionConflict};
pub use ids::{
    ApiKeyId, AttachmentId, BoardId, ColumnId, DocumentId, FolderId, MembershipId, ProjectId,
    PropertyDefinitionId, RevisionId, SessionId, TaskId, UserId, WorkspaceId,
};

pub trait HealthProbe {
    fn ping(&self) -> Result<(), DomainError>;
}
