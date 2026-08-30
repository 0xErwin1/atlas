use crate::ids::{MembershipId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// User/session/activation/api-key entities relocated to
/// `atlas_custos::entities::identity` (S2d). Re-exported here to keep every
/// existing `crate::entities::identity::*` import path compiling.
pub use atlas_custos::entities::identity::{
    ActivationToken, ApiKey, ApiKeyType, NewActivationToken, NewApiKey, NewSession, NewUser,
    Session, User,
};

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
