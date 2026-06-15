use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Request body for `POST /v1/workspaces/{ws}/projects/{project_slug}/folders`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateFolderRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_folder_id: Option<uuid::Uuid>,
}

/// Request body for `PATCH /v1/workspaces/{ws}/folders/{folder_id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RenameFolderRequest {
    pub name: String,
}

/// Request body for `PATCH /v1/workspaces/{ws}/folders/{folder_id}/move`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MoveFolderRequest {
    /// New parent folder ID, or `null` to move to the project root.
    pub parent_folder_id: Option<uuid::Uuid>,
}

/// Folder representation returned by all folder endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FolderDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_folder_id: Option<uuid::Uuid>,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
