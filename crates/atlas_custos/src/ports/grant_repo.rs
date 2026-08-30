use crate::WorkspaceScope;
use crate::entities::permissions::{
    NewPermissionGrant, PermissionGrant, PermissionGrantId, ResourceRole,
};
use crate::ids::ApiKeyId;
use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::ids::ResourceRef;
use uuid::Uuid;

/// Parameters for the hot-path grant resolution query.
///
/// `resource_refs` is the full set of resources this query may match, already
/// encoded as `atlas_core::ids::ResourceRef` by the caller — composition
/// (`atlas_server`) owns the Acta chain walk and the codec conversion at the
/// boundary, so this port never needs to know about a specific resource kind.
pub struct ResolutionQuery {
    pub workspace_id: WorkspaceScope,
    /// Set for User principals.
    pub user_id: Option<Uuid>,
    /// Set for ApiKey principals.
    pub api_key_id: Option<Uuid>,
    /// Group IDs the user belongs to in this workspace.
    pub group_ids: Vec<Uuid>,
    /// Every resource ref in the chain that might carry a grant, most-specific
    /// first, always including the workspace-scope ref.
    pub resource_refs: Vec<ResourceRef>,
}

#[async_trait]
pub trait PermissionGrantRepo: Send + Sync {
    /// Insert or update a grant (upsert on the unique principal+resource key).
    async fn upsert(&self, grant: NewPermissionGrant) -> Result<PermissionGrant, DomainError>;

    /// Load all grants applicable to a principal for a given chain of resource refs.
    async fn load_grants_for_resolution(
        &self,
        query: ResolutionQuery,
    ) -> Result<Vec<(ResourceRef, ResourceRole)>, DomainError>;

    /// Delete a specific grant by ID (scoped to the workspace for tenancy).
    async fn delete(
        &self,
        grant_id: PermissionGrantId,
        workspace_id: WorkspaceScope,
    ) -> Result<(), DomainError>;

    /// List grants for a specific resource (cursor-paginated).
    async fn list_for_resource(
        &self,
        workspace_id: WorkspaceScope,
        resource: &ResourceRef,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<PermissionGrant>, DomainError>;

    /// Find a grant by id, scoped to the workspace and the resource it was
    /// issued for. Returns `None` when the grant does not exist, belongs to a
    /// different workspace, or targets a different resource.
    async fn find_by_id(
        &self,
        workspace_id: WorkspaceScope,
        resource: &ResourceRef,
        grant_id: PermissionGrantId,
    ) -> Result<Option<PermissionGrant>, DomainError>;

    /// List all grants that belong to a specific API key, across all workspaces.
    async fn list_for_api_key(
        &self,
        api_key_id: ApiKeyId,
    ) -> Result<Vec<PermissionGrant>, DomainError>;

    /// Delete a grant by its id, ownership-checked — the grant must belong to the
    /// given api_key_id. Returns Ok(false) when the grant was not found or does not
    /// belong to the key (caller should treat that as 404).
    async fn delete_for_api_key(
        &self,
        grant_id: PermissionGrantId,
        api_key_id: ApiKeyId,
    ) -> Result<bool, DomainError>;
}
