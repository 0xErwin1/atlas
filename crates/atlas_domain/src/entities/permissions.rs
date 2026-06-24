use crate::ids::{
    ApiKeyId, BoardId, DocumentId, FolderId, GroupId, ProjectId, UserId, WorkspaceId,
};
use crate::permissions::ResourceRole;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PermissionGrantId(pub Uuid);

impl PermissionGrantId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for PermissionGrantId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct PermissionGrant {
    pub id: PermissionGrantId,
    pub workspace_id: WorkspaceId,
    pub user_id: Option<UserId>,
    pub api_key_id: Option<ApiKeyId>,
    pub group_id: Option<GroupId>,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
    pub document_id: Option<DocumentId>,
    pub board_id: Option<BoardId>,
    pub role: ResourceRole,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPermissionGrant {
    pub workspace_id: WorkspaceId,
    pub user_id: Option<UserId>,
    pub api_key_id: Option<ApiKeyId>,
    pub group_id: Option<GroupId>,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
    pub document_id: Option<DocumentId>,
    pub board_id: Option<BoardId>,
    pub role: ResourceRole,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
}
