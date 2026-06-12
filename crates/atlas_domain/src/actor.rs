use crate::ids::{ApiKeyId, UserId, WorkspaceId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    User(UserId),
    ApiKey(ApiKeyId),
}

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
