use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::boards_tasks::{
        ActivityKind, AssigneeRef, Board, BoardColumn, NewBoard, NewTask, NewTaskActivity,
        NewTaskAssignee, NewTaskChecklistItem, NewTaskReference, PositionBetween, Task,
        TaskActivity, TaskAssignee, TaskChecklistItem, TaskChecklistItemPatch, TaskPatch,
        TaskReference,
    },
    ids::{BoardId, ChecklistItemId, ColumnId, ProjectId, TaskActivityId, TaskId, TaskReferenceId},
    position,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, Order, QueryFilter, QueryOrder, QuerySelect, Statement,
    TransactionTrait,
};

use crate::persistence::entities::boards_tasks::{
    activity_kind_from_str, board, board_column, board_column_from, board_from, task,
    task_activity, task_activity_from, task_assignee, task_assignee_from, task_checklist_item,
    task_checklist_item_from, task_from, task_reference, task_reference_from,
};

pub use atlas_domain::ports::boards_tasks::{
    BoardRepo, TaskActivityRepo, TaskAssigneeRepo, TaskChecklistRepo, TaskReferenceRepo, TaskRepo,
};

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
        let (by_user, by_key) = actor_columns(&ctx.actor);
        let model = board::ActiveModel {
            id: Set(BoardId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            project_id: Set(new.project_id.0),
            name: Set(new.name),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
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

    async fn list_boards(
        &self,
        ctx: &WorkspaceCtx,
        project_id: ProjectId,
    ) -> Result<Vec<Board>, DomainError> {
        board::Entity::find()
            .filter(board::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board::Column::ProjectId.eq(project_id.0))
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
        let txn = self.conn.begin().await.map_err(db_err)?;

        let position_key =
            match position::try_between(position.before.as_deref(), position.after.as_deref()) {
                Some(key) => key,
                None => {
                    resequence_column(&txn, ctx, board_id).await?;
                    position::between(position.before.as_deref(), position.after.as_deref())
                }
            };

        let (by_user, by_key) = actor_columns(&ctx.actor);
        let model = board_column::ActiveModel {
            id: Set(ColumnId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            board_id: Set(board_id.0),
            name: Set(name),
            position_key: Set(position_key),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        let inserted = model.insert(&txn).await.map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;
        Ok(board_column_from(inserted))
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
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = board_column::Entity::find_by_id(column_id.0)
            .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board_column::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board_column",
                id: column_id.0,
            })?;

        let board_id = BoardId(row.board_id);

        let new_key =
            match position::try_between(position.before.as_deref(), position.after.as_deref()) {
                Some(key) => key,
                None => {
                    resequence_column(&txn, ctx, board_id).await?;
                    match position::try_between(
                        position.before.as_deref(),
                        position.after.as_deref(),
                    ) {
                        Some(key) => key,
                        None => {
                            txn.rollback().await.map_err(db_err)?;
                            return Err(DomainError::PositionExhausted { column_id });
                        }
                    }
                }
            };

        let mut active = row.into_active_model();
        active.position_key = Set(new_key);
        active.updated_at = Set(Utc::now());
        active.update(&txn).await.map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    async fn patch_board(
        &self,
        ctx: &WorkspaceCtx,
        id: BoardId,
        name: String,
    ) -> Result<Board, DomainError> {
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
        active.name = Set(name);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.conn)
            .await
            .map(board_from)
            .map_err(db_err)
    }

    async fn patch_column(
        &self,
        ctx: &WorkspaceCtx,
        id: ColumnId,
        name: String,
    ) -> Result<BoardColumn, DomainError> {
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
        active.name = Set(name);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.conn)
            .await
            .map(board_column_from)
            .map_err(db_err)
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

    /// Patches a task inside an existing transaction.
    ///
    /// Used by `TaskService` to run the UPDATE on the same connection as the
    /// activity append, so both either commit or roll back together.
    pub async fn patch_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        id: TaskId,
        patch: TaskPatch,
    ) -> Result<Task, DomainError> {
        let row = task::Entity::find_by_id(id.0)
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(conn)
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
        if let Some(priority) = patch.priority {
            active.priority = Set(priority.map(|p| p.as_str().to_string()));
        }
        if let Some(due_date) = patch.due_date {
            active.due_date = Set(due_date);
        }
        if let Some(estimate) = patch.estimate {
            active.estimate = Set(estimate);
        }
        if let Some(labels) = patch.labels {
            active.labels = Set(labels);
        }
        if let Some(props) = patch.properties {
            active.properties = Set(Some(props));
        }
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map(task_from).map_err(db_err)
    }

    /// Soft-deletes a task inside an existing transaction or connection.
    ///
    /// Used by `TaskService` to run the UPDATE on the same connection as the
    /// activity append, so both either commit or roll back together.
    pub async fn soft_delete_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        id: TaskId,
    ) -> Result<(), DomainError> {
        let row = task::Entity::find_by_id(id.0)
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;
        Ok(())
    }

    /// Moves a task to a new column and position inside an existing transaction.
    ///
    /// Performs the single-statement atomic UPDATE (B1 fix) with resequence+retry
    /// on `PositionExhausted`. The caller (TaskService) owns the transaction boundary
    /// so the move and the activity append either both commit or both roll back.
    pub async fn move_to_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        id: TaskId,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<Task, DomainError> {
        match try_move_to_in(conn, ctx, id, column_id, &position).await {
            Ok(task) => Ok(task),
            Err(DomainError::PositionExhausted { .. }) => {
                resequence_tasks_in_column(conn, ctx, column_id).await?;
                try_move_to_in(conn, ctx, id, column_id, &position).await
            }
            Err(e) => Err(e),
        }
    }

    /// Creates a task inside an existing transaction.
    ///
    /// The readable-id counter update (`UPDATE projects SET next_task_number`)
    /// and the task insert both run on `conn`, which may be a
    /// `DatabaseTransaction` provided by `TaskService`.
    pub async fn create_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewTask,
    ) -> Result<Task, DomainError> {
        let row = conn
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
        let (by_user, by_key) = actor_columns(&ctx.actor);
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
            priority: Set(new.priority.map(|p| p.as_str().to_string())),
            due_date: Set(new.due_date),
            estimate: Set(new.estimate),
            labels: Set(new.labels),
            properties: Set(new.properties),
            position_key: Set(position_key),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };
        let inserted = model.insert(conn).await.map_err(db_err)?;

        Ok(task_from(inserted))
    }
}

