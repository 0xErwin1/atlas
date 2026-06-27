use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Workspace property definition (custom field) representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PropertyDefinitionDto {
    pub id: uuid::Uuid,
    /// Stable machine key derived from the name (`^[a-z][a-z0-9_]{0,63}$`).
    pub key: String,
    pub name: String,
    /// One of `text` | `number` | `boolean` | `date` | `select` | `multi_select`.
    pub kind: String,
    /// For `select`/`multi_select`: the allowed string options. Absent for the
    /// other kinds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    /// One of `document` | `task` | `both`.
    pub applies_to: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /v1/workspaces/{ws}/property-definitions`.
///
/// The `key` is derived server-side from `name`; callers do not supply it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreatePropertyDefinitionRequest {
    pub name: String,
    /// One of `text` | `number` | `boolean` | `date` | `select` | `multi_select`.
    pub kind: String,
    /// Required for `select`/`multi_select` (a non-empty array of unique
    /// non-empty strings); must be absent/null for the other kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<serde_json::Value>,
    /// One of `document` | `task` | `both`. Defaults to `task` when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applies_to: Option<String>,
}
