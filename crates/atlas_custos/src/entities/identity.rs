use crate::WorkspaceScope;
use crate::capability::Capability;
use crate::ids::{ActivationTokenId, ApiKeyId, SessionId, UserId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    /// `None` for a pending (uninvited) account that has not yet set a password.
    pub password_hash: Option<String>,
    pub is_root: bool,
    pub is_system_admin: bool,
    pub disabled_at: Option<DateTime<Utc>>,
    /// `None` means the account is pending activation. `Some` means activated.
    pub activated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewUser {
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    /// `None` when creating a pending account (no credential yet).
    pub password_hash: Option<String>,
    pub is_root: bool,
    pub is_system_admin: bool,
}

/// A single-use activation token minted when creating a pending account.
///
/// The token hash is stored at rest; the plaintext is returned once to the
/// caller and never persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivationToken {
    pub id: ActivationTokenId,
    pub user_id: UserId,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub consumed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewActivationToken {
    pub user_id: UserId,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
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
    /// Deprecated binding to a single workspace scope. `None` for keys created
    /// after migration 020. Access is now determined exclusively by
    /// `permission_grants`. Opaque `WorkspaceScope` (D2) — Custos never
    /// dereferences this to a `Workspace` entity.
    pub workspace_id: Option<WorkspaceScope>,
    pub created_by_user_id: UserId,
    pub name: String,
    pub token_hash: String,
    pub type_: ApiKeyType,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// When true, the key inherits its creator's reach across every workspace
    /// (capped at editor and never above the creator's own role), instead of
    /// being limited to workspaces where it holds an explicit grant.
    pub is_global: bool,
    /// The capabilities this key may exercise, gated on top of (never above)
    /// its resolved role. Empty means the key can read and write nothing.
    pub scopes: Vec<Capability>,
}

#[derive(Debug, Clone)]
pub struct NewApiKey {
    pub name: String,
    pub token_hash: String,
    pub type_: ApiKeyType,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<Capability>,
}
