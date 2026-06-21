use crate::actor::Actor;
use crate::ids::{
    BoardId, ChecklistItemId, ColumnId, DocumentId, ProjectId, TaskId, TaskReferenceId, UserId,
    WorkspaceId,
};
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
    pub color: Option<String>,
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

/// Patch type for column updates, mirroring `TaskPatch`'s `Option<Option<T>>`
/// convention: `None` = leave unchanged; `Some(None)` = clear; `Some(Some(v))` = set.
#[derive(Debug, Clone, Default)]
pub struct ColumnPatch {
    pub name: Option<String>,
    pub color: Option<Option<String>>,
}

/// Task priority levels, ordered from lowest to highest urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Medium,
    High,
    Urgent,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
            Priority::Urgent => "urgent",
        }
    }
}

impl std::str::FromStr for Priority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Priority::Low),
            "medium" => Ok(Priority::Medium),
            "high" => Ok(Priority::High),
            "urgent" => Ok(Priority::Urgent),
            other => Err(format!("unknown priority: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
    pub board_id: BoardId,
    pub column_id: ColumnId,
    /// Set when this task is a sub-task of another. Sub-tasks are full tasks
    /// (their own status, assignees, description, etc.) but are excluded from the
    /// kanban board listing; promoting one clears this back to `None`.
    pub parent_task_id: Option<TaskId>,
    pub readable_id: String,
    pub title: String,
    pub description: String,
    pub priority: Option<Priority>,
    pub due_date: Option<DateTime<Utc>>,
    pub estimate: Option<i32>,
    pub labels: Vec<String>,
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
    pub priority: Option<Priority>,
    pub due_date: Option<DateTime<Utc>>,
    pub estimate: Option<i32>,
    pub labels: Vec<String>,
    pub properties: Option<serde_json::Value>,
    pub position: PositionBetween,
}

#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<Option<Priority>>,
    pub due_date: Option<Option<DateTime<Utc>>>,
    pub estimate: Option<Option<i32>>,
    pub labels: Option<Vec<String>>,
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

/// Who is assigned: a user or an api key acting as an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "id")]
pub enum AssigneeRef {
    User(UserId),
    ApiKey(crate::ids::ApiKeyId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssignee {
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub assignee: AssigneeRef,
    pub assigned_by: Actor,
    pub assigned_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewTaskAssignee {
    pub task_id: TaskId,
    pub assignee: AssigneeRef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskChecklistItem {
    pub id: ChecklistItemId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub checked: bool,
    pub position_key: String,
    pub promoted_task_id: Option<TaskId>,
    pub created_by: Actor,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewTaskChecklistItem {
    pub task_id: TaskId,
    pub title: String,
    pub position: PositionBetween,
}

#[derive(Debug, Clone, Default)]
pub struct TaskChecklistItemPatch {
    pub title: Option<String>,
    pub checked: Option<bool>,
    pub position: Option<PositionBetween>,
}

/// Discriminated activity kind, one entry per state-changing operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Created,
    Moved,
    Assigned,
    Unassigned,
    FieldChanged,
    ReferenceAdded,
    ReferenceRemoved,
    ChecklistAdded,
    ChecklistUpdated,
    ChecklistRemoved,
    ChecklistPromoted,
    Deleted,
}

impl ActivityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActivityKind::Created => "created",
            ActivityKind::Moved => "moved",
            ActivityKind::Assigned => "assigned",
            ActivityKind::Unassigned => "unassigned",
            ActivityKind::FieldChanged => "field_changed",
            ActivityKind::ReferenceAdded => "reference_added",
            ActivityKind::ReferenceRemoved => "reference_removed",
            ActivityKind::ChecklistAdded => "checklist_added",
            ActivityKind::ChecklistUpdated => "checklist_updated",
            ActivityKind::ChecklistRemoved => "checklist_removed",
            ActivityKind::ChecklistPromoted => "checklist_promoted",
            ActivityKind::Deleted => "deleted",
        }
    }
}

/// Typed payload per activity verb.
///
/// Serialized to JSONB in task_activity.payload; the kind column provides the discriminant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityPayload {
    Created,
    Moved {
        from_column_id: ColumnId,
        to_column_id: ColumnId,
    },
    Assigned {
        assignee: AssigneeRef,
    },
    Unassigned {
        assignee: AssigneeRef,
    },
    FieldChanged {
        field: String,
        old_value: serde_json::Value,
        new_value: serde_json::Value,
    },
    ReferenceAdded {
        reference_id: TaskReferenceId,
        kind: ReferenceKind,
    },
    ReferenceRemoved {
        reference_id: TaskReferenceId,
        kind: ReferenceKind,
    },
    ChecklistAdded {
        item_id: ChecklistItemId,
        title: String,
    },
    ChecklistUpdated {
        item_id: ChecklistItemId,
    },
    ChecklistRemoved {
        item_id: ChecklistItemId,
    },
    ChecklistPromoted {
        item_id: ChecklistItemId,
        promoted_task_id: TaskId,
    },
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskActivity {
    pub id: crate::ids::TaskActivityId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub kind: ActivityKind,
    pub actor: Actor,
    pub payload: ActivityPayload,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewTaskActivity {
    pub task_id: TaskId,
    pub kind: ActivityKind,
    pub payload: ActivityPayload,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_round_trips_through_str() {
        for p in [
            Priority::Low,
            Priority::Medium,
            Priority::High,
            Priority::Urgent,
        ] {
            let s = p.as_str();
            let back: Priority = s.parse().expect("must parse");
            assert_eq!(p, back);
        }
    }

    #[test]
    fn priority_unknown_string_errors() {
        let result: Result<Priority, _> = "critical".parse();
        assert!(result.is_err());
    }

    #[test]
    fn activity_kind_as_str_is_snake_case() {
        assert_eq!(ActivityKind::FieldChanged.as_str(), "field_changed");
        assert_eq!(
            ActivityKind::ChecklistPromoted.as_str(),
            "checklist_promoted"
        );
    }

    #[test]
    fn activity_payload_serializes_to_json() {
        let p = ActivityPayload::FieldChanged {
            field: "title".into(),
            old_value: serde_json::json!("old"),
            new_value: serde_json::json!("new"),
        };
        let s = serde_json::to_string(&p).expect("serialize");
        assert!(s.contains("field_changed"));
    }

    #[test]
    fn task_patch_default_has_all_nones() {
        let patch = TaskPatch::default();
        assert!(patch.title.is_none());
        assert!(patch.priority.is_none());
        assert!(patch.due_date.is_none());
        assert!(patch.estimate.is_none());
        assert!(patch.labels.is_none());
    }
}