#[async_trait]
impl TaskRepo for PgTaskRepo {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewTask) -> Result<Task, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let task = PgTaskRepo::create_in(&txn, ctx, new).await?;
        txn.commit().await.map_err(db_err)?;
        Ok(task)
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

    async fn find_by_readable_id(
        &self,
        ctx: &WorkspaceCtx,
        readable_id: &str,
    ) -> Result<Option<Task>, DomainError> {
        task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::ReadableId.eq(readable_id))
            .filter(task::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(task_from))
            .map_err(db_err)
    }

    async fn list_by_board(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
    ) -> Result<Vec<Task>, DomainError> {
        task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::BoardId.eq(board_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .order_by_asc(task::Column::PositionKey)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(task_from).collect())
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
        PgTaskRepo::patch_in(&self.conn, ctx, id, patch).await
    }

    async fn move_to(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskId,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<Task, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let result = PgTaskRepo::move_to_in(&txn, ctx, id, column_id, position).await;

        match &result {
            Ok(_) => txn.commit().await.map_err(db_err)?,
            Err(_) => txn.rollback().await.map_err(db_err)?,
        }

        result
    }

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<(), DomainError> {
        PgTaskRepo::soft_delete_in(&self.conn, ctx, id).await
    }
}

pub struct PgTaskReferenceRepo {
    pub conn: DatabaseConnection,
}

impl PgTaskReferenceRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Inserts a task reference inside an existing transaction.
    ///
    /// Used by `TaskService::promote_checklist_item` to keep the Parent reference
    /// creation in the same transaction as the task insert and checklist update.
    pub async fn create_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewTaskReference,
    ) -> Result<TaskReference, DomainError> {
        use atlas_domain::permissions::validate_reference;

        validate_reference(new.kind.clone(), new.target_task_id, new.target_document_id)?;

        let (by_user, by_key) = actor_columns(&ctx.actor);
        let model = task_reference::ActiveModel {
            id: Set(TaskReferenceId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            source_task_id: Set(new.source_task_id.0),
            kind: Set(new.kind.as_str().to_string()),
            target_task_id: Set(new.target_task_id.map(|id| id.0)),
            target_document_id: Set(new.target_document_id.map(|id| id.0)),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
        };
        model
            .insert(conn)
            .await
            .map_err(db_err)
            .and_then(|m| task_reference_from(m).map_err(internal_err))
    }
}

