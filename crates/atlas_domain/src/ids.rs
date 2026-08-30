/// Principal identity ids. Relocated to `atlas_core::principal` (D4) so
/// `atlas_core` owns principal identity; re-exported here to keep every
/// existing `crate::ids::{UserId, ApiKeyId, GroupId}` import path resolving
/// to the same type.
pub use atlas_core::principal::{ApiKeyId, GroupId, UserId};

/// Custos-owned ids relocated to `atlas_custos::ids` (S2d D6); re-exported
/// here to keep every existing `crate::ids::{SessionId, ActivationTokenId,
/// SecurityAuditId}` import path resolving to the same type.
pub use atlas_custos::ids::{ActivationTokenId, SecurityAuditId, SessionId};

/// Acta-owned ids relocated to `atlas_acta::ids` (S2d D6); re-exported here
/// to keep every existing `crate::ids::*` import path resolving to the same
/// type.
pub use atlas_acta::ids::{
    AttachmentId, BoardId, ChecklistItemId, ColumnId, CommentDraftId, CommentId,
    CommentLinkEventId, CommentLinkId, DocumentId, FolderId, MembershipId,
    PlatformStatusTemplateId, ProjectId, PropertyDefinitionId, PurgeOperationId, RevisionId,
    SavedSearchId, StatusTemplateId, TagId, TaskActivityId, TaskId, TaskReferenceId, TaskViewId,
    WorkspaceId,
};
