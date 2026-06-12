use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::boards_tasks::{
        Board, BoardColumn, NewBoard, NewTask, NewTaskReference, PositionBetween, Task, TaskPatch,
        TaskReference,
    },
    ids::{BoardId, ColumnId, TaskId},
    position,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, Statement, TransactionTrait,
};

use crate::persistence::entities::boards_tasks::{
    board, board_column, board_column_from, board_from, task, task_from, task_reference,
    task_reference_from,
};

pub use atlas_domain::ports::boards_tasks::{BoardRepo, TaskReferenceRepo, TaskRepo};

pub struct PgBoardRepo {
    pub conn: DatabaseConnection,
}

impl PgBoardRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl BoardRepo for PgBoardRepo {
    async fn create_board(&self, ctx: &WorkspaceCtx, new: NewBoard) -> Result<Board, DomainError> {
        let created_by_user_id = user_id_from_actor(&ctx.actor);
        let model = board::ActiveModel {
            id: Set(BoardId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            project_id: Set(new.project_id.0),
            name: Set(new.name),
            created_by_user_id: Set(created_by_user_id),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map(board_from)
            .map_err(db_err)
    }

    async fn find_board(
        &self,
        ctx: &WorkspaceCtx,
        id: BoardId,
    ) -> Result<Option<Board>, DomainError> {
        board::Entity::find_by_id(id.0)
            .filter(board::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(board_from))
            .map_err(db_err)
    }

    async fn list_boards(&self, ctx: &WorkspaceCtx) -> Result<Vec<Board>, DomainError> {
        board::Entity::find()
            .filter(board::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board::Column::DeletedAt.is_null())
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(board_from).collect())
            .map_err(db_err)
    }

    async fn add_column(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
        name: String,
        position: PositionBetween,
    ) -> Result<BoardColumn, DomainError> {
        let position_key = position::between(position.before.as_deref(), position.after.as_deref());
        let created_by_user_id = user_id_from_actor(&ctx.actor);
        let model = board_column::ActiveModel {
            id: Set(ColumnId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            board_id: Set(board_id.0),
            name: Set(name),
            position_key: Set(position_key),
            created_by_user_id: Set(created_by_user_id),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map(board_column_from)
            .map_err(db_err)
    }

    async fn list_columns(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
    ) -> Result<Vec<BoardColumn>, DomainError> {
        board_column::Entity::find()
            .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board_column::Column::BoardId.eq(board_id.0))
            .filter(board_column::Column::DeletedAt.is_null())
            .order_by_asc(board_column::Column::PositionKey)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(board_column_from).collect())
            .map_err(db_err)
    }

    async fn move_column(
        &self,
        ctx: &WorkspaceCtx,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<(), DomainError> {
        let row = board_column::Entity::find_by_id(column_id.0)
            .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board_column::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board_column",
                id: column_id.0,
            })?;

        let new_key = position::between(position.before.as_deref(), position.after.as_deref());
        let mut active = row.into_active_model();
        active.position_key = Set(new_key);
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn soft_delete_board(&self, ctx: &WorkspaceCtx, id: BoardId) -> Result<(), DomainError> {
        let row = board::Entity::find_by_id(id.0)
            .filter(board::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn soft_delete_column(
        &self,
        ctx: &WorkspaceCtx,
        id: ColumnId,
    ) -> Result<(), DomainError> {
        let row = board_column::Entity::find_by_id(id.0)
            .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board_column::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board_column",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

pub struct PgTaskRepo {
    pub conn: DatabaseConnection,
}

impl PgTaskRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl TaskRepo for PgTaskRepo {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewTask) -> Result<Task, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = txn
            .query_one_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Postgres,
                format!(
                    "UPDATE projects \
                     SET next_task_number = next_task_number + 1 \
                     WHERE id = '{}' \
                     RETURNING next_task_number, task_prefix",
                    new.project_id.0
                ),
            ))
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "project",
                id: new.project_id.0,
            })?;

        let task_number: i32 = row.try_get("", "next_task_number").map_err(db_err)?;
        let task_prefix: String = row.try_get("", "task_prefix").map_err(db_err)?;
        let readable_id = format!("{task_prefix}-{task_number}");

