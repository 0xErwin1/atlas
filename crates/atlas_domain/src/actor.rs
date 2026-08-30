use crate::ids::WorkspaceId;

pub use atlas_core::Attribution as Actor;
pub use atlas_core::attribution::{ApiKeyAttributionId, UserAttributionId};

#[derive(Debug, Clone)]
pub struct WorkspaceCtx {
    pub workspace_id: WorkspaceId,
    pub actor: Actor,
}

impl WorkspaceCtx {
    pub fn new(workspace_id: WorkspaceId, actor: Actor) -> Self {
        Self {
            workspace_id,
            actor,
        }
    }
}