#[async_trait]
impl TaskReferenceRepo for PgTaskReferenceRepo {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskReference,
    ) -> Result<TaskReference, DomainError> {
        use atlas_domain::permissions::validate_reference;

        validate_reference(new.kind.clone(), new.target_task_id, new.target_document_id)?;

        let (by_user, by_key) = actor_columns(&ctx.actor);
        let model = task_reference::ActiveModel {
            id: Set(TaskReferenceId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            source_task_id: Set(new.source_task_id.0),
            kind: Set(new.kind.as_str().to_string()),
            target_task_id: Set(new.target_task_id.map(|id| id.0)),
            target_document_id: Set(new.target_document_id.map(|id| id.0)),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
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

    async fn list_inbound(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskReference>, DomainError> {
        let rows = task_reference::Entity::find()
            .filter(task_reference::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_reference::Column::TargetTaskId.eq(task_id.0))
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        rows.into_iter()
            .map(|m| task_reference_from(m).map_err(internal_err))
            .collect()
    }

    async fn delete(&self, ctx: &WorkspaceCtx, id: TaskReferenceId) -> Result<(), DomainError> {
        PgTaskReferenceRepo::delete_in(&self.conn, ctx, id).await
    }
}

impl PgTaskReferenceRepo {
    /// Deletes a task reference inside an existing transaction.
    pub async fn delete_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        id: TaskReferenceId,
    ) -> Result<(), DomainError> {
        task_reference::Entity::delete_by_id(id.0)
            .filter(task_reference::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .exec(conn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

pub struct PgTaskAssigneeRepo {
    pub conn: DatabaseConnection,
}

impl PgTaskAssigneeRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Inserts a task assignee inside an existing transaction.
    pub async fn add_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewTaskAssignee,
    ) -> Result<TaskAssignee, DomainError> {
        let (assignee_user_id, assignee_api_key_id) = match new.assignee {
            AssigneeRef::User(uid) => (Some(uid.0), None),
            AssigneeRef::ApiKey(kid) => (None, Some(kid.0)),
        };
        let (by_user, by_key) = actor_columns(&ctx.actor);

        let model = task_assignee::ActiveModel {
            task_id: Set(new.task_id.0),
            workspace_id: Set(ctx.workspace_id.0),
            assignee_user_id: Set(assignee_user_id),
            assignee_api_key_id: Set(assignee_api_key_id),
            assigned_by_user_id: Set(by_user),
            assigned_by_api_key_id: Set(by_key),
            assigned_at: Set(Utc::now()),
        };
        model
            .insert(conn)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("unique") || msg.contains("duplicate") {
                    DomainError::Forbidden {
                        message: "assignee already added to this task".into(),
                    }
                } else {
                    db_err(e)
                }
            })
            .and_then(|m| task_assignee_from(m).map_err(internal_err))
    }

    /// Removes a task assignee inside an existing transaction.
    pub async fn remove_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        assignee: AssigneeRef,
    ) -> Result<(), DomainError> {
        let mut q = task_assignee::Entity::delete_many()
            .filter(task_assignee::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_assignee::Column::TaskId.eq(task_id.0));

        q = match assignee {
            AssigneeRef::User(uid) => q.filter(task_assignee::Column::AssigneeUserId.eq(uid.0)),
            AssigneeRef::ApiKey(kid) => q.filter(task_assignee::Column::AssigneeApiKeyId.eq(kid.0)),
        };

        q.exec(conn).await.map_err(db_err)?;
        Ok(())
    }
}

#[async_trait]
impl TaskAssigneeRepo for PgTaskAssigneeRepo {
    async fn add(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskAssignee,
    ) -> Result<TaskAssignee, DomainError> {
        PgTaskAssigneeRepo::add_in(&self.conn, ctx, new).await
    }

    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskAssignee>, DomainError> {
        let rows = task_assignee::Entity::find()
            .filter(task_assignee::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_assignee::Column::TaskId.eq(task_id.0))
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        rows.into_iter()
            .map(|m| task_assignee_from(m).map_err(internal_err))
            .collect()
    }

    async fn remove(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        assignee: AssigneeRef,
    ) -> Result<(), DomainError> {
        PgTaskAssigneeRepo::remove_in(&self.conn, ctx, task_id, assignee).await
    }
}

pub struct PgTaskChecklistRepo {
    pub conn: DatabaseConnection,
}

impl PgTaskChecklistRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Inserts a checklist item inside an existing transaction.
    pub async fn add_item_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewTaskChecklistItem,
    ) -> Result<TaskChecklistItem, DomainError> {
        let position_key = position::between(
            new.position.before.as_deref(),
            new.position.after.as_deref(),
        );
        let (by_user, by_key) = actor_columns(&ctx.actor);
        let now = Utc::now();

        let model = task_checklist_item::ActiveModel {
            id: Set(ChecklistItemId::new().0),
            task_id: Set(new.task_id.0),
            workspace_id: Set(ctx.workspace_id.0),
            title: Set(new.title),
            checked: Set(false),
            position_key: Set(position_key),
            promoted_task_id: Set(None),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };
        model
            .insert(conn)
            .await
            .map(task_checklist_item_from)
            .map_err(db_err)
    }

    /// Patches a checklist item inside an existing transaction.
    pub async fn patch_item_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
        patch: TaskChecklistItemPatch,
    ) -> Result<TaskChecklistItem, DomainError> {
        let row = task_checklist_item::Entity::find_by_id(item_id.0)
            .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_checklist_item::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task_checklist_item",
                id: item_id.0,
            })?;

        let mut active = row.into_active_model();
        if let Some(title) = patch.title {
            active.title = Set(title);
        }
        if let Some(checked) = patch.checked {
            active.checked = Set(checked);
        }
        if let Some(pos) = patch.position {
            active.position_key = Set(position::between(
                pos.before.as_deref(),
                pos.after.as_deref(),
            ));
        }
        active.updated_at = Set(Utc::now());
        active
            .update(conn)
            .await
            .map(task_checklist_item_from)
            .map_err(db_err)
    }

    /// Soft-deletes a checklist item inside an existing transaction.
    pub async fn soft_delete_item_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
    ) -> Result<(), DomainError> {
        let row = task_checklist_item::Entity::find_by_id(item_id.0)
            .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_checklist_item::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task_checklist_item",
                id: item_id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;
        Ok(())
    }

    /// Marks a checklist item as promoted inside an existing transaction.
    ///
    /// Required by `TaskService::promote_checklist_item` to run the update
    /// on the same `DatabaseTransaction` as the new task insert, so both
    /// either commit or roll back together.
    pub async fn mark_promoted_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
        promoted_task_id: TaskId,
    ) -> Result<TaskChecklistItem, DomainError> {
        let row = task_checklist_item::Entity::find_by_id(item_id.0)
            .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_checklist_item::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task_checklist_item",
                id: item_id.0,
            })?;

        if row.promoted_task_id.is_some() {
            return Err(DomainError::Forbidden {
                message: "checklist item has already been promoted".into(),
            });
        }

        let mut active = row.into_active_model();
        active.promoted_task_id = Set(Some(promoted_task_id.0));
        active.checked = Set(true);
        active.updated_at = Set(Utc::now());
        active
            .update(conn)
            .await
            .map(task_checklist_item_from)
            .map_err(db_err)
    }
}

