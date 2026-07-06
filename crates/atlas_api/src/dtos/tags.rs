use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Workspace tag representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TagDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /api/workspaces/{ws}/tags`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateTagRequest {
    pub name: String,
}

/// Request body for `PATCH /api/workspaces/{ws}/tags/{tag_id}`.
///
/// Both fields are optional: supply `name` to rename, `color` to recolor, or both.
/// `color: None` in the JSON payload is treated as "leave unchanged"; there is no
/// way to explicitly clear a tag color via this endpoint (NULL = unset).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateTagRequest {
    pub name: Option<String>,
    pub color: Option<String>,
}
