/// The live authorization-time actor identity. Relocated to `atlas_core::principal`
/// (D4) so `atlas_core` owns principal identity; re-exported here to keep every
/// existing `crate::permissions::Principal` import path compiling.
pub use atlas_core::principal::Principal;

/// The api-key scope catalog. Relocated to `atlas_custos::capability` since
/// `ApiKey.scopes: Vec<Capability>` moved to `atlas_custos` and Custos cannot
/// depend back on `atlas_domain`; re-exported here to keep every existing
/// `crate::permissions::Capability` import path compiling.
pub use atlas_custos::capability::{Capability, CapabilityAction, CapabilityFamily};

pub use atlas_acta::permissions::resource_ref_codec;
/// `ResourceRef`, `Visibility`, `VisibilityRole`, `validate_reference` and the
/// `resource_ref_codec` module relocated to `atlas_acta::permissions` (S2d
/// D6); re-exported here to keep every existing `crate::permissions::*`
/// import path compiling.
pub use atlas_acta::permissions::{ResourceRef, Visibility, VisibilityRole, validate_reference};

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