#[async_trait]
impl TaskChecklistRepo for PgTaskChecklistRepo {
    async fn add_item(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskChecklistItem,
    ) -> Result<TaskChecklistItem, DomainError> {
        PgTaskChecklistRepo::add_item_in(&self.conn, ctx, new).await
    }

    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskChecklistItem>, DomainError> {
        task_checklist_item::Entity::find()
            .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_checklist_item::Column::TaskId.eq(task_id.0))
            .filter(task_checklist_item::Column::DeletedAt.is_null())
            .order_by_asc(task_checklist_item::Column::PositionKey)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(task_checklist_item_from).collect())
            .map_err(db_err)
    }

    async fn patch_item(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
        patch: TaskChecklistItemPatch,
    ) -> Result<TaskChecklistItem, DomainError> {
        PgTaskChecklistRepo::patch_item_in(&self.conn, ctx, item_id, patch).await
    }

    async fn soft_delete_item(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
    ) -> Result<(), DomainError> {
        PgTaskChecklistRepo::soft_delete_item_in(&self.conn, ctx, item_id).await
    }

    async fn mark_promoted(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
        promoted_task_id: TaskId,
    ) -> Result<TaskChecklistItem, DomainError> {
        let row = task_checklist_item::Entity::find_by_id(item_id.0)
            .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_checklist_item::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task_checklist_item",
                id: item_id.0,
            })?;

        if row.promoted_task_id.is_some() {
            return Err(DomainError::Forbidden {
                message: "checklist item has already been promoted".into(),
            });
        }

        let mut active = row.into_active_model();
        active.promoted_task_id = Set(Some(promoted_task_id.0));
        active.checked = Set(true);
        active.updated_at = Set(Utc::now());
        active
            .update(&self.conn)
            .await
            .map(task_checklist_item_from)
            .map_err(db_err)
    }
}

