//! Server-owned state that is neither Custos (identity/security) nor Acta
//! (workspace content) in nature: per-user client preferences persisted by
//! the server but never reasoned about by either product crate.

use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::principal::UserId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Per-user UI state: an opaque JSON object the web app persists across devices
/// (e.g. which sidebar folders are collapsed). One row per user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserUiState {
    pub user_id: UserId,
    pub state: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// Persistence for per-user UI state. Scoped to a single user (not a workspace),
/// so its methods take a `UserId` rather than a `WorkspaceCtx`.
#[async_trait]
pub trait UiStateRepo: Send + Sync {
    /// Returns the user's stored UI state, or `None` when no row exists yet.
    async fn find(&self, user_id: UserId) -> Result<Option<UserUiState>, DomainError>;
    /// Inserts or replaces the user's UI state, returning the stored row.
    async fn upsert(
        &self,
        user_id: UserId,
        state: serde_json::Value,
    ) -> Result<UserUiState, DomainError>;
}
