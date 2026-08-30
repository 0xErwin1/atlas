use crate::{
    actor::WorkspaceCtx,
    entities::identity::{MemberRole, NewWorkspace, Workspace, WorkspaceMembership},
    ids::WorkspaceId,
};
use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::principal::{ApiKeyId, UserId};

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
