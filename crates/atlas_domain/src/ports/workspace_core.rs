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
    /// All non-deleted folders in the workspace, regardless of nesting depth.
    /// Callers that build a folder tree need every folder, not just root-level
    /// children.
    async fn list_all(&self, ctx: &WorkspaceCtx) -> Result<Vec<Folder>, DomainError>;
    /// A keyset page of a project's non-deleted folders ordered by id, returning
    /// folders with `id > after_id` (or from the start when `None`), capped at
    /// `limit`. Used by the paginated folder list so a project's whole tree can be
    /// fetched page by page.
    async fn list_paginated_by_project(
        &self,
        ctx: &WorkspaceCtx,
        project_id: ProjectId,
        after_id: Option<FolderId>,
        limit: u64,
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
