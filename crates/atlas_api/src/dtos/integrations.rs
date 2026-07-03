use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Request body for `POST /v1/workspaces/{ws}/integration-configs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateIntegrationConfigRequest {
    /// Integration slug. Only `"github"` is supported in v1.
    pub integration: String,
}

/// Integration config as returned by list and get.
///
/// Never contains the plaintext or encrypted HMAC secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct IntegrationConfigDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub integration: String,
    /// Provisioned `ApiKeyType::Integration` key used as the automation actor.
    pub integration_api_key_id: uuid::Uuid,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `PATCH /v1/workspaces/{ws}/integration-configs/{config_id}`.
///
/// Only `is_active` can be changed; the integration slug and secret are fixed at
/// creation. Omitting `is_active` leaves the config unchanged. Setting it to
/// `false` makes the inbound ingest reject events for this integration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateIntegrationConfigRequest {
    pub is_active: Option<bool>,
}

/// Response from `POST /v1/workspaces/{ws}/integration-configs`.
///
/// Carries the HMAC secret in plaintext exactly once. After this response the
/// secret is never available again — configure your integration immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct IntegrationConfigCreatedDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub integration: String,
    pub integration_api_key_id: uuid::Uuid,
    pub is_active: bool,
    /// Plaintext HMAC-SHA256 signing secret (`integ_…`). Shown exactly once.
    pub secret: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
