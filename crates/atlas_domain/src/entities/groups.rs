use crate::ids::{GroupId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Group {
    pub id: GroupId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub created_by: UserId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewGroup {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub created_by: UserId,
}

#[derive(Debug, Clone)]
pub struct GroupMember {
    pub group_id: GroupId,
    pub user_id: UserId,
    pub created_at: DateTime<Utc>,
}