        let position_key = position::between(
            new.position.before.as_deref(),
            new.position.after.as_deref(),
        );
        let created_by_user_id = user_id_from_actor(&ctx.actor);
        let now = Utc::now();

        let model = task::ActiveModel {
            id: Set(TaskId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            project_id: Set(new.project_id.0),
            board_id: Set(new.board_id.0),
            column_id: Set(new.column_id.0),
            readable_id: Set(readable_id),
            title: Set(new.title),
            description: Set(new.description),
            properties: Set(None),
            position_key: Set(position_key),
            created_by_user_id: Set(created_by_user_id),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };
        let inserted = model.insert(&txn).await.map_err(db_err)?;

        txn.commit().await.map_err(db_err)?;

        Ok(task_from(inserted))
    }

    async fn find(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<Option<Task>, DomainError> {
        task::Entity::find_by_id(id.0)
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(task_from))
            .map_err(db_err)
    }

    async fn list_by_column(
        &self,
        ctx: &WorkspaceCtx,
        column_id: ColumnId,
    ) -> Result<Vec<Task>, DomainError> {
        task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::ColumnId.eq(column_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .order_by_asc(task::Column::PositionKey)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(task_from).collect())
            .map_err(db_err)
    }

    async fn patch(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskId,
        patch: TaskPatch,
    ) -> Result<Task, DomainError> {
        let row = task::Entity::find_by_id(id.0)
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        if let Some(title) = patch.title {
            active.title = Set(title);
        }
        if let Some(description) = patch.description {
            active.description = Set(description);
        }
        if let Some(props) = patch.properties {
            active.properties = Set(Some(props));
        }
        active.updated_at = Set(Utc::now());
        active
            .update(&self.conn)
            .await
            .map(task_from)
            .map_err(db_err)
    }

    async fn move_to(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskId,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<Task, DomainError> {
        let row = task::Entity::find_by_id(id.0)
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task",
                id: id.0,
            })?;

        let new_key = position::between(position.before.as_deref(), position.after.as_deref());
        let mut active = row.into_active_model();
        active.column_id = Set(column_id.0);
        active.position_key = Set(new_key);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.conn)
            .await
            .map(task_from)
            .map_err(db_err)
    }

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<(), DomainError> {
        let row = task::Entity::find_by_id(id.0)
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

pub struct PgTaskReferenceRepo {
    pub conn: DatabaseConnection,
}

impl PgTaskReferenceRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl TaskReferenceRepo for PgTaskReferenceRepo {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskReference,
    ) -> Result<TaskReference, DomainError> {
        use atlas_domain::ids::TaskId as TRefId;
        use atlas_domain::permissions::validate_reference;

        validate_reference(new.kind.clone(), new.target_task_id, new.target_document_id)?;

        let created_by_user_id = user_id_from_actor(&ctx.actor);
        let model = task_reference::ActiveModel {
            id: Set(TRefId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            source_task_id: Set(new.source_task_id.0),
            kind: Set(new.kind.as_str().to_string()),
            target_task_id: Set(new.target_task_id.map(|id| id.0)),
            target_document_id: Set(new.target_document_id.map(|id| id.0)),
            created_by_user_id: Set(created_by_user_id),
            created_at: Set(Utc::now()),
        };
        model
            .insert(&self.conn)
            .await
            .map_err(db_err)
            .and_then(|m| task_reference_from(m).map_err(internal_err))
    }

    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskReference>, DomainError> {
        let rows = task_reference::Entity::find()
            .filter(task_reference::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_reference::Column::SourceTaskId.eq(task_id.0))
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        rows.into_iter()
            .map(|m| task_reference_from(m).map_err(internal_err))
            .collect()
    }

    async fn delete(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<(), DomainError> {
        task_reference::Entity::delete_by_id(id.0)
            .filter(task_reference::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .exec(&self.conn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

fn user_id_from_actor(actor: &Actor) -> Option<uuid::Uuid> {
    match actor {
        Actor::User(uid) => Some(uid.0),
        Actor::ApiKey(_) => None,
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}

fn internal_err(msg: String) -> DomainError {
    DomainError::Internal { message: msg }
}
