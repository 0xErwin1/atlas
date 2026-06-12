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
}
