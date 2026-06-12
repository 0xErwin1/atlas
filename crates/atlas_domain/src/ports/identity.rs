use crate::{
    DomainError, WorkspaceCtx,
    entities::identity::{
        ApiKey, MemberRole, NewApiKey, NewSession, NewUser, NewWorkspace, Session, User, Workspace,
        WorkspaceMembership,
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
}

#[async_trait]
pub trait UserRepo: Send + Sync {
    async fn create(&self, new: NewUser) -> Result<User, DomainError>;
    async fn find_by_username(&self, username: &str) -> Result<Option<User>, DomainError>;
    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, DomainError>;
    async fn find_root(&self) -> Result<Option<User>, DomainError>;
}

#[async_trait]
pub trait SessionRepo: Send + Sync {
    async fn create(&self, new: NewSession) -> Result<Session, DomainError>;
    async fn find_active_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<Session>, DomainError>;
    async fn revoke(&self, id: SessionId) -> Result<(), DomainError>;
    async fn touch_last_used(&self, id: SessionId) -> Result<(), DomainError>;
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
