//! The `resolve()` role-resolution algorithm over a resource chain, and the
//! grant-authorization guards built on top of it.
//!
//! The permission-grant cluster itself (`PermissionGrant`, `NewPermissionGrant`,
//! `ResolutionQuery`, `PermissionGrantRepo`, `ResourceRole`) was relocated from
//! `atlas_domain` (S2d D1) and parked here through S3a/S3b: it carried five
//! Acta resource ids as typed fields (`project_id`, `folder_id`, `document_id`,
//! `board_id`, plus `workspace_id`), so moving it into `atlas_custos` as-is
//! would have reintroduced the forbidden `custos -> acta` edge. S3c's
//! `resource_ref`/`WorkspaceScope` collapse removed the last Acta type from
//! the cluster, and T6.9 moved it into `atlas_custos` (+ the repo impl into
//! `atlas_custos_postgres`). The re-exports below keep every existing
//! `crate::authz::policy::{...}` import path resolving unchanged.
//!
//! `resolve()` itself stays here: it walks an Acta `ResourceChain` and reads
//! Acta's `MemberRole`, so it cannot move into Custos without reintroducing
//! that same edge. No resolution logic changed in this move.

use atlas_acta::entities::identity::MemberRole;
use atlas_acta::permissions::ResourceRef;
use atlas_acta::permissions::Visibility;
use atlas_core::principal::Principal;

pub use atlas_custos::entities::permissions::{
    NewPermissionGrant, PermissionGrant, PermissionGrantId, ResourceRole,
};
pub use atlas_custos::ports::grant_repo::{PermissionGrantRepo, ResolutionQuery};

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

/// `ShareDenied` and the grant-authorization guards, relocated from
/// `atlas_domain` (S2e). They stay in `atlas_server::authz` alongside
/// `resolve()`, the only other consumer of `ResourceRole` outside the moved
/// grant cluster (T6.9).
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
