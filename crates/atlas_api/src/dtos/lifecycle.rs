use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Resource kinds with independent Trash, restore, and purge contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum TrashKindDto {
    Project,
    Folder,
    Document,
    Comment,
    Attachment,
}

/// Durable progress of a confirmed purge request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum PurgeStatusDto {
    DbCommitted,
    CleanupPending,
    CleanupFailed,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TrashItemDto {
    /// Workspace that owns the deleted resource.
    pub workspace_id: uuid::Uuid,
    /// One of the five independently recoverable resource kinds.
    pub kind: TrashKindDto,
    /// UUID of the deleted resource.
    pub target_id: uuid::Uuid,
    /// UTC timestamp of the recoverable deletion.
    pub deleted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RestoreTrashItemRequest {
    /// Kind of resource to restore.
    pub kind: TrashKindDto,
    /// UUID of the deleted resource to restore.
    pub target_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PurgeTrashItemRequest {
    /// Kind of resource to purge permanently.
    pub kind: TrashKindDto,
    /// UUID of the deleted resource to purge permanently.
    pub target_id: uuid::Uuid,
    /// Must be true before a permanent purge is attempted.
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PurgeStatusDtoResponse {
    /// Durable operation identifier for status polling.
    pub operation_id: uuid::Uuid,
    /// Kind of resource being purged.
    pub kind: TrashKindDto,
    /// UUID of the purged resource.
    pub target_id: uuid::Uuid,
    /// Current durable database and cleanup progress.
    pub status: PurgeStatusDto,
    /// Number of durable cleanup attempts.
    pub attempts: u32,
}
