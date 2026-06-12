use crate::{
    DomainError, WorkspaceCtx,
    entities::workspace_core::{
        Folder, NewFolder, NewProject, NewPropertyDefinition, Project, PropertyDefinition,
        UpdateProject,
    },
    ids::{FolderId, ProjectId, PropertyDefinitionId},
    permissions::Principal,
};
use async_trait::async_trait;

#[async_trait]
pub trait PropertyDefinitionRepo: Send + Sync {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewPropertyDefinition,
    ) -> Result<PropertyDefinition, DomainError>;
    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: PropertyDefinitionId,
    ) -> Result<Option<PropertyDefinition>, DomainError>;
    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<PropertyDefinition>, DomainError>;
    async fn soft_delete(
        &self,
        ctx: &WorkspaceCtx,
        id: PropertyDefinitionId,
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait ProjectRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewProject) -> Result<Project, DomainError>;
    async fn find(&self, ctx: &WorkspaceCtx, id: ProjectId)
    -> Result<Option<Project>, DomainError>;
    async fn find_by_slug(
        &self,
        ctx: &WorkspaceCtx,
        slug: &str,
    ) -> Result<Option<Project>, DomainError>;
    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<Project>, DomainError>;
    /// List projects visible to a principal: private projects only appear if the principal
    /// has an explicit grant; workspace-visibility and public projects are always included.
    async fn list_visible(
        &self,
        ctx: &WorkspaceCtx,
        principal: &Principal,
        after_id: Option<uuid::Uuid>,
        limit: u64,
    ) -> Result<Vec<Project>, DomainError>;
    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: ProjectId,
        name: String,
    ) -> Result<(), DomainError>;
    async fn update(
        &self,
        ctx: &WorkspaceCtx,
        id: ProjectId,
        update: UpdateProject,
    ) -> Result<Project, DomainError>;
    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: ProjectId) -> Result<(), DomainError>;
}

#[async_trait]
pub trait FolderRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewFolder) -> Result<Folder, DomainError>;
    async fn find(&self, ctx: &WorkspaceCtx, id: FolderId) -> Result<Option<Folder>, DomainError>;
    async fn list_children(
        &self,
        ctx: &WorkspaceCtx,
        parent: Option<FolderId>,
    ) -> Result<Vec<Folder>, DomainError>;
    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: FolderId,
        name: String,
    ) -> Result<(), DomainError>;
    async fn move_to(
        &self,
        ctx: &WorkspaceCtx,
        id: FolderId,
        new_parent: Option<FolderId>,
    ) -> Result<(), DomainError>;
    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: FolderId) -> Result<(), DomainError>;
}
