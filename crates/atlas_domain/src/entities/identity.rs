/// User/session/activation/api-key entities relocated to
/// `atlas_custos::entities::identity` (S2d). Re-exported here to keep every
/// existing `crate::entities::identity::*` import path compiling.
pub use atlas_custos::entities::identity::{
    ActivationToken, ApiKey, ApiKeyType, NewActivationToken, NewApiKey, NewSession, NewUser,
    Session, User,
};

/// Workspace/membership entities relocated to `atlas_acta::entities::identity`
/// (S2d). Re-exported here to keep every existing
/// `crate::entities::identity::*` import path compiling.
///
/// `UserUiState` is the one exception (D7): it moved to
/// `atlas_server::platform` and is no longer reachable from this facade.
pub use atlas_acta::entities::identity::{
    MemberRole, NewWorkspace, Workspace, WorkspaceMembership,
};
