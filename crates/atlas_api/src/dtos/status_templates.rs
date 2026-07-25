use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Workspace status template representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct StatusTemplateDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub position_key: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Atlas-wide default status representation, returned by the
/// `/api/admin/status-templates` surface. Same shape as [`StatusTemplateDto`]
/// minus `workspace_id`: these rows belong to the instance, not to a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PlatformStatusTemplateDto {
    pub id: uuid::Uuid,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub position_key: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /api/workspaces/{ws}/status-templates` and
/// `POST /api/admin/status-templates`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateStatusTemplateRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

fn present_value<'de, D>(de: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(de).map(Some)
}

/// Request body for `PATCH /api/workspaces/{ws}/status-templates/{id}` and
/// `PATCH /api/admin/status-templates/{id}`.
///
/// `name`: `None` = leave unchanged; `Some(v)` = rename.
/// `color`: absent = leave unchanged; explicit `null` = clear; string = set.
/// `before`/`after`: position anchors for reorder.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateStatusTemplateRequest {
    pub name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub color: Option<serde_json::Value>,
    pub before: Option<String>,
    pub after: Option<String>,
}
