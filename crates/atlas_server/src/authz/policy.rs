//! The permission-grant cluster: `PermissionGrant` and its repository port, and
//! the `resolve()` role-resolution algorithm over a resource chain.
//!
//! Relocated from `atlas_domain` (S2d D1). `PermissionGrant` carries five Acta
//! resource ids as typed fields (`project_id`, `folder_id`, `document_id`,
//! `board_id`, plus `workspace_id`) — the exact type-level mirror of the FK
//! columns S3c collapses into `resource_ref` — so moving it into `atlas_custos`
//! as-is would reintroduce the forbidden `custos -> acta` edge. Grant
//! resolution over Acta chains **is** composition today; it belongs at the
//! `atlas_server` composition layer until S3b lifts it into Custos behind a
//! `ResourceProvider` port. No resolution logic changed in this move.

use async_trait::async_trait;
use atlas_acta::entities::identity::MemberRole;
use atlas_acta::ids::BoardId;
use atlas_acta::ids::DocumentId;
use atlas_acta::ids::FolderId;
use atlas_acta::ids::ProjectId;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::permissions::ResourceRef;
use atlas_acta::permissions::Visibility;
use atlas_core::error::DomainError;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::GroupId;
use atlas_core::principal::{Principal, UserId};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PermissionGrantId(pub Uuid);

impl PermissionGrantId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for PermissionGrantId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct PermissionGrant {
    pub id: PermissionGrantId,
    pub workspace_id: WorkspaceId,
    pub user_id: Option<UserId>,
    pub api_key_id: Option<ApiKeyId>,
    pub group_id: Option<GroupId>,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
    pub document_id: Option<DocumentId>,
    pub board_id: Option<BoardId>,
    pub role: ResourceRole,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPermissionGrant {
    pub workspace_id: WorkspaceId,
    pub user_id: Option<UserId>,
    pub api_key_id: Option<ApiKeyId>,
    pub group_id: Option<GroupId>,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
    pub document_id: Option<DocumentId>,
    pub board_id: Option<BoardId>,
    pub role: ResourceRole,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
}

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

#[derive(Debug, Clone)]
pub struct ChainSegment {
    pub resource: ResourceRef,
    pub visibility: Option<Visibility>,
}

/// Most-specific-first ordered list of resource segments ending with Workspace.
pub struct ResourceChain {
    pub segments: Vec<ChainSegment>,
}

pub struct ResolutionInput<'a> {
    pub principal: &'a Principal,
    /// None for ApiKey principals.
    pub membership: Option<MemberRole>,
    pub chain: &'a ResourceChain,
    /// Applicable grants loaded from the DB for this principal and chain.
    pub grants: &'a [(ResourceRef, ResourceRole)],
}

/// Determines the effective role for a principal on the most-specific resource in the chain.
///
/// Rules applied in order:
/// 1. Implicit admin: workspace Owner/Admin membership → Admin immediately.
/// 2. Walk chain most-specific-first; at each segment collect candidates (explicit grant +
///    visibility contribution for member users). First segment with candidates wins; max taken.
/// 3. Workspace-scope grants are the last segment (least specific).
/// 4. Agent cap: ApiKey result is capped at Editor.
/// 5. Default deny: no candidates → None.
pub fn resolve(input: &ResolutionInput<'_>) -> Option<ResourceRole> {
    // Rule 1: implicit admin for workspace owner/admin (users only).
    if matches!(input.principal, Principal::User(_))
        && matches!(
            input.membership,
            Some(MemberRole::Owner | MemberRole::Admin)
        )
    {
        return Some(ResourceRole::Admin);
    }

    // Rule 2-3: walk chain most-specific-first.
    for segment in &input.chain.segments {
        let mut candidates: Vec<ResourceRole> = Vec::new();

        // Collect explicit grant for this segment.
        for (grant_ref, grant_role) in input.grants {
            if grant_ref == &segment.resource {
                candidates.push(*grant_role);
            }
        }

        // Visibility contribution: only for User principals with workspace membership.
        // Group is a grant target, not an auth principal, so it never contributes visibility.
        if matches!(input.principal, Principal::User(_))
            && input.membership.is_some()
            && let Some(vis) = &segment.visibility
        {
            match vis {
                Visibility::Workspace(vis_role) | Visibility::Public(vis_role) => {
                    candidates.push(visibility_role_to_resource_role(vis_role));
                }
                Visibility::Private => {}
            }
        }

        if !candidates.is_empty() {
            let max = candidates.into_iter().max();
            return apply_agent_cap(input.principal, max);
        }
    }

    None
}

fn visibility_role_to_resource_role(vis: &atlas_acta::permissions::VisibilityRole) -> ResourceRole {
    use atlas_acta::permissions::VisibilityRole;
    match vis {
        VisibilityRole::Viewer => ResourceRole::Viewer,
        VisibilityRole::Editor => ResourceRole::Editor,
    }
}

fn apply_agent_cap(principal: &Principal, role: Option<ResourceRole>) -> Option<ResourceRole> {
    match principal {
        Principal::ApiKey(_) => role.map(|r| r.min(ResourceRole::Editor)),
        Principal::User(_) | Principal::Group(_) => role,
    }
}

/// `ResourceRole`, `ShareDenied`, and the grant-authorization guards, relocated
/// from `atlas_domain` (S2e). They stay in `atlas_server::authz` alongside
/// `resolve()`, the only other consumer of these types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceRole {
    Viewer,
    Editor,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareDenied {
    AgentsNeverManageGrants,
    RoleExceedsGrantors,
    InsufficientRoleToShare,
    AgentCannotBeAdmin,
}

/// Enforces the agent cap at grant write time: an ApiKey principal can never be
/// the target of an `Admin` grant. The cap is also applied at resolution time,
/// but rejecting here prevents persisting a grant row that misrepresents the
/// agent's effective role.
pub fn authorize_grant_target(
    target: &Principal,
    role_in_play: ResourceRole,
) -> Result<(), ShareDenied> {
    if matches!(target, Principal::ApiKey(_)) && role_in_play == ResourceRole::Admin {
        return Err(ShareDenied::AgentCannotBeAdmin);
    }

    Ok(())
}

/// Determines whether a principal with the given effective role may manage a grant for `role_in_play`.
pub fn authorize_share(
    actor: &Principal,
    actor_effective: ResourceRole,
    role_in_play: ResourceRole,
) -> Result<(), ShareDenied> {
    if matches!(actor, Principal::ApiKey(_) | Principal::Group(_)) {
        return Err(ShareDenied::AgentsNeverManageGrants);
    }

    if actor_effective < ResourceRole::Editor {
        return Err(ShareDenied::InsufficientRoleToShare);
    }

    if role_in_play > actor_effective {
        return Err(ShareDenied::RoleExceedsGrantors);
    }

    Ok(())
}