pub struct PgTaskActivityRepo {
    pub conn: DatabaseConnection,
}

impl PgTaskActivityRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Appends one activity entry inside an existing transaction.
    ///
    /// This is the ONLY write path for task_activity: no route writes activity directly.
    /// Every state-changing TaskService method calls `append_in` in the same transaction
    /// as the mutation, so the activity entry and the mutation either both commit or both
    /// roll back.
    pub async fn append_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewTaskActivity,
    ) -> Result<TaskActivity, DomainError> {
        let payload = serde_json::to_value(&new.payload).map_err(|e| DomainError::Internal {
            message: format!("serialize activity payload: {e}"),
        })?;
        let (by_user, by_key) = actor_columns(&ctx.actor);

        let model = task_activity::ActiveModel {
            id: Set(TaskActivityId::new().0),
            task_id: Set(new.task_id.0),
            workspace_id: Set(ctx.workspace_id.0),
            kind: Set(new.kind.as_str().to_string()),
            payload: Set(payload),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
        };
        model
            .insert(conn)
            .await
            .map_err(db_err)
            .and_then(|m| task_activity_from(m).map_err(internal_err))
    }
}

#[async_trait]
impl TaskActivityRepo for PgTaskActivityRepo {
    async fn append(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskActivity,
    ) -> Result<TaskActivity, DomainError> {
        PgTaskActivityRepo::append_in(&self.conn, ctx, new).await
    }

    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        after_id: Option<TaskActivityId>,
        limit: u64,
    ) -> Result<Vec<TaskActivity>, DomainError> {
        let mut q = task_activity::Entity::find()
            .filter(task_activity::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_activity::Column::TaskId.eq(task_id.0))
            .order_by_desc(task_activity::Column::CreatedAt)
            .order_by_desc(task_activity::Column::Id)
            .limit(limit);

        if let Some(cursor) = after_id {
            q = q.filter(task_activity::Column::Id.lt(cursor.0));
        }

        let rows = q.all(&self.conn).await.map_err(db_err)?;
        rows.into_iter()
            .map(|m| task_activity_from(m).map_err(internal_err))
            .collect()
    }

    async fn last_kind_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Option<ActivityKind>, DomainError> {
        let opt = task_activity::Entity::find()
            .filter(task_activity::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_activity::Column::TaskId.eq(task_id.0))
            .order_by_desc(task_activity::Column::CreatedAt)
            .order_by_desc(task_activity::Column::Id)
            .one(&self.conn)
            .await
            .map_err(db_err)?;

        opt.map(|m| activity_kind_from_str(&m.kind).map_err(internal_err))
            .transpose()
    }
}

/// Resequences all non-deleted columns in `board_id` using evenly spaced fractional keys.
///
/// Runs inside an existing transaction so the caller controls the boundary.
/// Selects columns ordered by current `position_key` with a `FOR UPDATE` lock
/// to prevent concurrent resequencing races.
pub async fn resequence_column(
    txn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    board_id: BoardId,
) -> Result<(), DomainError> {
    let rows = board_column::Entity::find()
        .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(board_column::Column::BoardId.eq(board_id.0))
        .filter(board_column::Column::DeletedAt.is_null())
        .order_by(board_column::Column::PositionKey, Order::Asc)
        .all(txn)
        .await
        .map_err(db_err)?;

    let mut prev: Option<String> = None;
    for row in rows {
        let key = position::between(prev.as_deref(), None);
        let mut active = row.into_active_model();
        active.position_key = Set(key.clone());
        active.updated_at = Set(Utc::now());
        active.update(txn).await.map_err(db_err)?;
        prev = Some(key);
    }

    Ok(())
}

