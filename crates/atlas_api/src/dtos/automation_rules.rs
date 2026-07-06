use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Request body for `POST /api/workspaces/{ws}/automation-rules`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateAutomationRuleRequest {
    pub name: String,
    /// Must start with `"external."` (e.g. `"external.github.workflow_run"`).
    pub trigger_event_type: String,
    /// Optional top-level string-equality filter. All keys must match for the rule to fire.
    #[serde(default)]
    pub trigger_filter: Option<serde_json::Value>,
    /// Reserved for future project-scoped automation; must be omitted in v1.
    #[serde(default)]
    pub project_id: Option<uuid::Uuid>,
    /// Action to execute on match. Only `"create_task"` is supported in v1.
    pub action_type: String,
    /// Parameters for the action. For `"create_task"`: `{board_id, column_id, title_template, priority?}`.
    pub action_params: serde_json::Value,
}

/// Partial update for `PATCH /api/workspaces/{ws}/automation-rules/{rule_id}`.
///
/// Omitted fields are left unchanged. Pass `{"trigger_filter": null}` to clear the filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PatchAutomationRuleRequest {
    pub name: Option<String>,
    pub is_active: Option<bool>,
    /// `Some(null)` clears the filter; `Some(value)` replaces it; absent leaves it unchanged.
    #[serde(default)]
    pub trigger_filter: Option<Option<serde_json::Value>>,
    pub action_params: Option<serde_json::Value>,
}

/// Automation rule representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AutomationRuleDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub name: String,
    pub is_active: bool,
    pub trigger_event_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<uuid::Uuid>,
    pub action_type: String,
    pub action_params: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
