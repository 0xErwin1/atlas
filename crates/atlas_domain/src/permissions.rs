use crate::entities::boards_tasks::ReferenceKind;
use crate::error::DomainError;
use crate::ids::{BoardId, DocumentId, FolderId, ProjectId, TaskId};
use serde::{Deserialize, Serialize};

pub mod resource_ref_codec;

/// The live authorization-time actor identity. Relocated to `atlas_core::principal`
/// (D4) so `atlas_core` owns principal identity; re-exported here to keep every
/// existing `crate::permissions::Principal` import path compiling.
pub use atlas_core::principal::Principal;

/// The api-key scope catalog. Relocated to `atlas_custos::capability` since
/// `ApiKey.scopes: Vec<Capability>` moved to `atlas_custos` and Custos cannot
/// depend back on `atlas_domain`; re-exported here to keep every existing
/// `crate::permissions::Capability` import path compiling.
pub use atlas_custos::capability::{Capability, CapabilityAction, CapabilityFamily};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceRole {
    Viewer,
    Editor,
    Admin,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// Validates that a task reference has exactly one target consistent with its kind,
/// and that the source task does not reference itself.
///
/// Spec/Docs → document target; Relates/Blocks/Parent → task target.
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
        ReferenceKind::Spec | ReferenceKind::Docs => {
            if target_document_id.is_none() {
                return Err(DomainError::InvalidInput {
                    message: format!("{} reference requires a document target", kind.as_str()),
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
