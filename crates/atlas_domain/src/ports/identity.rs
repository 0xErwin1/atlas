/// User/session/activation/api-key ports relocated to
/// `atlas_custos::ports::identity` (S2d). Re-exported here to keep every
/// existing `crate::ports::identity::*` import path compiling.
pub use atlas_custos::ports::identity::{ActivationTokenRepo, ApiKeyRepo, SessionRepo, UserRepo};

/// Workspace/membership ports relocated to `atlas_acta::ports::identity`
/// (S2d). Re-exported here to keep every existing
/// `crate::ports::identity::*` import path compiling.
///
/// `UiStateRepo` is the one exception (D7): it moved to
/// `atlas_server::platform` and is no longer reachable from this facade.
pub use atlas_acta::ports::identity::{MembershipRepo, WorkspaceRepo};
