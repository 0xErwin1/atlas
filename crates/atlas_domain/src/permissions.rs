use crate::entities::boards_tasks::ReferenceKind;
use crate::entities::identity::MemberRole;
use crate::error::DomainError;
use crate::ids::{ApiKeyId, BoardId, DocumentId, FolderId, ProjectId, TaskId, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceRole {
    Viewer,
    Editor,
    Admin,
}

#[derive(Debug, Clone)]
pub enum Principal {
    User(UserId),
    ApiKey(ApiKeyId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceRef {
    Workspace,
    Project(ProjectId),
    Folder(FolderId),
    Document(DocumentId),
    Board(BoardId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VisibilityRole {
    Viewer,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Private,
    Workspace(VisibilityRole),
    Public(VisibilityRole),
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

fn visibility_role_to_resource_role(vis: &VisibilityRole) -> ResourceRole {
    match vis {
        VisibilityRole::Viewer => ResourceRole::Viewer,
        VisibilityRole::Editor => ResourceRole::Editor,
    }
}

fn apply_agent_cap(principal: &Principal, role: Option<ResourceRole>) -> Option<ResourceRole> {
    match principal {
        Principal::ApiKey(_) => role.map(|r| r.min(ResourceRole::Editor)),
        Principal::User(_) => role,
    }
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
    if matches!(actor, Principal::ApiKey(_)) {
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

/// Validates that a task reference has exactly one target consistent with its kind,
/// and that the source task does not reference itself.
///
/// Spec → document target; Relates/Blocks/Parent → task target.
/// Multi-node Parent cycles (A→B→A) are not detected here; they require DB
/// ancestry traversal and are left as a follow-up.
pub fn validate_reference(
    source_task_id: TaskId,
    kind: ReferenceKind,
    target_task_id: Option<TaskId>,
    target_document_id: Option<DocumentId>,
) -> Result<(), DomainError> {
    match (target_task_id, target_document_id) {
        (Some(_), Some(_)) => {
            return Err(DomainError::InvalidInput {
                message: "a task reference must have exactly one target, not both".into(),
            });
        }
        (None, None) => {
            return Err(DomainError::InvalidInput {
                message: "a task reference must have exactly one target".into(),
            });
        }
        _ => {}
    }

    if target_task_id == Some(source_task_id) {
        return Err(DomainError::InvalidInput {
            message: "a task cannot reference itself".into(),
        });
    }

    match kind {
        ReferenceKind::Spec => {
            if target_document_id.is_none() {
                return Err(DomainError::InvalidInput {
                    message: "Spec reference requires a document target".into(),
                });
            }
        }
        ReferenceKind::Relates | ReferenceKind::Blocks | ReferenceKind::Parent => {
            if target_task_id.is_none() {
                return Err(DomainError::InvalidInput {
                    message: format!("{} reference requires a task target", kind.as_str()),
                });
            }
        }
    }

    Ok(())
}
