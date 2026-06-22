use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use super::documents::ActorDto;

// ---------------------------------------------------------------------------
// Board DTOs
// ---------------------------------------------------------------------------

/// Full board representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BoardDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub name: String,
    pub created_by: ActorDto,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Lightweight board summary for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BoardSummaryDto {
    pub id: uuid::Uuid,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Column representation (always returned in board context).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ColumnDto {
    pub id: uuid::Uuid,
    pub board_id: uuid::Uuid,
    pub name: String,
    pub position_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Task DTOs
// ---------------------------------------------------------------------------

/// Typed task properties validated at the API boundary.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskPropertiesDto {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<chrono::DateTime<chrono::Utc>>,
    /// Non-negative work estimate in story-point units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Free-form JSONB escape hatch; no workspace-schema validation in M1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<serde_json::Value>,
}

/// Full task representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub project_id: uuid::Uuid,
    pub board_id: uuid::Uuid,
    pub column_id: uuid::Uuid,
    /// Set when this task is a sub-task of another; absent for top-level tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_task_id: Option<uuid::Uuid>,
    pub readable_id: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_date: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Value>,
    pub created_by: ActorDto,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Name of the board this task belongs to.
    pub board_name: String,
    /// Name of the column (status) this task is currently in.
    pub column_name: String,
}

/// Lightweight task summary for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskSummaryDto {
    pub id: uuid::Uuid,
    pub readable_id: String,
    pub board_id: uuid::Uuid,
    pub column_id: uuid::Uuid,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Non-negative work estimate in story-point units; surfaced inline (e.g. on a
    /// sub-task row) without loading the full task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Assigned actors (users and agents), resolved with display names so the
    /// kanban card can render an avatar and the agent badge without a follow-up
    /// request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<ActorDto>,
    /// Name of the board this task belongs to.
    pub board_name: String,
    /// Name of the column (status) this task is currently in.
    pub column_name: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Assignee DTOs
// ---------------------------------------------------------------------------

/// An actor assigned to a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AssigneeDto {
    pub assignee: ActorDto,
    pub assigned_by: ActorDto,
    pub assigned_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Reference DTOs
// ---------------------------------------------------------------------------

/// A typed outbound reference from a task to another task or document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReferenceDto {
    pub id: uuid::Uuid,
    /// "relates" | "blocks" | "parent" | "spec"
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_task_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_readable_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_document_id: Option<uuid::Uuid>,
    /// Title of the target document, for display. Present only for resolved
    /// document references (task references use `target_readable_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_title: Option<String>,
    /// False when the target no longer exists (broken ref), consistent with E04.
    pub target_resolved: bool,
    pub created_by: ActorDto,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// An inbound reference — another task that points to this one.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TaskBacklinkDto {
    pub source_task_id: uuid::Uuid,
    pub source_readable_id: String,
    pub source_title: String,
    /// "relates" | "blocks" | "parent" | "spec"
    pub kind: String,
}

// ---------------------------------------------------------------------------
// Checklist DTOs
// ---------------------------------------------------------------------------

/// A single checklist item.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ChecklistItemDto {
    pub id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub title: String,
    pub checked: bool,
    pub position_key: String,
    /// Set once the item has been promoted to a task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promoted_task_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promoted_readable_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Response from `POST .../checklist/{item_id}/promote`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PromotionDto {
    pub task: TaskDto,
    pub parent_reference: Option<ReferenceDto>,
    pub checklist_item: ChecklistItemDto,
}

// ---------------------------------------------------------------------------
// Activity DTOs
// ---------------------------------------------------------------------------

/// A single activity entry on a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActivityEntryDto {
    pub id: uuid::Uuid,
    /// "created" | "moved" | "assigned" | "unassigned" | "field_changed" |
    /// "reference_added" | "reference_removed" | "checklist_added" |
    /// "checklist_updated" | "checklist_removed" | "checklist_promoted" | "deleted"
    pub kind: String,
    pub actor: ActorDto,
    /// Typed-per-verb payload; schema varies by `kind`.
    pub payload: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------

