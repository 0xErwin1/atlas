use atlas_domain::actor::Actor;
use atlas_domain::entities::boards_tasks::{
    ActivityKind, AssigneeRef, Board, BoardColumn, ReferenceKind, Task, TaskActivity, TaskAssignee,
    TaskChecklistItem, TaskReference,
};
use atlas_domain::ids::{
    ApiKeyId, BoardId, ChecklistItemId, ColumnId, FolderId, ProjectId, TaskActivityId, TaskId,
    TaskReferenceId, UserId, WorkspaceId,
};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod board {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "boards")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub project_id: Uuid,
        pub folder_id: Option<Uuid>,
        pub name: String,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod board_column {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "board_columns")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub board_id: Uuid,
        pub name: String,
        pub position_key: String,
        pub color: Option<String>,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod task {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "tasks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub project_id: Uuid,
        pub board_id: Uuid,
        pub column_id: Uuid,
        pub parent_task_id: Option<Uuid>,
        pub readable_id: String,
        pub title: String,
        pub description: String,
        pub priority: Option<String>,
        pub due_date: Option<DateTime<Utc>>,
        pub estimate: Option<i32>,
        pub labels: Vec<String>,
        pub properties: Option<Json>,
        pub position_key: String,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod task_reference {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "task_references")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub source_task_id: Uuid,
        pub kind: String,
        pub target_task_id: Option<Uuid>,
        pub target_document_id: Option<Uuid>,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod task_assignee {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "task_assignees")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false, column_name = "task_id")]
        pub task_id: Uuid,
        pub workspace_id: Uuid,
        pub assignee_user_id: Option<Uuid>,
        pub assignee_api_key_id: Option<Uuid>,
        pub assigned_by_user_id: Option<Uuid>,
        pub assigned_by_api_key_id: Option<Uuid>,
        pub assigned_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod task_checklist_item {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "task_checklist_items")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub task_id: Uuid,
        pub workspace_id: Uuid,
        pub title: String,
        pub checked: bool,
        pub position_key: String,
        pub promoted_task_id: Option<Uuid>,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod task_activity {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "task_activity")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub task_id: Uuid,
        pub workspace_id: Uuid,
        pub kind: String,
        pub payload: Json,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Constructs an `Actor` from the XOR user/api-key pair carried by every actor-attributed row.
///
/// The DB XOR CHECK constraint guarantees exactly one of the two columns is non-null, so the
/// both-null arm is unreachable in practice. The fallback returns a fabricated `Actor::User`
/// rather than panicking; threading `Result` through every infallible read mapper is out of scope.
pub fn actor_from_columns(user_id: Option<Uuid>, api_key_id: Option<Uuid>) -> Actor {
    match (user_id, api_key_id) {
        (Some(uid), None) => Actor::User(UserId(uid)),
        (None, Some(kid)) => Actor::ApiKey(ApiKeyId(kid)),
        _ => Actor::User(UserId::new()),
    }
}

