use crate::DomainError;
use crate::entities::permissions::{NewPermissionGrant, PermissionGrant, PermissionGrantId};
use crate::ids::WorkspaceId;
use crate::permissions::{ResourceRef, ResourceRole};
use async_trait::async_trait;
use uuid::Uuid;

/// Parameters for the hot-path grant resolution query.
pub struct ResolutionQuery {
    pub workspace_id: WorkspaceId,
    /// Set for User principals.
    pub user_id: Option<Uuid>,
    /// Set for ApiKey principals.
    pub api_key_id: Option<Uuid>,
    /// Group IDs the user belongs to in this workspace.
    /// Populated by B2 (build_resolution_query); defaults empty — no group grants gathered.
    pub group_ids: Vec<Uuid>,
    pub chain_projects: Vec<Uuid>,
    pub chain_folders: Vec<Uuid>,
    pub doc_id: Option<Uuid>,
    pub board_id: Option<Uuid>,
}

#[async_trait]
pub trait PermissionGrantRepo: Send + Sync {
    /// Insert or update a grant (upsert on the unique principal+resource key).
    async fn upsert(&self, grant: NewPermissionGrant) -> Result<PermissionGrant, DomainError>;

    /// Load all grants applicable to a principal for a given chain of resource IDs.
    async fn load_grants_for_resolution(
        &self,
        query: ResolutionQuery,
    ) -> Result<Vec<(ResourceRef, ResourceRole)>, DomainError>;

    /// Delete a specific grant by ID (scoped to the workspace for tenancy).
    async fn delete(
        &self,
        grant_id: PermissionGrantId,
        workspace_id: WorkspaceId,
    ) -> Result<(), DomainError>;

    /// List grants for a specific resource (cursor-paginated).
    async fn list_for_resource(
        &self,
        workspace_id: WorkspaceId,
        resource: &ResourceRef,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<PermissionGrant>, DomainError>;

    /// Find a grant by id, scoped to the workspace and the resource it was
    /// issued for. Returns `None` when the grant does not exist, belongs to a
    /// different workspace, or targets a different resource.
    async fn find_by_id(
        &self,
        workspace_id: WorkspaceId,
        resource: &ResourceRef,
        grant_id: PermissionGrantId,
    ) -> Result<Option<PermissionGrant>, DomainError>;

    /// List all grants that belong to a specific API key, across all workspaces.
    async fn list_for_api_key(
        &self,
        api_key_id: crate::ids::ApiKeyId,
    ) -> Result<Vec<PermissionGrant>, DomainError>;

    /// Delete a grant by its id, ownership-checked — the grant must belong to the
    /// given api_key_id. Returns Ok(false) when the grant was not found or does not
    /// belong to the key (caller should treat that as 404).
    async fn delete_for_api_key(
        &self,
        grant_id: PermissionGrantId,
        api_key_id: crate::ids::ApiKeyId,
    ) -> Result<bool, DomainError>;
}
