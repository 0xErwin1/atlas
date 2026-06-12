use atlas_domain::entities::boards_tasks::{
    Board, BoardColumn, ReferenceKind, Task, TaskReference,
};
use atlas_domain::ids::{BoardId, ColumnId, ProjectId, TaskId, UserId, WorkspaceId};
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
        pub name: String,
        pub created_by_user_id: Option<Uuid>,
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
        pub created_by_user_id: Option<Uuid>,
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
        pub readable_id: String,
        pub title: String,
        pub description: String,
        pub properties: Option<Json>,
        pub position_key: String,
        pub created_by_user_id: Option<Uuid>,
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
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn board_from(m: board::Model) -> Board {
    Board {
        id: BoardId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        project_id: ProjectId(m.project_id),
        name: m.name,
        created_by_user_id: m.created_by_user_id.map(UserId),
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
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

pub fn task_from(m: task::Model) -> Task {
    Task {
        id: TaskId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        project_id: ProjectId(m.project_id),
        board_id: BoardId(m.board_id),
        column_id: ColumnId(m.column_id),
        readable_id: m.readable_id,
        title: m.title,
        description: m.description,
        properties: m.properties,
        position_key: m.position_key,
        created_by_user_id: m.created_by_user_id.map(UserId),
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
        other => return Err(format!("unknown ReferenceKind: {other}")),
    };
    Ok(TaskReference {
        id: TaskId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        source_task_id: TaskId(m.source_task_id),
        kind,
        target_task_id: m.target_task_id.map(TaskId),
        target_document_id: m.target_document_id.map(atlas_domain::ids::DocumentId),
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_at: m.created_at,
    })
}
