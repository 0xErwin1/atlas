use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Subscription representation returned by list, get, and update.
///
/// Never contains the HMAC secret or any ciphertext. The plaintext secret is
/// returned exactly once in `WebhookCreatedDto` at creation time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WebhookDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub target_url: String,
    /// Ordered list of event-type strings this subscription receives.
    pub event_types: Vec<String>,
    /// Scope discriminant: `"workspace"`, `"project"`, or `"board"`.
    pub scope_type: String,
    /// UUID of the project or board when `scope_type` is not `"workspace"`.
    pub scope_id: Option<uuid::Uuid>,
    pub is_active: bool,
    pub label: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Response from `POST /api/workspaces/{ws}/webhooks`.
///
/// Carries the HMAC secret in plaintext exactly once. After this response the
/// secret is never available again — store it immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WebhookCreatedDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub target_url: String,
    pub event_types: Vec<String>,
    pub scope_type: String,
    pub scope_id: Option<uuid::Uuid>,
    pub is_active: bool,
    pub label: Option<String>,
    /// Plaintext HMAC-SHA256 signing secret (`whsec_…`). Shown exactly once.
    pub secret: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /api/workspaces/{ws}/webhooks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateWebhookRequest {
    /// Absolute HTTPS (or HTTP for local testing) URL to POST events to.
    pub target_url: String,
    /// At least one event-type string is required.
    pub event_types: Vec<String>,
    /// Scope discriminant: `"workspace"` (default), `"project"`, or `"board"`.
    #[serde(default = "default_scope_type")]
    pub scope_type: String,
    /// Required when `scope_type` is `"project"` or `"board"`.
    pub scope_id: Option<uuid::Uuid>,
    /// Optional human-readable label for the subscription.
    pub label: Option<String>,
}

fn default_scope_type() -> String {
    "workspace".to_string()
}

/// Request body for `PATCH /api/workspaces/{ws}/webhooks/{webhook_id}`.
///
/// All fields are optional; omitted fields are left unchanged.
/// The secret is never changed via this endpoint (no rotation endpoint in v1).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateWebhookRequest {
    pub target_url: Option<String>,
    pub event_types: Option<Vec<String>>,
    pub scope_type: Option<String>,
    pub scope_id: Option<Option<uuid::Uuid>>,
    pub is_active: Option<bool>,
    pub label: Option<Option<String>>,
}

/// One delivery-attempt log entry for a subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WebhookDeliveryDto {
    pub id: uuid::Uuid,
    pub subscription_id: uuid::Uuid,
    pub outbox_event_id: uuid::Uuid,
    pub attempt_no: i32,
    /// `"success"` or `"failure"`.
    pub outcome: String,
    pub status_code: Option<i32>,
    pub response_snippet: Option<String>,
    pub error: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
