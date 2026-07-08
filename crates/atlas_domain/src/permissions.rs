use crate::entities::boards_tasks::ReferenceKind;
use crate::entities::identity::MemberRole;
use crate::error::DomainError;
use crate::ids::{ApiKeyId, BoardId, DocumentId, FolderId, GroupId, ProjectId, TaskId, UserId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceRole {
    Viewer,
    Editor,
    Admin,
}

/// The resource family a capability governs. Together with `CapabilityAction`
/// forms the closed `family:action` catalog that API key scopes are drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityFamily {
    Tasks,
    Docs,
    Boards,
    Folders,
    Projects,
    Webhooks,
    Config,
    Grants,
    SavedSearches,
    TaskViews,
}

/// The CRUD verb of a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityAction {
    Read,
    Create,
    Update,
    Delete,
}

/// A single `family:action` capability, e.g. `tasks:read`. This is the unit of
/// an API key's scope set. The catalog (`Capability::ALL`) is the cross product
/// of families and actions, except `grants`, which is read-only and so
/// contributes only `grants:read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability {
    pub family: CapabilityFamily,
    pub action: CapabilityAction,
}

impl Capability {
    /// The closed catalog of every valid capability, in `family:action` order
    /// with families ordered `tasks, docs, boards, folders, projects, webhooks,
    /// config` and actions ordered `read, create, update, delete`. This is the
    /// single source of truth other derived sets (defaults, wire enums) are
    /// built from.
    pub const ALL: [Capability; 37] = [
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Webhooks,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Webhooks,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Webhooks,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Webhooks,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Delete,
        },
        // `grants` is read-only: grant WRITES stay domain-blocked for agents by
        // `authorize_share` (AgentsNeverManageGrants), so the catalog exposes
        // only `grants:read` and no grant-write capability can ever be granted.
        Capability {
            family: CapabilityFamily::Grants,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::SavedSearches,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::SavedSearches,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::SavedSearches,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::SavedSearches,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::TaskViews,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::TaskViews,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::TaskViews,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::TaskViews,
            action: CapabilityAction::Delete,
        },
    ];

    /// The scope set a newly created API key receives when the caller selects
    /// no scopes: read access to every family, write access to none.
    pub const DEFAULT_READ_ONLY: [Capability; 5] = [
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Read,
        },
    ];

    /// The wire/storage representation, e.g. `"tasks:read"`. Stable and used
    /// both for the `TEXT[]` storage column and the wire DTO.
    pub fn as_str(&self) -> &'static str {
        match (self.family, self.action) {
            (CapabilityFamily::Tasks, CapabilityAction::Read) => "tasks:read",
            (CapabilityFamily::Tasks, CapabilityAction::Create) => "tasks:create",
            (CapabilityFamily::Tasks, CapabilityAction::Update) => "tasks:update",
            (CapabilityFamily::Tasks, CapabilityAction::Delete) => "tasks:delete",
            (CapabilityFamily::Docs, CapabilityAction::Read) => "docs:read",
            (CapabilityFamily::Docs, CapabilityAction::Create) => "docs:create",
            (CapabilityFamily::Docs, CapabilityAction::Update) => "docs:update",
            (CapabilityFamily::Docs, CapabilityAction::Delete) => "docs:delete",
            (CapabilityFamily::Boards, CapabilityAction::Read) => "boards:read",
            (CapabilityFamily::Boards, CapabilityAction::Create) => "boards:create",
            (CapabilityFamily::Boards, CapabilityAction::Update) => "boards:update",
            (CapabilityFamily::Boards, CapabilityAction::Delete) => "boards:delete",
            (CapabilityFamily::Folders, CapabilityAction::Read) => "folders:read",
            (CapabilityFamily::Folders, CapabilityAction::Create) => "folders:create",
            (CapabilityFamily::Folders, CapabilityAction::Update) => "folders:update",
            (CapabilityFamily::Folders, CapabilityAction::Delete) => "folders:delete",
            (CapabilityFamily::Projects, CapabilityAction::Read) => "projects:read",
            (CapabilityFamily::Projects, CapabilityAction::Create) => "projects:create",
            (CapabilityFamily::Projects, CapabilityAction::Update) => "projects:update",
            (CapabilityFamily::Projects, CapabilityAction::Delete) => "projects:delete",
            (CapabilityFamily::Webhooks, CapabilityAction::Read) => "webhooks:read",
            (CapabilityFamily::Webhooks, CapabilityAction::Create) => "webhooks:create",
            (CapabilityFamily::Webhooks, CapabilityAction::Update) => "webhooks:update",
            (CapabilityFamily::Webhooks, CapabilityAction::Delete) => "webhooks:delete",
            (CapabilityFamily::Config, CapabilityAction::Read) => "config:read",
            (CapabilityFamily::Config, CapabilityAction::Create) => "config:create",
            (CapabilityFamily::Config, CapabilityAction::Update) => "config:update",
            (CapabilityFamily::Config, CapabilityAction::Delete) => "config:delete",
            // The grant-write arms keep this match total; because `grants:read`
            // is the only grants entry in `Capability::ALL` and `FromStr`
            // iterates `ALL`, these write strings are never produced or parsed.
            (CapabilityFamily::Grants, CapabilityAction::Read) => "grants:read",
            (CapabilityFamily::Grants, CapabilityAction::Create) => "grants:create",
            (CapabilityFamily::Grants, CapabilityAction::Update) => "grants:update",
            (CapabilityFamily::Grants, CapabilityAction::Delete) => "grants:delete",
            (CapabilityFamily::SavedSearches, CapabilityAction::Read) => "saved_searches:read",
            (CapabilityFamily::SavedSearches, CapabilityAction::Create) => "saved_searches:create",
            (CapabilityFamily::SavedSearches, CapabilityAction::Update) => "saved_searches:update",
            (CapabilityFamily::SavedSearches, CapabilityAction::Delete) => "saved_searches:delete",
            (CapabilityFamily::TaskViews, CapabilityAction::Read) => "task_views:read",
            (CapabilityFamily::TaskViews, CapabilityAction::Create) => "task_views:create",
            (CapabilityFamily::TaskViews, CapabilityAction::Update) => "task_views:update",
            (CapabilityFamily::TaskViews, CapabilityAction::Delete) => "task_views:delete",
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Capability::ALL
            .into_iter()
            .find(|cap| cap.as_str() == s)
            .ok_or_else(|| format!("unknown capability: {s}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    User(UserId),
    ApiKey(ApiKeyId),
    Group(GroupId),
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

fn visibility_role_to_resource_role(vis: &VisibilityRole) -> ResourceRole {
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
