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
    pub workspace_id: uuid::Uuid,
    pub kind: TrashKindDto,
    pub target_id: uuid::Uuid,
    pub deleted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RestoreTrashItemRequest {
    pub kind: TrashKindDto,
    pub target_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PurgeStatusDtoResponse {
    pub operation_id: uuid::Uuid,
    pub kind: TrashKindDto,
    pub target_id: uuid::Uuid,
    pub status: PurgeStatusDto,
    pub attempts: u32,
}