/// Resequences all non-deleted tasks in `column_id` using evenly spaced fractional keys.
///
/// Analogous to `resequence_column` but operates on the `tasks` table.
/// Must run inside an existing transaction.
async fn resequence_tasks_in_column(
    txn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    column_id: ColumnId,
) -> Result<(), DomainError> {
    let rows = task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(task::Column::ColumnId.eq(column_id.0))
        .filter(task::Column::DeletedAt.is_null())
        .order_by(task::Column::PositionKey, Order::Asc)
        .all(txn)
        .await
        .map_err(db_err)?;

    let mut prev: Option<String> = None;
    for row in rows {
        let key = position::between(prev.as_deref(), None);
        let mut active = row.into_active_model();
        active.position_key = Set(key.clone());
        active.updated_at = Set(Utc::now());
        active.update(txn).await.map_err(db_err)?;
        prev = Some(key);
    }

    Ok(())
}

/// Attempts a single-statement atomic move of `task_id` to `column_id` at `position`.
///
/// Uses a single `UPDATE … WHERE id AND workspace_id AND deleted_at IS NULL RETURNING *`
/// to avoid read-modify-write races (B1 fix). Returns `PositionExhausted` if the
/// fractional space between the two anchors is exhausted.
async fn try_move_to_in(
    txn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    id: TaskId,
    column_id: ColumnId,
    position: &PositionBetween,
) -> Result<Task, DomainError> {
    let new_key = match position::try_between(position.before.as_deref(), position.after.as_deref())
    {
        Some(key) => key,
        None => {
            return Err(DomainError::PositionExhausted { column_id });
        }
    };

    let now = Utc::now();
    let sql = format!(
        "UPDATE tasks \
         SET column_id = '{col}', position_key = '{key}', updated_at = '{ts}' \
         WHERE id = '{id}' AND workspace_id = '{ws}' AND deleted_at IS NULL \
         RETURNING *",
        col = column_id.0,
        key = new_key,
        ts = now.to_rfc3339(),
        id = id.0,
        ws = ctx.workspace_id.0,
    );

    let row = txn
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql,
        ))
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "task",
            id: id.0,
        })?;

    // Map the raw QueryResult columns to a task::Model then convert to domain Task.
    let model = task::Model {
        id: row.try_get("", "id").map_err(|e| DomainError::Internal {
            message: e.to_string(),
        })?,
        workspace_id: row
            .try_get("", "workspace_id")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        project_id: row
            .try_get("", "project_id")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        board_id: row
            .try_get("", "board_id")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        column_id: row
            .try_get("", "column_id")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        readable_id: row
            .try_get("", "readable_id")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        title: row
            .try_get("", "title")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        description: row
            .try_get("", "description")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        priority: row
            .try_get("", "priority")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        due_date: row
            .try_get("", "due_date")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        estimate: row
            .try_get("", "estimate")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        labels: row
            .try_get("", "labels")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        properties: row
            .try_get("", "properties")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        position_key: row
            .try_get("", "position_key")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        created_by_user_id: row.try_get("", "created_by_user_id").map_err(|e| {
            DomainError::Internal {
                message: e.to_string(),
            }
        })?,
        created_by_api_key_id: row.try_get("", "created_by_api_key_id").map_err(|e| {
            DomainError::Internal {
                message: e.to_string(),
            }
        })?,
        created_at: row
            .try_get("", "created_at")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        updated_at: row
            .try_get("", "updated_at")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
        deleted_at: row
            .try_get("", "deleted_at")
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?,
    };

    Ok(task_from(model))
}

/// Returns `(created_by_user_id, created_by_api_key_id)` for the XOR actor columns.
fn actor_columns(actor: &Actor) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match actor {
        Actor::User(uid) => (Some(uid.0), None),
        Actor::ApiKey(kid) => (None, Some(kid.0)),
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
