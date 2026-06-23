use crate::{
    DomainError, WorkspaceCtx,
    entities::identity::{
        ApiKey, MemberRole, NewApiKey, NewSession, NewUser, NewWorkspace, Session, User,
        UserUiState, Workspace, WorkspaceMembership,
    },
    ids::{ApiKeyId, SessionId, UserId, WorkspaceId},
};
use async_trait::async_trait;

#[async_trait]
pub trait WorkspaceRepo: Send + Sync {
    async fn create(&self, new: NewWorkspace) -> Result<Workspace, DomainError>;
    async fn find_by_id(&self, id: WorkspaceId) -> Result<Option<Workspace>, DomainError>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<Workspace>, DomainError>;
    async fn list_for_user(&self, user_id: UserId) -> Result<Vec<Workspace>, DomainError>;
    /// Returns the slugs of every workspace, used to resolve slug collisions
    /// when deriving a new workspace slug from its name.
    async fn list_slugs(&self) -> Result<Vec<String>, DomainError>;
    /// Updates the display name of a workspace. The slug is never re-derived;
    /// only `name` and `updated_at` change.
    async fn rename(&self, id: WorkspaceId, name: String) -> Result<Workspace, DomainError>;
    /// Returns every workspace in the system, ordered by `created_at` ascending.
    /// Intended for root/admin use only — the route layer enforces the guard.
    async fn list_all(&self) -> Result<Vec<Workspace>, DomainError>;
}

#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn create(&self, new: NewUser) -> Result<User, DomainError>;
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, DomainError>;
    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, DomainError>;
    async fn find_root(&self) -> Result<Option<User>, DomainError>;
    /// Lists all users (active and disabled), ordered by `created_at` ascending.
    async fn list(&self) -> Result<Vec<User>, DomainError>;
    /// Batch lookup by id, for resolving display names without one query per id.
    async fn list_by_ids(&self, ids: &[UserId]) -> Result<Vec<User>, DomainError>;
    async fn disable(&self, id: UserId) -> Result<(), DomainError>;
    async fn enable(&self, id: UserId) -> Result<(), DomainError>;
    /// Replaces the user's password hash and bumps `updated_at`.
    async fn set_password_hash(&self, id: UserId, hash: String) -> Result<(), DomainError>;
    /// Updates the user's self-service profile fields. Only `Some` fields are
    /// overwritten; `None` leaves the existing value untouched. Bumps `updated_at`
    /// and returns the updated user.
    async fn update_profile(
        &self,
        id: UserId,
        email: Option<String>,
        display_name: Option<String>,
    ) -> Result<User, DomainError>;
    /// Sets the `is_system_admin` flag for a user. Only root may call this path;
    /// the route layer enforces the guard.
    async fn set_system_admin(
        &self,
        id: UserId,
        is_system_admin: bool,
    ) -> Result<User, DomainError>;
}

#[async_trait]
pub trait SessionRepo: Send + Sync {
    async fn create(&self, new: NewSession) -> Result<Session, DomainError>;
    async fn find_active_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<Session>, DomainError>;
    async fn revoke(&self, id: SessionId) -> Result<(), DomainError>;
    /// Revoke all active sessions for a user (used when disabling a user).
    async fn revoke_all_for_user(&self, user_id: UserId) -> Result<(), DomainError>;
    /// Revoke all active sessions for a user except the one the caller is currently
    /// authenticated with. Used on self password-change so stolen tokens are
    /// invalidated while the performing session stays alive.
    async fn revoke_all_for_user_except(
        &self,
        user_id: UserId,
        keep_session_id: SessionId,
    ) -> Result<(), DomainError>;
    /// Update last_used_at and slide expires_at by ttl_hours, capped at created_at + max_ttl_hours.
    /// Throttled: only writes if last_used_at is older than 60 seconds.
    async fn touch(
        &self,
        id: SessionId,
        ttl_hours: i64,
        max_ttl_hours: i64,
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait ApiKeyRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewApiKey) -> Result<ApiKey, DomainError>;
    async fn find_active_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiKey>, DomainError>;
    async fn revoke(&self, ctx: &WorkspaceCtx, id: ApiKeyId) -> Result<(), DomainError>;
    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<ApiKey>, DomainError>;
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

#[async_trait]
pub trait MembershipRepo: Send + Sync {
    async fn add(
        &self,
        ctx: &WorkspaceCtx,
        user_id: UserId,
        role: MemberRole,
    ) -> Result<WorkspaceMembership, DomainError>;
    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        user_id: UserId,
    ) -> Result<Option<WorkspaceMembership>, DomainError>;
    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<WorkspaceMembership>, DomainError>;
    async fn remove(&self, ctx: &WorkspaceCtx, user_id: UserId) -> Result<(), DomainError>;
}
