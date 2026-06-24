use crate::DomainError;
use crate::entities::groups::{Group, GroupMember, NewGroup};
use crate::ids::{GroupId, UserId, WorkspaceId};
use async_trait::async_trait;

#[async_trait]
pub trait GroupRepo: Send + Sync {
    async fn create(&self, group: NewGroup) -> Result<Group, DomainError>;

    async fn get(
        &self,
        id: GroupId,
        workspace_id: WorkspaceId,
    ) -> Result<Option<Group>, DomainError>;

    async fn list(&self, workspace_id: WorkspaceId) -> Result<Vec<Group>, DomainError>;

    async fn soft_delete(
        &self,
        id: GroupId,
        workspace_id: WorkspaceId,
    ) -> Result<bool, DomainError>;

    async fn add_member(
        &self,
        group_id: GroupId,
        user_id: UserId,
    ) -> Result<GroupMember, DomainError>;

    async fn remove_member(&self, group_id: GroupId, user_id: UserId) -> Result<bool, DomainError>;

    async fn list_members(&self, group_id: GroupId) -> Result<Vec<GroupMember>, DomainError>;
}