/// Request body for `POST /v1/workspaces/{ws}/projects/{ps}/boards`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateBoardRequest {
    pub name: String,
}

/// Request body for `PATCH /v1/workspaces/{ws}/boards/{board_id}`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateBoardRequest {
    pub name: Option<String>,
}

/// Request body for `POST /v1/workspaces/{ws}/boards/{board_id}/columns`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateColumnRequest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// Request body for `PATCH /v1/workspaces/{ws}/boards/{board_id}/columns/{column_id}`.
///
/// Color uses the same `present_value` convention as `UpdateTaskRequest` fields:
/// an absent `color` key leaves the color unchanged; explicit `null` clears it;
/// a string value sets it. `name`, `before`, and `after` remain simple `Option<String>`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateColumnRequest {
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

/// Request body for `POST /v1/workspaces/{ws}/boards/{board_id}/tasks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateTaskRequest {
    pub column_id: uuid::Uuid,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<TaskPropertiesDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// Captures field presence for nullable patch fields: an absent field stays
/// `None` (leave unchanged), while an explicit JSON `null` becomes
/// `Some(Value::Null)` so a caller can clear the value. Without this, serde
/// collapses `null` into `None` and clearing a field is impossible.
fn present_value<'de, D>(de: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(de).map(Some)
}

/// Request body for `PATCH /v1/workspaces/{ws}/tasks/{readable_id}`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateTaskRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub priority: Option<serde_json::Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub due_date: Option<serde_json::Value>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_value"
    )]
    pub estimate: Option<serde_json::Value>,
    pub labels: Option<Vec<String>>,
    pub properties: Option<serde_json::Value>,
}

/// Request body for `POST /v1/workspaces/{ws}/tasks/{readable_id}/move`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MoveTaskRequest {
    pub column_id: uuid::Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// Request body for `POST /v1/workspaces/{ws}/tasks/{readable_id}/assignees`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AddAssigneeRequest {
    /// "user" | "api_key"
    pub assignee_type: String,
    pub assignee_id: uuid::Uuid,
}

/// Request body for `POST /v1/workspaces/{ws}/tasks/{readable_id}/references`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateReferenceRequest {
    /// "relates" | "blocks" | "parent" | "spec"
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_task_readable_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_document_id: Option<uuid::Uuid>,
}

/// Request body for `POST /v1/workspaces/{ws}/tasks/{readable_id}/subtasks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateSubtaskRequest {
    pub title: String,
}

/// Request body for `POST /v1/workspaces/{ws}/tasks/{readable_id}/checklist`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateChecklistItemRequest {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// Request body for `PATCH /v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateChecklistItemRequest {
    pub title: Option<String>,
    pub checked: Option<bool>,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// Request body for
/// `POST /v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PromoteChecklistItemRequest {
    /// Target board for the new task.
    pub board_id: uuid::Uuid,
    /// Target column for the new task.
    pub column_id: uuid::Uuid,
}

// ---------------------------------------------------------------------------
// Workspace-scoped task listing
// ---------------------------------------------------------------------------

/// Query parameters for `GET /v1/workspaces/{ws}/tasks`.
///
/// All fields are optional. Absent fields apply no filter (workspace-wide listing).
/// Used by both the client library and the test harness.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceTaskQueryParams {
    /// `me` | `user:{uuid}` | `api_key:{uuid}`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,

    /// `user` | `api_key` — restrict to tasks created by this actor type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,

    /// Restrict to tasks in these columns (repeated param).
    #[serde(default, rename = "column_id", skip_serializing_if = "Vec::is_empty")]
    pub column_ids: Vec<String>,

    /// Restrict to tasks with these priorities (repeated param).
    #[serde(default, rename = "priority", skip_serializing_if = "Vec::is_empty")]
    pub priorities: Vec<String>,

    /// Restrict to tasks carrying ALL of these labels (array-contains).
    #[serde(default, rename = "label", skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,

    /// Scope to a single board.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,

    /// Sort key — see D8 whitelist in design. Defaults to `updated_at_desc`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,

    /// Opaque pagination cursor (34-char `SearchCursor` wire format).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,

    /// Page size (1–200, default 50).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}
