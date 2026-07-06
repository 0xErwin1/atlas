use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Task view filter set as exposed at the API boundary.
///
/// All fields are optional. An empty `{}` is a valid "all workspace tasks" view.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskViewFiltersDto {
    /// Sort order. Valid values: updated_at_desc, updated_at_asc, created_at_desc,
    /// created_at_asc, priority_desc, title_asc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,

    /// Restrict to these priority levels. Valid values: low, medium, high, urgent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub priorities: Vec<String>,

    /// Restrict to tasks carrying ALL of these labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,

    /// Restrict to these board column ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_ids: Vec<uuid::Uuid>,

    /// Scope to a single board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_id: Option<uuid::Uuid>,

    /// Assignee filter. Encoded as a string: "me", "user:{uuid}", "api_key:{uuid}".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,

    /// Creator actor-type filter. Valid values: "user", "api_key".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_type: Option<String>,
}

/// Task view representation. Does not expose the owner principal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskViewDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub name: String,
    pub filters: TaskViewFiltersDto,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /api/workspaces/{ws}/task-views`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateTaskViewRequest {
    pub name: String,
    pub filters: TaskViewFiltersDto,
}

/// Request body for `PATCH /api/workspaces/{ws}/task-views/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateTaskViewRequest {
    pub name: String,
    pub filters: TaskViewFiltersDto,
}
