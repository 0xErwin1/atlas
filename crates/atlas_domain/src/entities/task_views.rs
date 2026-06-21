use crate::actor::Actor;
use crate::entities::boards_tasks::Priority;
use crate::ids::{ApiKeyId, BoardId, ColumnId, TaskViewId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Identifies the owner of a task view (user XOR api_key).
///
/// Structurally identical to `Owner` in `entities::saved_searches` but defined
/// independently to keep modules decoupled. Each resource type owns its own
/// `Owner`; the XOR pattern is consistent across both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Owner {
    User(UserId),
    ApiKey(ApiKeyId),
}

impl Owner {
    pub fn from_actor(actor: &Actor) -> Self {
        match actor {
            Actor::User(uid) => Owner::User(*uid),
            Actor::ApiKey(kid) => Owner::ApiKey(*kid),
        }
    }

    pub fn matches_actor(&self, actor: &Actor) -> bool {
        match (self, actor) {
            (Owner::User(oid), Actor::User(aid)) => oid == aid,
            (Owner::ApiKey(oid), Actor::ApiKey(aid)) => oid == aid,
            _ => false,
        }
    }
}

/// Assignee filter for a task view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssigneeFilter {
    /// The caller's own principal — resolved at query time, not at storage time.
    Me,
    User(UserId),
    ApiKey(ApiKeyId),
}

/// Creator actor-type filter (powers the "Agent activity" predefined view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorTypeFilter {
    User,
    ApiKey,
}

/// Sort order for a task view listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSort {
    UpdatedDesc,
    UpdatedAsc,
    CreatedDesc,
    CreatedAsc,
    PriorityDesc,
    TitleAsc,
}

impl TaskSort {
    pub fn as_param_str(self) -> &'static str {
        match self {
            TaskSort::UpdatedDesc => "updated_at_desc",
            TaskSort::UpdatedAsc => "updated_at_asc",
            TaskSort::CreatedDesc => "created_at_desc",
            TaskSort::CreatedAsc => "created_at_asc",
            TaskSort::PriorityDesc => "priority_desc",
            TaskSort::TitleAsc => "title_asc",
        }
    }

    pub fn from_param_str(s: &str) -> Option<Self> {
        match s {
            "updated_at_desc" => Some(TaskSort::UpdatedDesc),
            "updated_at_asc" => Some(TaskSort::UpdatedAsc),
            "created_at_desc" => Some(TaskSort::CreatedDesc),
            "created_at_asc" => Some(TaskSort::CreatedAsc),
            "priority_desc" => Some(TaskSort::PriorityDesc),
            "title_asc" => Some(TaskSort::TitleAsc),
            _ => None,
        }
    }
}

/// The filter set stored in the `filters` JSONB column of a `task_views` row.
///
/// Every field is optional/defaulted so an empty `{}` is a valid "all workspace
/// tasks" view. This struct is the single source of truth shared by Slice 1
/// (storage) and Slice 2 (query execution).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskViewFilters {
    /// Restrict to tasks assigned to this principal. None = no assignee filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<AssigneeFilter>,

    /// Restrict by creator actor type. None = any creator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_type: Option<ActorTypeFilter>,

    /// Restrict to these board columns (by id). Empty = all columns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_ids: Vec<ColumnId>,

    /// Restrict to these priority levels. Empty = all priorities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub priorities: Vec<Priority>,

    /// Restrict to tasks carrying ALL of these labels. Empty = no label filter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,

    /// Scope to a single board. None = workspace-wide (cross-board).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_id: Option<BoardId>,

    /// Sort order. None defaults to UpdatedDesc at query time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<TaskSort>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskView {
    pub id: TaskViewId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub filters: TaskViewFilters,
    pub owner: Owner,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewTaskView {
    pub name: String,
    pub filters: TaskViewFilters,
}
