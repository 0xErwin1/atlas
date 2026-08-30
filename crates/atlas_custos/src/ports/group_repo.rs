use crate::WorkspaceScope;
use crate::entities::groups::{Group, GroupMember, NewGroup};
use crate::ids::{GroupId, UserId};
use async_trait::async_trait;
use atlas_core::error::DomainError;

#[async_trait]
pub trait GroupRepo: Send + Sync {
    async fn create(&self, group: NewGroup) -> Result<Group, DomainError>;

    async fn get(&self, id: GroupId, scope: WorkspaceScope) -> Result<Option<Group>, DomainError>;

    async fn list(&self, scope: WorkspaceScope) -> Result<Vec<Group>, DomainError>;

    async fn soft_delete(&self, id: GroupId, scope: WorkspaceScope) -> Result<bool, DomainError>;

    async fn add_member(
        &self,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<GroupMember, DomainError>;

    async fn remove_member(&self, group_id: GroupId, user_id: UserId) -> Result<bool, DomainError>;

    async fn list_members(&self, group_id: GroupId) -> Result<Vec<GroupMember>, DomainError>;
}
