use crate::actor::Actor;
use crate::ids::{BoardId, ColumnId, DocumentId, ProjectId, TaskId, TaskReferenceId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub id: BoardId,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub name: String,
    pub created_by: Actor,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewBoard {
    pub project_id: ProjectId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardColumn {
    pub id: ColumnId,
    pub workspace_id: WorkspaceId,
    pub board_id: BoardId,
    pub name: String,
    pub position_key: String,
    pub created_by: Actor,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewBoardColumn {
    pub board_id: BoardId,
    pub name: String,
    pub position_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub board_id: BoardId,
    pub column_id: ColumnId,
    pub readable_id: String,
    pub title: String,
    pub description: String,
    pub properties: Option<serde_json::Value>,
    pub position_key: String,
    pub created_by: Actor,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewTask {
    pub project_id: ProjectId,
    pub board_id: BoardId,
    pub column_id: ColumnId,
    pub title: String,
    pub description: String,
    pub position: PositionBetween,
}

#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub properties: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct PositionBetween {
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Relates,
    Blocks,
    Parent,
    Spec,
}

impl ReferenceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReferenceKind::Relates => "relates",
            ReferenceKind::Blocks => "blocks",
            ReferenceKind::Parent => "parent",
            ReferenceKind::Spec => "spec",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskReference {
    pub id: TaskReferenceId,
    pub workspace_id: WorkspaceId,
    pub source_task_id: TaskId,
    pub kind: ReferenceKind,
    pub target_task_id: Option<TaskId>,
    pub target_document_id: Option<DocumentId>,
    pub created_by: Actor,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewTaskReference {
    pub source_task_id: TaskId,
    pub kind: ReferenceKind,
    pub target_task_id: Option<TaskId>,
    pub target_document_id: Option<DocumentId>,
}
