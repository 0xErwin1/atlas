use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Response from `GET /health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct HealthResponse {
    pub status: String,
}

/// Response from `GET /version`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VersionResponse {
    pub version: String,
}

/// Request body for `POST /v1/auth/login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Response body from `POST /v1/auth/login`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LoginResponse {
    /// Opaque session token — also delivered as an HttpOnly cookie.
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub user: UserDto,
}

/// Public user representation (no password hash).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UserDto {
    pub id: uuid::Uuid,
    pub username: String,
    pub display_name: String,
    pub is_root: bool,
    pub disabled_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Response from `GET /v1/auth/me`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MeResponse {
    pub principal_type: String,
    pub username: String,
}

/// Request body for `POST /v1/users`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateUserRequest {
    pub username: String,
    pub display_name: String,
    pub password: String,
}

/// Request body for `POST /v1/workspaces/{ws}/api-keys`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Response for `POST /v1/workspaces/{ws}/api-keys` (secret returned exactly once).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ApiKeyCreated {
    pub id: uuid::Uuid,
    pub name: String,
    /// The full `atlas_`-prefixed secret. Shown exactly once; not stored.
    pub secret: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Summary representation of an API key (no secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ApiKeyDto {
    pub id: uuid::Uuid,
    pub name: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /v1/workspaces/{ws}/projects`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateProjectRequest {
    pub name: String,
    pub slug: String,
    pub task_prefix: String,
    /// "private" | "workspace" | "public" (default: "workspace")
    pub visibility: Option<String>,
    /// "viewer" | "editor" (default: "editor")
    pub visibility_role: Option<String>,
}

/// Request body for `PATCH /v1/workspaces/{ws}/projects/{project_slug}`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub visibility: Option<String>,
    pub visibility_role: Option<String>,
}

/// Project representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ProjectDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    pub name: String,
    pub slug: String,
    pub task_prefix: String,
    pub visibility: String,
    pub visibility_role: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /v1/workspaces/{ws}/projects/{slug}/grants`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateGrantRequest {
    pub principal: GrantPrincipal,
    pub role: String,
}

/// Identifies a grant principal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrantPrincipal {
    pub r#type: String,
    pub id: uuid::Uuid,
}

/// Grant representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrantDto {
    pub id: uuid::Uuid,
    pub principal: GrantPrincipal,
    pub role: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Workspace representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkspaceDto {
    pub id: uuid::Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
