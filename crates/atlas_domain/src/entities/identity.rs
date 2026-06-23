use crate::ids::{ApiKeyId, MembershipId, SessionId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewWorkspace {
    pub id: WorkspaceId,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub password_hash: String,
    pub is_root: bool,
    pub is_system_admin: bool,
    pub disabled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewUser {
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub password_hash: String,
    pub is_root: bool,
    pub is_system_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub user_id: UserId,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub user_id: UserId,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

/// The declared purpose of an API key. Does not vary the agent cap (always ≤ editor);
/// stored for attribution and future per-type policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ApiKeyType {
    #[default]
    Agent,
    Cli,
    Bot,
    Integration,
}

impl ApiKeyType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiKeyType::Agent => "agent",
            ApiKeyType::Cli => "cli",
            ApiKeyType::Bot => "bot",
            ApiKeyType::Integration => "integration",
        }
    }
}

impl std::str::FromStr for ApiKeyType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agent" => Ok(ApiKeyType::Agent),
            "cli" => Ok(ApiKeyType::Cli),
            "bot" => Ok(ApiKeyType::Bot),
            "integration" => Ok(ApiKeyType::Integration),
            other => Err(format!("unknown api key type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: ApiKeyId,
    /// Deprecated binding to a single workspace. `None` for keys created after migration 020.
    /// Access is now determined exclusively by `permission_grants`.
    pub workspace_id: Option<WorkspaceId>,
    pub created_by_user_id: UserId,
    pub name: String,
    pub token_hash: String,
    pub type_: ApiKeyType,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewApiKey {
    pub name: String,
    pub token_hash: String,
    pub type_: ApiKeyType,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberRole {
    Owner,
    Admin,
    Member,
}

impl MemberRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemberRole::Owner => "owner",
            MemberRole::Admin => "admin",
            MemberRole::Member => "member",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMembership {
    pub id: MembershipId,
    pub workspace_id: WorkspaceId,
    pub user_id: UserId,
    pub role: MemberRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Per-user UI state: an opaque JSON object the web app persists across devices
/// (e.g. which sidebar folders are collapsed). One row per user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserUiState {
    pub user_id: UserId,
    pub state: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}
