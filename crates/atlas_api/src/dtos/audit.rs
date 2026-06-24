use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use super::documents::ActorDto;

/// A single security audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AuditEntryDto {
    pub id: uuid::Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<uuid::Uuid>,
    pub actor: ActorDto,
    pub action: String,
    pub target_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<uuid::Uuid>,
    /// Human-readable label for the target (e.g. user display_name or api key name),
    /// resolved cheaply when target_type is "user" or "api_key".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
