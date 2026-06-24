pub mod audit;
pub mod boards_tasks;
pub mod documents;
pub mod folders;
pub mod groups;
pub mod saved_searches;
pub mod search;
pub mod status_templates;
pub mod tags;
pub mod task_views;

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
    pub email: Option<String>,
    pub is_root: bool,
    pub is_system_admin: bool,
    pub disabled_at: Option<chrono::DateTime<chrono::Utc>>,
    /// `None` means the account has not yet been activated.
    pub activated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /v1/auth/change-password`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// Request body for `PATCH /v1/users/me`. Only the provided fields are updated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateMeRequest {
    pub email: Option<String>,
    pub display_name: Option<String>,
}

/// Request body for `POST /v1/users/{user_id}/reset-password`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ResetPasswordRequest {
    pub new_password: String,
}

/// Response from `GET /v1/auth/me`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MeResponse {
    pub principal_type: String,
    pub username: String,
    pub email: Option<String>,
    pub id: Option<uuid::Uuid>,
    pub display_name: Option<String>,
    pub is_root: bool,
    pub is_system_admin: bool,
}

/// Request body for `POST /v1/users/{user_id}/system-admin`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SetSystemAdminRequest {
    pub is_system_admin: bool,
}

/// Response from `GET /v1/meta`. Server build information for the About screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ServerMetaDto {
    pub version: String,
    pub build: Option<String>,
    /// Public base URL of this server, when configured (`ATLAS_SERVER_URL`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Request body for `POST /v1/users`.
///
/// Creates a pending account with no password. The returned `activation_link`
/// must be shared with the invitee so they can set their own password.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateUserRequest {
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub email: Option<String>,
    /// Workspace slug where the new user will be added.
    pub workspace: String,
    /// Membership role: `"admin"` or `"member"` (owner is rejected with 422).
    pub role: String,
}

/// Response from `POST /v1/users`.
///
/// `activation_link` is the plaintext single-use link shown exactly once.
/// It is not stored — only the hash is kept server-side.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateUserResponse {
    pub user: UserDto,
    /// Single-use activation path (e.g. `/activate/<token>`). Share this with the invitee.
    pub activation_link: String,
}

/// Response from `POST /v1/users/{user_id}/activation-link`.
///
/// `activation_link` is a freshly issued single-use link. Prior unconsumed tokens
/// for the same user are invalidated.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActivationLinkResponse {
    /// Single-use activation path (e.g. `/activate/<token>`). Share this with the invitee.
    pub activation_link: String,
}

/// Minimal user info returned by `GET /v1/activate/{token}` so the activation
/// page can display a personalised heading without requiring authentication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActivationInfoDto {
    pub username: String,
    pub display_name: String,
}

/// Request body for `POST /v1/activate/{token}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActivatePasswordRequest {
    pub password: String,
}

/// Optional initial workspace grant included in a `POST /v1/api-keys` request.
///
/// When present, a workspace-scope grant at the given role is created atomically
/// with the key so the key is immediately usable in that workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct InitialGrantRequest {
    /// Workspace slug for the initial grant.
    pub workspace: String,
    /// Role: `"viewer"` or `"editor"` (admin is rejected by the agent cap).
    pub role: String,
}

/// Request body for `POST /v1/api-keys` (top-level, user-owned key creation).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateUserApiKeyRequest {
    pub name: String,
    /// Key purpose: `"agent"` | `"cli"` | `"bot"` | `"integration"`. Defaults to `"agent"`.
    #[serde(default)]
    pub r#type: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Optional initial grant so the key is immediately usable in one workspace.
    #[serde(default)]
    pub initial_grant: Option<InitialGrantRequest>,
}

/// Response for `POST /v1/api-keys` (secret returned exactly once).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ApiKeyCreated {
    pub id: uuid::Uuid,
    pub name: String,
    /// The full `atlas_`-prefixed secret. Shown exactly once; not stored.
    pub secret: String,
    /// Key purpose: `"agent"` | `"cli"` | `"bot"` | `"integration"`.
    pub r#type: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Summary representation of an API key (no secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ApiKeyDto {
    pub id: uuid::Uuid,
    pub name: String,
    /// Key purpose: `"agent"` | `"cli"` | `"bot"` | `"integration"`.
    pub r#type: String,
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
    pub task_prefix: Option<String>,
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

/// A principal (workspace member or agent) that a grant can be addressed to.
///
/// Returned by `GET /v1/workspaces/{ws}/members` so the share dialog can resolve
/// a human-readable name to the principal id required by a grant request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PrincipalDto {
    /// `"user"` for a workspace member, `"api_key"` for an agent.
    pub principal_type: String,
    pub id: uuid::Uuid,
    /// Display name: the user's display name, or the api key's name.
    pub display: String,
    /// For `api_key` principals: the key purpose (`"agent"` | `"cli"` | `"bot"` | `"integration"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_type: Option<String>,
    /// Workspace membership role (`"owner"` | `"admin"` | `"member"`).
    /// Present for `user` principals; absent for `api_key` principals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Account lifecycle state for user principals: `"active"`, `"deactivated"`, or `"pending"`.
    /// Absent for `api_key` principals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_status: Option<String>,
}

/// Request body for `PATCH /v1/workspaces/{ws}/members/{user_id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateMemberRoleRequest {
    pub role: String,
}

/// Request body for `POST /v1/workspaces`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateWorkspaceRequest {
    pub name: String,
}

/// Request body for `PATCH /v1/workspaces/{ws}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateWorkspaceRequest {
    pub name: String,
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

/// Response from `GET /v1/me/ui-state` and `PUT /v1/me/ui-state`.
///
/// `state` is an opaque JSON object owned by the client (e.g. which sidebar
/// folders are collapsed). The server stores and returns it verbatim and does
/// not validate its inner shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UiStateDto {
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub state: serde_json::Value,
}

/// Request body for `PUT /v1/me/ui-state`. The `state` is stored verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateUiStateRequest {
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub state: serde_json::Value,
}

/// A single grant belonging to an API key, with resolved resource labels.
///
/// Returned by `GET /v1/api-keys/{key_id}/grants` so the keys panel can display
/// human-readable resource names without additional lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ApiKeyGrantDto {
    pub id: uuid::Uuid,
    /// `"viewer"` | `"editor"`
    pub role: String,
    /// `"workspace"` | `"project"` | `"folder"` | `"document"` | `"board"`
    pub resource_kind: String,
    /// Human-readable label: workspace/project name, or id+kind for sub-resources.
    pub resource_label: String,
    /// Workspace slug (always present — all grants live inside a workspace).
    pub workspace_slug: String,
    /// Project slug (present for project-scoped grants).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
}
