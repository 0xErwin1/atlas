use crate::ids::{PlatformStatusTemplateId, StatusTemplateId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusTemplate {
    pub id: StatusTemplateId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub color: Option<String>,
    pub position_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Atlas-wide default status, the platform-level counterpart of
/// [`StatusTemplate`]. A new workspace copies these into its own status
/// templates at creation time; editing one never retro-updates a workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformStatusTemplate {
    pub id: PlatformStatusTemplateId,
    pub name: String,
    pub color: Option<String>,
    pub position_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewStatusTemplate {
    pub name: String,
    pub color: Option<String>,
    pub position_key: String,
}

/// Patch type mirroring `ColumnPatch`:
/// `None` = leave unchanged; `Some(None)` = clear; `Some(Some(v))` = set.
#[derive(Debug, Clone, Default)]
pub struct StatusTemplatePatch {
    pub name: Option<String>,
    pub color: Option<Option<String>>,
}
