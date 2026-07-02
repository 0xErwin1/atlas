use crate::{
    DomainError, WorkspaceCtx,
    entities::identity::{
        ActivationToken, ApiKey, MemberRole, NewActivationToken, NewApiKey, NewSession, NewUser,
        NewWorkspace, Session, User, UserUiState, Workspace, WorkspaceMembership,
    },
    ids::{ActivationTokenId, ApiKeyId, SessionId, UserId, WorkspaceId},
};
use async_trait::async_trait;

#[async_trait]
pub trait WorkspaceRepo: Send + Sync {
    async fn create(&self, new: NewWorkspace) -> Result<Workspace, DomainError>;
    async fn find_by_id(&self, id: WorkspaceId) -> Result<Option<Workspace>, DomainError>;
    async fn find_by_slug(&self, slug: &str) -> Result<Option<Workspace>, DomainError>;
    async fn list_for_user(&self, user_id: UserId) -> Result<Vec<Workspace>, DomainError>;
    /// Returns every workspace the user is a member of, paired with the
    /// membership role. Unlike `list_for_user`, this carries the per-workspace
    /// role so an admin "workspace access" editor can show and assign a user's
    /// role across workspaces without switching the active workspace.
    async fn list_memberships_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<(Workspace, MemberRole)>, DomainError>;
    /// Returns the distinct workspaces where the api_key holds at least one
    /// permission grant. This is the grant-based equivalent of `list_for_user`
    /// for non-human principals.
    async fn list_for_api_key(&self, api_key_id: ApiKeyId) -> Result<Vec<Workspace>, DomainError>;
    /// Returns the slugs of every workspace, used to resolve slug collisions
    /// when deriving a new workspace slug from its name.
    async fn list_slugs(&self) -> Result<Vec<String>, DomainError>;
    /// Updates the display name of a workspace. The slug is never re-derived;
    /// only `name` and `updated_at` change.
    async fn rename(&self, id: WorkspaceId, name: String) -> Result<Workspace, DomainError>;
    /// Replaces the workspace slug with a caller-supplied value and bumps
    /// `updated_at`. The caller is responsible for validating the slug format and
    /// resolving collisions; this method performs the write only. Returns
    /// `DomainError::NotFound` when the workspace does not exist or is soft-deleted.
    async fn set_slug(&self, id: WorkspaceId, slug: String) -> Result<Workspace, DomainError>;
    /// Returns every live workspace in the system, ordered by `created_at`
    /// ascending. Soft-deleted workspaces are excluded. Intended for root/admin
    /// use only — the route layer enforces the guard.
    async fn list_all(&self) -> Result<Vec<Workspace>, DomainError>;
    /// Soft-deletes a workspace by stamping `deleted_at = now()`, hiding it from
    /// every lookup while preserving its rows. Returns `DomainError::NotFound`
    /// when the workspace does not exist or is already soft-deleted.
    async fn soft_delete(&self, id: WorkspaceId) -> Result<(), DomainError>;
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
    /// Sets the user's `password_hash` and `activated_at = now()`, completing
    /// the activation flow. Returns the updated user record.
    async fn activate(&self, id: UserId, password_hash: String) -> Result<User, DomainError>;
}

/// Persistence for single-use account activation tokens.
///
/// Tokens are stored as a SHA-256 hex hash (`token_hash`); the plaintext is
/// returned once by the create/regenerate route and never persisted.
/// The active predicate is `consumed_at IS NULL AND expires_at > now()`.
#[async_trait]
pub trait ActivationTokenRepo: Send + Sync {
    async fn create(&self, new: NewActivationToken) -> Result<ActivationToken, DomainError>;
    /// Returns `Some` only when the token is unconsumed and not yet expired.
    async fn find_active_by_token_hash(
        &self,
        hash: &str,
    ) -> Result<Option<ActivationToken>, DomainError>;
    /// Marks the token consumed (sets `consumed_at = now()`).
    async fn consume(&self, id: ActivationTokenId) -> Result<(), DomainError>;
    /// Invalidates every unconsumed token for the given user by setting
    /// `consumed_at = now()`. Called before issuing a fresh token (regenerate).
    async fn invalidate_unconsumed_for_user(&self, user_id: UserId) -> Result<(), DomainError>;
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
    /// Creates an API key owned by `user_id`, with `workspace_id = NULL`.
    /// Returns the created key (without the secret — the caller holds the plaintext).
    async fn create_for_user(&self, user_id: UserId, new: NewApiKey)
    -> Result<ApiKey, DomainError>;
    async fn find_active_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<ApiKey>, DomainError>;
    async fn revoke(&self, ctx: &WorkspaceCtx, id: ApiKeyId) -> Result<(), DomainError>;
    /// Revokes a key by id after verifying the caller owns it (or is root/system-admin).
    /// Returns `DomainError::NotFound` when the key does not exist or is already revoked.
    /// Returns `DomainError::Forbidden` when `user_id` does not own the key.
    async fn revoke_for_user(&self, user_id: UserId, id: ApiKeyId) -> Result<(), DomainError>;
    /// Lists non-revoked keys scoped to the workspace via the deprecated workspace_id FK.
    /// Kept for callers that have not yet migrated to grant-based listing (C2b).
    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<ApiKey>, DomainError>;
    /// Lists all non-revoked keys owned by the given user, across all workspaces.
    async fn list_for_user(&self, user_id: UserId) -> Result<Vec<ApiKey>, DomainError>;
    /// Looks up a single key by its id, regardless of workspace.
    async fn get_by_id(&self, id: ApiKeyId) -> Result<Option<ApiKey>, DomainError>;
    /// Batch lookup by id, regardless of workspace or revocation state.
    ///
    /// Mirrors `get_by_id`'s unscoped semantics for a set of ids, so attribution
    /// paths can resolve the names of global or later-revoked keys without the
    /// grant/revocation filter applied by `list_granted_in_workspace`.
    async fn list_by_ids(&self, ids: &[ApiKeyId]) -> Result<Vec<ApiKey>, DomainError>;
    /// Lists all non-revoked keys that hold at least one grant in the given workspace.
    async fn list_granted_in_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<ApiKey>, DomainError>;
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
    /// Updates the `role` of an existing membership and bumps `updated_at`.
    ///
    /// Returns the updated membership. Returns `DomainError::NotFound` when no
    /// membership row exists for `(ctx.workspace_id, user_id)` — including the
    /// window between a `find` and this call (see design note on the accepted race).
    async fn update_role(
        &self,
        ctx: &WorkspaceCtx,
        user_id: UserId,
        role: MemberRole,
    ) -> Result<WorkspaceMembership, DomainError>;
}