pub fn board_from(m: board::Model) -> Board {
    Board {
        id: BoardId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        project_id: ProjectId(m.project_id),
        folder_id: m.folder_id.map(FolderId),
        name: m.name,
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

pub fn board_column_from(m: board_column::Model) -> BoardColumn {
    BoardColumn {
        id: ColumnId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        board_id: BoardId(m.board_id),
        name: m.name,
        position_key: m.position_key,
        color: m.color,
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

pub fn task_from(m: task::Model) -> Task {
    use std::str::FromStr;
    Task {
        id: TaskId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        project_id: ProjectId(m.project_id),
        board_id: BoardId(m.board_id),
        column_id: ColumnId(m.column_id),
        parent_task_id: m.parent_task_id.map(TaskId),
        readable_id: m.readable_id,
        title: m.title,
        description: m.description,
        priority: m
            .priority
            .as_deref()
            .and_then(|s| atlas_domain::entities::boards_tasks::Priority::from_str(s).ok()),
        due_date: m.due_date,
        estimate: m.estimate,
        labels: m.labels,
        properties: m.properties,
        position_key: m.position_key,
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

pub fn task_reference_from(m: task_reference::Model) -> Result<TaskReference, String> {
    let kind = match m.kind.as_str() {
        "relates" => ReferenceKind::Relates,
        "blocks" => ReferenceKind::Blocks,
        "parent" => ReferenceKind::Parent,
        "spec" => ReferenceKind::Spec,
        "docs" => ReferenceKind::Docs,
        other => return Err(format!("unknown ReferenceKind: {other}")),
    };
    Ok(TaskReference {
        id: TaskReferenceId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        source_task_id: TaskId(m.source_task_id),
        kind,
        target_task_id: m.target_task_id.map(TaskId),
        target_document_id: m.target_document_id.map(atlas_domain::ids::DocumentId),
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        created_at: m.created_at,
    })
}

pub fn task_assignee_from(m: task_assignee::Model) -> Result<TaskAssignee, String> {
    let assignee = match (m.assignee_user_id, m.assignee_api_key_id) {
        (Some(uid), None) => AssigneeRef::User(UserId(uid)),
        (None, Some(kid)) => AssigneeRef::ApiKey(ApiKeyId(kid)),
        _ => return Err("task_assignee: invalid assignee XOR state".into()),
    };
    let assigned_by = actor_from_columns(m.assigned_by_user_id, m.assigned_by_api_key_id);

    Ok(TaskAssignee {
        task_id: TaskId(m.task_id),
        workspace_id: WorkspaceId(m.workspace_id),
        assignee,
        assigned_by,
        assigned_at: m.assigned_at,
    })
}

pub fn task_checklist_item_from(m: task_checklist_item::Model) -> TaskChecklistItem {
    TaskChecklistItem {
        id: ChecklistItemId(m.id),
        task_id: TaskId(m.task_id),
        workspace_id: WorkspaceId(m.workspace_id),
        title: m.title,
        checked: m.checked,
        position_key: m.position_key,
        promoted_task_id: m.promoted_task_id.map(TaskId),
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

pub fn task_activity_from(m: task_activity::Model) -> Result<TaskActivity, String> {
    use std::str::FromStr;

    let kind = activity_kind_from_str(&m.kind)?;
    let payload: atlas_domain::entities::boards_tasks::ActivityPayload =
        serde_json::from_value(m.payload)
            .map_err(|e| format!("activity payload deserialize: {e}"))?;

    let actor = actor_from_columns(m.created_by_user_id, m.created_by_api_key_id);

    // Suppress unused import warning from the std::str::FromStr brought into scope.
    let _ = String::from_str;

    Ok(TaskActivity {
        id: TaskActivityId(m.id),
        task_id: TaskId(m.task_id),
        workspace_id: WorkspaceId(m.workspace_id),
        kind,
        actor,
        payload,
        created_at: m.created_at,
    })
}

pub fn activity_kind_from_str(s: &str) -> Result<ActivityKind, String> {
    match s {
        "created" => Ok(ActivityKind::Created),
        "moved" => Ok(ActivityKind::Moved),
        "assigned" => Ok(ActivityKind::Assigned),
        "unassigned" => Ok(ActivityKind::Unassigned),
        "field_changed" => Ok(ActivityKind::FieldChanged),
        "reference_added" => Ok(ActivityKind::ReferenceAdded),
        "reference_removed" => Ok(ActivityKind::ReferenceRemoved),
        "checklist_added" => Ok(ActivityKind::ChecklistAdded),
        "checklist_updated" => Ok(ActivityKind::ChecklistUpdated),
        "checklist_removed" => Ok(ActivityKind::ChecklistRemoved),
        "checklist_promoted" => Ok(ActivityKind::ChecklistPromoted),
        "document_mentioned" => Ok(ActivityKind::DocumentMentioned),
        "deleted" => Ok(ActivityKind::Deleted),
        other => Err(format!("unknown ActivityKind: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_domain::entities::boards_tasks::Priority;

    #[test]
    fn actor_from_columns_user() {
        let uid = Uuid::now_v7();
        let actor = actor_from_columns(Some(uid), None);
        assert!(matches!(actor, Actor::User(id) if id.0 == uid));
    }

    #[test]
    fn actor_from_columns_api_key() {
        let kid = Uuid::now_v7();
        let actor = actor_from_columns(None, Some(kid));
        assert!(matches!(actor, Actor::ApiKey(id) if id.0 == kid));
    }

    #[test]
    fn board_column_from_maps_color() {
        let now = chrono::Utc::now();
        let uid = Uuid::now_v7();
        let m = board_column::Model {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            board_id: Uuid::now_v7(),
            name: "Done".into(),
            position_key: "80".into(),
            color: Some("green".into()),
            created_by_user_id: Some(uid),
            created_by_api_key_id: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        let col = board_column_from(m);
        assert_eq!(col.color.as_deref(), Some("green"));
    }

    #[test]
    fn task_from_maps_priority() {
        let now = chrono::Utc::now();
        let m = task::Model {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Uuid::now_v7(),
            board_id: Uuid::now_v7(),
            column_id: Uuid::now_v7(),
            parent_task_id: None,
            readable_id: "P-1".into(),
            title: "T".into(),
            description: "".into(),
            priority: Some("high".into()),
            due_date: None,
            estimate: Some(3),
            labels: vec!["backend".into()],
            properties: None,
            position_key: "80".into(),
            created_by_user_id: Some(Uuid::now_v7()),
            created_by_api_key_id: None,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };
        let task = task_from(m);
        assert_eq!(task.priority, Some(Priority::High));
        assert_eq!(task.estimate, Some(3));
        assert_eq!(task.labels, vec!["backend".to_string()]);
    }

    #[test]
    fn activity_kind_round_trips() {
        let kinds = [
            "created",
            "moved",
            "assigned",
            "unassigned",
            "field_changed",
            "reference_added",
            "reference_removed",
            "checklist_added",
            "checklist_updated",
            "checklist_removed",
            "checklist_promoted",
            "document_mentioned",
            "deleted",
        ];
        for k in &kinds {
            let parsed = activity_kind_from_str(k).expect("must parse");
            assert_eq!(parsed.as_str(), *k);
        }
    }

    #[test]
    fn task_reference_from_uses_task_reference_id() {
        let ref_id = Uuid::now_v7();
        let m = task_reference::Model {
            id: ref_id,
            workspace_id: Uuid::now_v7(),
            source_task_id: Uuid::now_v7(),
            kind: "relates".into(),
            target_task_id: Some(Uuid::now_v7()),
            target_document_id: None,
            created_by_user_id: Some(Uuid::now_v7()),
            created_by_api_key_id: None,
            created_at: chrono::Utc::now(),
        };
        let r = task_reference_from(m).expect("must succeed");
        assert_eq!(r.id.0, ref_id);
    }
}
