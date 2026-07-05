use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::boards_tasks::{
        ActivityKind, AssigneeRef, Board, BoardColumn, ColumnPatch, NewBoard, NewTask,
        NewTaskActivity, NewTaskAssignee, NewTaskChecklistItem, NewTaskReference, PositionBetween,
        Task, TaskActivity, TaskAssignee, TaskChecklistItem, TaskChecklistItemPatch, TaskPatch,
        TaskReference,
    },
    entities::events::{
        BoardCreatedPayload, BoardDeletedPayload, ColumnCreatedPayload, ColumnDeletedPayload,
        DomainEvent,
    },
    entities::task_views::{ActorTypeFilter, AssigneeFilter, TaskSort, TaskViewFilters},
    ids::{BoardId, ChecklistItemId, ColumnId, ProjectId, TaskActivityId, TaskId, TaskReferenceId},
    ports::boards_tasks::{
        TaskListCursor, WorkspaceActivityFilters, WorkspaceActivityRow, WorkspaceActivityScope,
    },
    position,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, FromQueryResult, IntoActiveModel, Order, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Statement, TransactionTrait,
};

use crate::persistence::entities::boards_tasks::{
    activity_kind_from_str, board, board_column, board_column_from, board_from, task,
    task_activity, task_activity_from, task_assignee, task_assignee_from, task_checklist_item,
    task_checklist_item_from, task_from, task_reference, task_reference_from,
};
use crate::persistence::entities::status_templates::status_template;
use crate::persistence::repos::PgOutboxRepo;

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
        let txn = self.conn.begin().await.map_err(db_err)?;

        let (by_user, by_key) = actor_columns(&ctx.actor);
        let board_project_id = new.project_id;
        let board_name = new.name.clone();
        let board_model = board::ActiveModel {
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
        let inserted_board = board_model.insert(&txn).await.map_err(db_err)?;
        let board_id = BoardId(inserted_board.id);

        let templates = status_template::Entity::find()
            .filter(status_template::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(status_template::Column::DeletedAt.is_null())
            .order_by_asc(status_template::Column::PositionKey)
            .all(&txn)
            .await
            .map_err(db_err)?;

        let mut prev: Option<String> = None;
        for tpl in templates {
            let position_key = position::between(prev.as_deref(), None);
            let col_model = board_column::ActiveModel {
                id: Set(ColumnId::new().0),
                workspace_id: Set(ctx.workspace_id.0),
                board_id: Set(board_id.0),
                name: Set(tpl.name),
                position_key: Set(position_key.clone()),
                color: Set(tpl.color),
                created_by_user_id: Set(by_user),
                created_by_api_key_id: Set(by_key),
                created_at: Set(Utc::now()),
                updated_at: Set(Utc::now()),
                deleted_at: Set(None),
            };
            col_model.insert(&txn).await.map_err(db_err)?;
            prev = Some(position_key);
        }

        PgOutboxRepo::insert_in(
            &txn,
            ctx,
            Some(board_project_id),
            Some(board_id),
            DomainEvent::BoardCreated(BoardCreatedPayload {
                board_id,
                project_id: board_project_id,
                name: board_name,
            }),
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(board_from(inserted_board))
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
        color: Option<String>,
        position: PositionBetween,
    ) -> Result<BoardColumn, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let board_row = board::Entity::find_by_id(board_id.0)
            .filter(board::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board",
                id: board_id.0,
            })?;

        let board_project_id = ProjectId(board_row.project_id);

        let position_key =
            match position::try_between(position.before.as_deref(), position.after.as_deref()) {
                Some(key) => key,
                None => {
                    let remap = resequence_column(&txn, ctx, board_id).await?;
                    let rebalanced = remap_anchors(&position, &remap);
                    match position::try_between(
                        rebalanced.before.as_deref(),
                        rebalanced.after.as_deref(),
                    ) {
                        Some(key) => key,
                        None => {
                            txn.rollback().await.map_err(db_err)?;
                            return Err(DomainError::PositionExhausted {
                                column_id: ColumnId(board_id.0),
                            });
                        }
                    }
                }
            };

        let (by_user, by_key) = actor_columns(&ctx.actor);
        let col_name = name.clone();
        let model = board_column::ActiveModel {
            id: Set(ColumnId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            board_id: Set(board_id.0),
            name: Set(name),
            position_key: Set(position_key),
            color: Set(color),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        let inserted = model.insert(&txn).await.map_err(db_err)?;
        let column_id = ColumnId(inserted.id);

        PgOutboxRepo::insert_in(
            &txn,
            ctx,
            Some(board_project_id),
            Some(board_id),
            DomainEvent::ColumnCreated(ColumnCreatedPayload {
                board_id,
                column_id,
                name: col_name,
            }),
        )
        .await?;

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
        board_id: BoardId,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = board_column::Entity::find_by_id(column_id.0)
            .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board_column::Column::BoardId.eq(board_id.0))
            .filter(board_column::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board_column",
                id: column_id.0,
            })?;

        let new_key =
            match position::try_between(position.before.as_deref(), position.after.as_deref()) {
                Some(key) => key,
                None => {
                    let remap = resequence_column(&txn, ctx, board_id).await?;
                    let rebalanced = remap_anchors(&position, &remap);
                    match position::try_between(
                        rebalanced.before.as_deref(),
                        rebalanced.after.as_deref(),
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
        board_id: BoardId,
        id: ColumnId,
        patch: ColumnPatch,
    ) -> Result<BoardColumn, DomainError> {
        let row = board_column::Entity::find_by_id(id.0)
            .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board_column::Column::BoardId.eq(board_id.0))
            .filter(board_column::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board_column",
                id: id.0,
            })?;

        let mut active = row.into_active_model();

        if let Some(name) = patch.name {
            active.name = Set(name);
        }

        if let Some(color) = patch.color {
            active.color = Set(color);
        }

        active.updated_at = Set(Utc::now());
        active
            .update(&self.conn)
            .await
            .map(board_column_from)
            .map_err(db_err)
    }

    async fn soft_delete_board(&self, ctx: &WorkspaceCtx, id: BoardId) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = board::Entity::find_by_id(id.0)
            .filter(board::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board",
                id: id.0,
            })?;

        let board_project_id = ProjectId(row.project_id);

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&txn).await.map_err(db_err)?;

        PgOutboxRepo::insert_in(
            &txn,
            ctx,
            Some(board_project_id),
            Some(id),
            DomainEvent::BoardDeleted(BoardDeletedPayload {
                board_id: id,
                project_id: board_project_id,
            }),
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    async fn soft_delete_column(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
        id: ColumnId,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = board_column::Entity::find_by_id(id.0)
            .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(board_column::Column::BoardId.eq(board_id.0))
            .filter(board_column::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "board_column",
                id: id.0,
            })?;

        let live_tasks = task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::ColumnId.eq(id.0))
            .filter(task::Column::DeletedAt.is_null())
            .count(&txn)
            .await
            .map_err(db_err)?;

        if live_tasks > 0 {
            txn.rollback().await.map_err(db_err)?;
            return Err(DomainError::InvalidInput {
                message: "column still has tasks; move or delete them first".to_string(),
            });
        }

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&txn).await.map_err(db_err)?;

        PgOutboxRepo::insert_in(
            &txn,
            ctx,
            None,
            Some(board_id),
            DomainEvent::ColumnDeleted(ColumnDeletedPayload {
                board_id,
                column_id: id,
            }),
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }
}

impl PgBoardRepo {
    /// Batch-loads boards by their IDs, scoped to the workspace.
    ///
    /// Returns only non-deleted boards. Missing IDs (deleted or unknown) are
    /// silently absent from the result, not an error.
    pub async fn list_boards_by_ids(
        &self,
        workspace_id: uuid::Uuid,
        ids: &[uuid::Uuid],
    ) -> Result<Vec<Board>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        board::Entity::find()
            .filter(board::Column::WorkspaceId.eq(workspace_id))
            .filter(board::Column::Id.is_in(ids.to_vec()))
            .filter(board::Column::DeletedAt.is_null())
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(board_from).collect())
            .map_err(db_err)
    }

    /// Batch-loads columns by their IDs, scoped to the workspace.
    ///
    /// Returns only non-deleted columns. Missing IDs are silently absent.
    pub async fn list_columns_by_ids(
        &self,
        workspace_id: uuid::Uuid,
        ids: &[uuid::Uuid],
    ) -> Result<Vec<BoardColumn>, DomainError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        board_column::Entity::find()
            .filter(board_column::Column::WorkspaceId.eq(workspace_id))
            .filter(board_column::Column::Id.is_in(ids.to_vec()))
            .filter(board_column::Column::DeletedAt.is_null())
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(board_column_from).collect())
            .map_err(db_err)
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
        // The destination column may live on any board/project in the workspace:
        // a move can cross boards. Resolve the column's board and project so the
        // task adopts them; for a same-board move these equal the current values.
        let (board_id, project_id) = resolve_column_target_in(conn, ctx, column_id).await?;

        // Reject positions where a provided anchor string is not a valid
        // fractional-index key. Invalid anchors are silently dropped to None
        // by try_between, which turns both-invalid into (None, None) → default
        // midpoint. That silent success is wrong: the caller said "between X and
        // Y" and we must honour that or surface PositionExhausted.
        if anchor_is_invalid(position.before.as_deref())
            || anchor_is_invalid(position.after.as_deref())
        {
            return Err(DomainError::PositionExhausted { column_id });
        }

        match try_move_to_in(conn, ctx, id, column_id, board_id, project_id, &position).await {
            Ok(task) => Ok(task),
            Err(DomainError::PositionExhausted { .. }) => {
                let remap = resequence_tasks_in_column(conn, ctx, column_id).await?;
                let rebalanced = remap_anchors(&position, &remap);

                // If both original anchors were specified but neither appears in
                // the resequence map, they are phantom keys that do not correspond
                // to any row in this column. Resequencing cannot help; return
                // PositionExhausted rather than silently placing at the default midpoint.
                if position.before.is_some()
                    && position.after.is_some()
                    && rebalanced.before.is_none()
                    && rebalanced.after.is_none()
                {
                    return Err(DomainError::PositionExhausted { column_id });
                }

                try_move_to_in(conn, ctx, id, column_id, board_id, project_id, &rebalanced).await
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
        parent_task_id: Option<TaskId>,
    ) -> Result<Task, DomainError> {
        let row = conn
            .query_one_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE projects \
                 SET next_task_number = next_task_number + 1 \
                 WHERE id = $1 AND workspace_id = $2 \
                 RETURNING next_task_number, task_prefix",
                [new.project_id.0.into(), ctx.workspace_id.0.into()],
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

        let position_key = match position::try_between(
            new.position.before.as_deref(),
            new.position.after.as_deref(),
        ) {
            Some(key) => key,
            None => {
                let remap = resequence_tasks_in_column(conn, ctx, new.column_id).await?;
                let rebalanced = remap_anchors(&new.position, &remap);
                position::try_between(rebalanced.before.as_deref(), rebalanced.after.as_deref())
                    .ok_or(DomainError::PositionExhausted {
                        column_id: new.column_id,
                    })?
            }
        };
        let (by_user, by_key) = actor_columns(&ctx.actor);
        let now = Utc::now();

        let model = task::ActiveModel {
            id: Set(TaskId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            project_id: Set(new.project_id.0),
            board_id: Set(new.board_id.0),
            column_id: Set(new.column_id.0),
            parent_task_id: Set(parent_task_id.map(|t| t.0)),
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
        let task = PgTaskRepo::create_in(&txn, ctx, new, None).await?;
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
            .filter(task::Column::ParentTaskId.is_null())
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
            .filter(task::Column::ParentTaskId.is_null())
            .filter(task::Column::DeletedAt.is_null())
            .order_by_asc(task::Column::PositionKey)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(task_from).collect())
            .map_err(db_err)
    }

    async fn list_children(
        &self,
        ctx: &WorkspaceCtx,
        parent_task_id: TaskId,
    ) -> Result<Vec<Task>, DomainError> {
        task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::ParentTaskId.eq(parent_task_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .order_by_asc(task::Column::PositionKey)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(task_from).collect())
            .map_err(db_err)
    }

    async fn count_children_for_parents(
        &self,
        ctx: &WorkspaceCtx,
        parent_task_ids: &[TaskId],
    ) -> Result<Vec<(TaskId, i64)>, DomainError> {
        if parent_task_ids.is_empty() {
            return Ok(Vec::new());
        }

        #[derive(FromQueryResult)]
        struct ChildCountRow {
            parent_task_id: uuid::Uuid,
            child_count: i64,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();

        // $1 — workspace_id
        values.push(ctx.workspace_id.0.into());

        // $2..$N — parent_task_id IN list
        let placeholders: String = parent_task_ids
            .iter()
            .map(|id| {
                values.push(id.0.into());
                format!("${}", values.len())
            })
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "SELECT t.parent_task_id AS parent_task_id, count(*) AS child_count
             FROM tasks t
             WHERE t.workspace_id = $1
               AND t.parent_task_id IN ({placeholders})
               AND t.deleted_at IS NULL
             GROUP BY t.parent_task_id"
        );

        let rows = ChildCountRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows
            .into_iter()
            .map(|r| (TaskId(r.parent_task_id), r.child_count))
            .collect())
    }

    async fn detach(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<Task, DomainError> {
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
        active.parent_task_id = Set(None);
        active.updated_at = Set(Utc::now());

        active
            .update(&self.conn)
            .await
            .map(task_from)
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

    async fn list_by_workspace_filtered(
        &self,
        ctx: &WorkspaceCtx,
        filters: &TaskViewFilters,
        after: Option<TaskListCursor>,
        limit: u64,
    ) -> Result<Vec<Task>, DomainError> {
        use chrono::{TimeZone, Utc};

        let mut values: Vec<sea_orm::Value> = Vec::new();

        // $1 — workspace_id (always-on predicate)
        values.push(ctx.workspace_id.0.into());

        let mut clauses: Vec<String> = Vec::new();

        // Optional filter: board_id
        if let Some(board_id) = &filters.board_id {
            values.push(board_id.0.into());
            clauses.push(format!("t.board_id = ${}", values.len()));
        }

        // Optional filter: column_ids — expanded to OR conditions over individual params
        if !filters.column_ids.is_empty() {
            let mut parts = Vec::new();
            for col_id in &filters.column_ids {
                values.push(col_id.0.into());
                parts.push(format!("t.column_id = ${}", values.len()));
            }
            clauses.push(format!("({})", parts.join(" OR ")));
        }

        // Optional filter: priorities — expanded to OR conditions over individual params
        if !filters.priorities.is_empty() {
            let mut parts = Vec::new();
            for priority in &filters.priorities {
                values.push(priority.as_str().to_string().into());
                parts.push(format!("t.priority = ${}", values.len()));
            }
            clauses.push(format!("({})", parts.join(" OR ")));
        }

        // Optional filter: labels (array-contains all requested labels)
        for label in &filters.labels {
            values.push(label.clone().into());
            clauses.push(format!("${} = ANY(t.labels)", values.len()));
        }

        // Optional filter: actor_type (which column created_by_* is non-null)
        if let Some(actor_type) = &filters.actor_type {
            let cond = match actor_type {
                ActorTypeFilter::User => "t.created_by_user_id IS NOT NULL",
                ActorTypeFilter::ApiKey => "t.created_by_api_key_id IS NOT NULL",
            };
            clauses.push(cond.to_string());
        }

        // Optional filter: assignee EXISTS subquery
        if let Some(assignee) = &filters.assignee {
            let exists_cond = match assignee {
                AssigneeFilter::Me => match &ctx.actor {
                    Actor::User(uid) => {
                        values.push(uid.0.into());
                        let pn = values.len();
                        format!(
                            "EXISTS (
                                    SELECT 1 FROM task_assignees ta
                                    WHERE ta.task_id = t.id
                                      AND ta.workspace_id = t.workspace_id
                                      AND ta.assignee_user_id = ${pn}
                                )"
                        )
                    }
                    Actor::ApiKey(kid) => {
                        values.push(kid.0.into());
                        let pn = values.len();
                        format!(
                            "EXISTS (
                                    SELECT 1 FROM task_assignees ta
                                    WHERE ta.task_id = t.id
                                      AND ta.workspace_id = t.workspace_id
                                      AND ta.assignee_api_key_id = ${pn}
                                )"
                        )
                    }
                },
                AssigneeFilter::User(uid) => {
                    values.push(uid.0.into());
                    let pn = values.len();
                    format!(
                        "EXISTS (
                            SELECT 1 FROM task_assignees ta
                            WHERE ta.task_id = t.id
                              AND ta.workspace_id = t.workspace_id
                              AND ta.assignee_user_id = ${pn}
                        )"
                    )
                }
                AssigneeFilter::ApiKey(kid) => {
                    values.push(kid.0.into());
                    let pn = values.len();
                    format!(
                        "EXISTS (
                            SELECT 1 FROM task_assignees ta
                            WHERE ta.task_id = t.id
                              AND ta.workspace_id = t.workspace_id
                              AND ta.assignee_api_key_id = ${pn}
                        )"
                    )
                }
            };
            clauses.push(exists_cond);
        }

        // Keyset pagination: (updated_at, id) < ($ts, $id) for DESC ordering
        let cursor_cond = if let Some(ref cursor) = after {
            let micros = match &cursor.sort_value {
                serde_json::Value::Number(n) => {
                    n.as_i64().ok_or_else(|| DomainError::Internal {
                        message: "cursor sort_value is not a valid i64".to_string(),
                    })?
                }
                other => {
                    return Err(DomainError::Internal {
                        message: format!("unexpected cursor sort_value type: {other:?}"),
                    });
                }
            };
            let ts =
                Utc.timestamp_micros(micros)
                    .single()
                    .ok_or_else(|| DomainError::Internal {
                        message: format!("cursor timestamp {micros} is out of range"),
                    })?;
            values.push(ts.into());
            let pts = values.len();
            values.push(cursor.id.0.into());
            let pid = values.len();
            format!("AND (t.updated_at, t.id) < (${pts}, ${pid})")
        } else {
            String::new()
        };

        // ORDER BY is determined exclusively by the sort enum — no user strings ever
        // reach the query. Unknown sort values are rejected at the route layer (→ 400)
        // before this method is called.
        let order_clause = match filters.sort.as_ref().unwrap_or(&TaskSort::UpdatedDesc) {
            TaskSort::UpdatedDesc => "ORDER BY t.updated_at DESC, t.id DESC",
            TaskSort::UpdatedAsc => "ORDER BY t.updated_at ASC, t.id ASC",
            TaskSort::CreatedDesc => "ORDER BY t.created_at DESC, t.id DESC",
            TaskSort::CreatedAsc => "ORDER BY t.created_at ASC, t.id ASC",
            TaskSort::PriorityDesc => {
                "ORDER BY CASE t.priority \
                    WHEN 'urgent' THEN 1 \
                    WHEN 'high' THEN 2 \
                    WHEN 'medium' THEN 3 \
                    WHEN 'low' THEN 4 \
                    ELSE 5 END ASC, t.id ASC"
            }
            TaskSort::TitleAsc => "ORDER BY t.title ASC, t.id ASC",
        };

        let extra_where = if clauses.is_empty() {
            String::new()
        } else {
            format!("AND {}", clauses.join("\n  AND "))
        };

        values.push((limit as i64).into());
        let limit_param = values.len();

        let sql = format!(
            r#"
            SELECT t.*
            FROM tasks t
            WHERE t.workspace_id = $1
              AND t.parent_task_id IS NULL
              AND t.deleted_at IS NULL
              {extra_where}
              {cursor_cond}
            {order_clause}
            LIMIT ${limit_param}
            "#
        );

        task::Model::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map(|rows| rows.into_iter().map(task_from).collect())
        .map_err(db_err)
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

        validate_reference(
            new.source_task_id,
            new.kind.clone(),
            new.target_task_id,
            new.target_document_id,
        )?;

        let target_id = reference_target_id(&new);
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
            .map_err(|e| classify_reference_insert_err(e, target_id))
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

        validate_reference(
            new.source_task_id,
            new.kind.clone(),
            new.target_task_id,
            new.target_document_id,
        )?;

        let target_id = reference_target_id(&new);
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
            .map_err(|e| classify_reference_insert_err(e, target_id))
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
        task_reference::Entity::delete_by_id(id.0)
            .filter(task_reference::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .exec(&self.conn)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

impl PgTaskReferenceRepo {
    /// Deletes a task reference inside an existing transaction.
    ///
    /// `expected_source_task_id` is the task authorized by the caller. The
    /// reference must originate from it; a reference on another task in the same
    /// workspace resolves to `NotFound` rather than being silently deleted.
    pub async fn delete_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        expected_source_task_id: TaskId,
        id: TaskReferenceId,
    ) -> Result<(), DomainError> {
        let result = task_reference::Entity::delete_by_id(id.0)
            .filter(task_reference::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_reference::Column::SourceTaskId.eq(expected_source_task_id.0))
            .exec(conn)
            .await
            .map_err(db_err)?;

        if result.rows_affected == 0 {
            return Err(DomainError::NotFound {
                entity: "task_reference",
                id: id.0,
            });
        }

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
        let principal_id = match new.assignee {
            AssigneeRef::User(uid) => uid.0,
            AssigneeRef::ApiKey(kid) => kid.0,
        };
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
            .map_err(|e| classify_insert_err(e, principal_id))
            .and_then(|m| task_assignee_from(m).map_err(internal_err))
    }

    /// Removes a task assignee inside an existing transaction.
    ///
    /// Returns `NotFound` when the assignee was not present on this task so that
    /// callers can surface a 404 and avoid writing phantom `Unassigned` activity.
    pub async fn remove_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        assignee: AssigneeRef,
    ) -> Result<(), DomainError> {
        let principal_id = match assignee {
            AssigneeRef::User(uid) => uid.0,
            AssigneeRef::ApiKey(kid) => kid.0,
        };

        let mut q = task_assignee::Entity::delete_many()
            .filter(task_assignee::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_assignee::Column::TaskId.eq(task_id.0));

        q = match assignee {
            AssigneeRef::User(uid) => q.filter(task_assignee::Column::AssigneeUserId.eq(uid.0)),
            AssigneeRef::ApiKey(kid) => q.filter(task_assignee::Column::AssigneeApiKeyId.eq(kid.0)),
        };

        let result = q.exec(conn).await.map_err(db_err)?;

        if result.rows_affected == 0 {
            return Err(DomainError::NotFound {
                entity: "task_assignee",
                id: principal_id,
            });
        }

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

    /// Returns all task assignees for a single task.
    ///
    /// Keep-visible invariant (Slack/ClickUp model): user assignees are intentionally
    /// NOT filtered by `disabled_at`. A disabled user must still appear on tasks they
    /// are assigned to, marked with `account_status = "deactivated"` at the display
    /// layer. Adding a `LEFT JOIN users ... AND u.disabled_at IS NULL` here would
    /// silently break that contract — do not add it.
    ///
    /// Revoked api-keys ARE hidden (the `ak.revoked_at IS NULL` filter stays) because
    /// key revocation is tier-2 and behaves differently from account deactivation.
    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskAssignee>, DomainError> {
        let rows = task_assignee::Model::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT ta.task_id, ta.workspace_id, ta.assignee_user_id, ta.assignee_api_key_id,
                    ta.assigned_by_user_id, ta.assigned_by_api_key_id, ta.assigned_at
             FROM task_assignees ta
             LEFT JOIN api_keys ak ON ak.id = ta.assignee_api_key_id
             WHERE ta.workspace_id = $1
               AND ta.task_id = $2
               AND (ta.assignee_api_key_id IS NULL OR ak.revoked_at IS NULL)",
            [ctx.workspace_id.0.into(), task_id.0.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        rows.into_iter()
            .map(|m| task_assignee_from(m).map_err(internal_err))
            .collect()
    }

    /// Returns all task assignees for a batch of tasks (board/list view path).
    ///
    /// Keep-visible invariant: same as `list_for_task` — user assignees are NOT
    /// filtered by `disabled_at`. See that function's doc for the full rationale.
    async fn list_for_tasks(
        &self,
        ctx: &WorkspaceCtx,
        task_ids: &[TaskId],
    ) -> Result<Vec<TaskAssignee>, DomainError> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();

        // $1 — workspace_id
        values.push(ctx.workspace_id.0.into());

        // $2..$N — task_id IN list
        let placeholders: String = task_ids
            .iter()
            .map(|t| {
                values.push(t.0.into());
                format!("${}", values.len())
            })
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "SELECT ta.task_id, ta.workspace_id, ta.assignee_user_id, ta.assignee_api_key_id,
                    ta.assigned_by_user_id, ta.assigned_by_api_key_id, ta.assigned_at
             FROM task_assignees ta
             LEFT JOIN api_keys ak ON ak.id = ta.assignee_api_key_id
             WHERE ta.workspace_id = $1
               AND ta.task_id IN ({placeholders})
               AND (ta.assignee_api_key_id IS NULL OR ak.revoked_at IS NULL)"
        );

        let rows = task_assignee::Model::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
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
        let position_key = match position::try_between(
            new.position.before.as_deref(),
            new.position.after.as_deref(),
        ) {
            Some(key) => key,
            None => {
                let remap = resequence_checklist_items(conn, ctx, new.task_id).await?;
                let rebalanced = remap_anchors(&new.position, &remap);
                position::try_between(rebalanced.before.as_deref(), rebalanced.after.as_deref())
                    .ok_or(DomainError::PositionExhausted {
                        column_id: ColumnId(new.task_id.0),
                    })?
            }
        };
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
    ///
    /// `expected_task_id` is the task authorized by the caller's extractor. The
    /// item must belong to it; an item from another task in the same workspace
    /// resolves to `NotFound`, closing the intra-workspace IDOR path.
    pub async fn patch_item_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        expected_task_id: TaskId,
        item_id: ChecklistItemId,
        patch: TaskChecklistItemPatch,
    ) -> Result<TaskChecklistItem, DomainError> {
        let row = task_checklist_item::Entity::find_by_id(item_id.0)
            .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_checklist_item::Column::TaskId.eq(expected_task_id.0))
            .filter(task_checklist_item::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task_checklist_item",
                id: item_id.0,
            })?;

        let task_id = TaskId(row.task_id);

        let mut active = row.into_active_model();
        if let Some(title) = patch.title {
            active.title = Set(title);
        }
        if let Some(checked) = patch.checked {
            active.checked = Set(checked);
        }
        if let Some(pos) = patch.position {
            let new_key = match position::try_between(pos.before.as_deref(), pos.after.as_deref()) {
                Some(key) => key,
                None => {
                    let remap = resequence_checklist_items(conn, ctx, task_id).await?;
                    let rebalanced = remap_anchors(&pos, &remap);
                    position::try_between(rebalanced.before.as_deref(), rebalanced.after.as_deref())
                        .ok_or(DomainError::PositionExhausted {
                            column_id: ColumnId(task_id.0),
                        })?
                }
            };
            active.position_key = Set(new_key);
        }
        active.updated_at = Set(Utc::now());
        active
            .update(conn)
            .await
            .map(task_checklist_item_from)
            .map_err(db_err)
    }

    /// Soft-deletes a checklist item inside an existing transaction.
    ///
    /// `expected_task_id` scopes the item to the authorized task; a mismatch
    /// resolves to `NotFound` (intra-workspace IDOR guard).
    pub async fn soft_delete_item_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        expected_task_id: TaskId,
        item_id: ChecklistItemId,
    ) -> Result<(), DomainError> {
        let row = task_checklist_item::Entity::find_by_id(item_id.0)
            .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_checklist_item::Column::TaskId.eq(expected_task_id.0))
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
        expected_task_id: TaskId,
        item_id: ChecklistItemId,
        promoted_task_id: TaskId,
    ) -> Result<TaskChecklistItem, DomainError> {
        let row = task_checklist_item::Entity::find_by_id(item_id.0)
            .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_checklist_item::Column::TaskId.eq(expected_task_id.0))
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

    /// Resolves the owning task id of a live checklist item within the workspace.
    ///
    /// Used by the unscoped trait methods to feed the task-scoped `_in` variants
    /// a self-consistent constraint; the HTTP handlers pass the authorized task.
    async fn item_task_id(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
    ) -> Result<TaskId, DomainError> {
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

        Ok(TaskId(row.task_id))
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
        let task_id = self.item_task_id(ctx, item_id).await?;
        PgTaskChecklistRepo::patch_item_in(&self.conn, ctx, task_id, item_id, patch).await
    }

    async fn soft_delete_item(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
    ) -> Result<(), DomainError> {
        let task_id = self.item_task_id(ctx, item_id).await?;
        PgTaskChecklistRepo::soft_delete_item_in(&self.conn, ctx, task_id, item_id).await
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

/// Consecutive `field_changed` activity on the same task field, by the same actor,
/// within this window is merged into a single entry rather than appended as a new
/// row. Chosen generously so an ordinary editing session (a burst of debounced
/// autosaves on a description) collapses to one activity entry.
const FIELD_CHANGE_COALESCE_WINDOW_MINUTES: i64 = 10;

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

    /// Appends a `field_changed` activity, or merges it into a recent same-field
    /// change by the same actor instead of adding a new row.
    ///
    /// A burst of autosaves on one field (e.g. editing a task description) would
    /// otherwise append one activity entry per save, flooding the feed. When the
    /// most recent `field_changed` on this task, by this actor, within
    /// [`FIELD_CHANGE_COALESCE_WINDOW_MINUTES`], targets the same field, that entry
    /// is updated in place: the original `old_value` (the burst's starting point)
    /// is kept, `new_value` becomes the latest, and `created_at` is bumped so the
    /// single entry reflects the last edit. A change to a different field, a
    /// different actor, or an older entry starts a fresh row.
    pub async fn append_or_coalesce_field_change_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        field: String,
        old_value: serde_json::Value,
        new_value: serde_json::Value,
    ) -> Result<TaskActivity, DomainError> {
        use atlas_domain::entities::boards_tasks::ActivityPayload;

        let (by_user, by_key) = actor_columns(&ctx.actor);
        let cutoff = Utc::now() - chrono::Duration::minutes(FIELD_CHANGE_COALESCE_WINDOW_MINUTES);

        let mut query = task_activity::Entity::find()
            .filter(task_activity::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_activity::Column::TaskId.eq(task_id.0))
            .filter(task_activity::Column::Kind.eq(ActivityKind::FieldChanged.as_str()))
            .filter(task_activity::Column::CreatedAt.gte(cutoff))
            .order_by_desc(task_activity::Column::CreatedAt)
            .order_by_desc(task_activity::Column::Id);

        // Never merge across principals: a different actor's edit is its own entry.
        query = match (by_user, by_key) {
            (Some(uid), _) => query.filter(task_activity::Column::CreatedByUserId.eq(uid)),
            (_, Some(kid)) => query.filter(task_activity::Column::CreatedByApiKeyId.eq(kid)),
            (None, None) => query,
        };

        if let Some(model) = query.one(conn).await.map_err(db_err)?
            && let Ok(ActivityPayload::FieldChanged {
                field: recent_field,
                old_value: original_old,
                ..
            }) = serde_json::from_value::<ActivityPayload>(model.payload.clone())
            && recent_field == field
        {
            let merged = ActivityPayload::FieldChanged {
                field,
                old_value: original_old,
                new_value,
            };
            let payload = serde_json::to_value(&merged).map_err(|e| DomainError::Internal {
                message: format!("serialize activity payload: {e}"),
            })?;

            let mut active = model.into_active_model();
            active.payload = Set(payload);
            active.created_at = Set(Utc::now());

            return active
                .update(conn)
                .await
                .map_err(db_err)
                .and_then(|m| task_activity_from(m).map_err(internal_err));
        }

        Self::append_in(
            conn,
            ctx,
            NewTaskActivity {
                task_id,
                kind: ActivityKind::FieldChanged,
                payload: ActivityPayload::FieldChanged {
                    field,
                    old_value,
                    new_value,
                },
            },
        )
        .await
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

    async fn list_for_workspace(
        &self,
        ctx: &WorkspaceCtx,
        scope: WorkspaceActivityScope,
        filters: WorkspaceActivityFilters,
        after: Option<(chrono::DateTime<Utc>, TaskActivityId)>,
        limit: u64,
    ) -> Result<Vec<WorkspaceActivityRow>, DomainError> {
        use sea_orm::FromQueryResult;

        #[derive(Debug, FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            task_id: uuid::Uuid,
            workspace_id: uuid::Uuid,
            kind: String,
            payload: serde_json::Value,
            created_by_user_id: Option<uuid::Uuid>,
            created_by_api_key_id: Option<uuid::Uuid>,
            created_at: chrono::DateTime<Utc>,
            task_readable_id: String,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();

        // $1 — workspace_id
        values.push(ctx.workspace_id.0.into());
        let ws_param = values.len();

        // Admin bypass: return all without id-set filtering.
        let scope_cond = if scope.is_admin {
            String::new()
        } else {
            let project_cond = if scope.project_ids.is_empty() {
                "FALSE".to_string()
            } else {
                let placeholders: String = scope
                    .project_ids
                    .iter()
                    .map(|pid| {
                        values.push(pid.0.into());
                        format!("${}", values.len())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("t.project_id = ANY(ARRAY[{placeholders}]::uuid[])")
            };

            let board_cond = if scope.board_ids.is_empty() {
                "FALSE".to_string()
            } else {
                let placeholders: String = scope
                    .board_ids
                    .iter()
                    .map(|bid| {
                        values.push(bid.0.into());
                        format!("${}", values.len())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("t.board_id = ANY(ARRAY[{placeholders}]::uuid[])")
            };

            format!("AND ({project_cond} OR {board_cond})")
        };

        // Actor-type filter
        let actor_cond = match filters.actor_type {
            Some(ActorTypeFilter::User) => "AND a.created_by_user_id IS NOT NULL".to_string(),
            Some(ActorTypeFilter::ApiKey) => "AND a.created_by_api_key_id IS NOT NULL".to_string(),
            None => String::new(),
        };

        // Date range filters
        let from_cond = if let Some(from) = filters.from {
            values.push(from.into());
            format!("AND a.created_at >= ${}", values.len())
        } else {
            String::new()
        };

        let to_cond = if let Some(to) = filters.to {
            values.push(to.into());
            format!("AND a.created_at <= ${}", values.len())
        } else {
            String::new()
        };

        // Keyset pagination cursor: (created_at DESC, id DESC)
        let cursor_cond = if let Some((ts, aid)) = after {
            values.push(ts.into());
            let ts_param = values.len();
            values.push(aid.0.into());
            let id_param = values.len();
            format!("AND (a.created_at, a.id) < (${ts_param}, ${id_param})")
        } else {
            String::new()
        };

        values.push((limit as i64).into());
        let limit_param = values.len();

        let sql = format!(
            r#"
            SELECT
                a.id,
                a.task_id,
                a.workspace_id,
                a.kind,
                a.payload,
                a.created_by_user_id,
                a.created_by_api_key_id,
                a.created_at,
                t.readable_id AS task_readable_id
            FROM task_activity a
            JOIN tasks t ON t.id = a.task_id AND t.workspace_id = a.workspace_id
            WHERE a.workspace_id = ${ws_param}
              AND t.deleted_at IS NULL
              {scope_cond}
              {actor_cond}
              {from_cond}
              {to_cond}
              {cursor_cond}
            ORDER BY a.created_at DESC, a.id DESC
            LIMIT ${limit_param}
            "#,
        );

        let rows = Row::find_by_statement(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        rows.into_iter()
            .map(|row| {
                let kind = activity_kind_from_str(&row.kind).map_err(internal_err)?;
                let actor = if let Some(uid) = row.created_by_user_id {
                    Actor::User(atlas_domain::ids::UserId(uid))
                } else if let Some(kid) = row.created_by_api_key_id {
                    Actor::ApiKey(atlas_domain::ids::ApiKeyId(kid))
                } else {
                    return Err(DomainError::Internal {
                        message: "task_activity row has no actor".into(),
                    });
                };
                let payload: atlas_domain::entities::boards_tasks::ActivityPayload =
                    serde_json::from_value(row.payload).map_err(|e| DomainError::Internal {
                        message: format!("deserialize activity payload: {e}"),
                    })?;
                let activity = TaskActivity {
                    id: TaskActivityId(row.id),
                    task_id: TaskId(row.task_id),
                    workspace_id: atlas_domain::ids::WorkspaceId(row.workspace_id),
                    kind,
                    payload,
                    actor,
                    created_at: row.created_at,
                };
                Ok(WorkspaceActivityRow {
                    activity,
                    task_readable_id: row.task_readable_id,
                })
            })
            .collect()
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
) -> Result<Vec<(String, String)>, DomainError> {
    let rows = board_column::Entity::find()
        .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(board_column::Column::BoardId.eq(board_id.0))
        .filter(board_column::Column::DeletedAt.is_null())
        .order_by(board_column::Column::PositionKey, Order::Asc)
        .order_by(board_column::Column::Id, Order::Asc)
        .lock_exclusive()
        .all(txn)
        .await
        .map_err(db_err)?;

    let mut remap = Vec::with_capacity(rows.len());
    let mut prev: Option<String> = None;
    for row in rows {
        let old_key = row.position_key.clone();
        let key = position::between(prev.as_deref(), None);
        let mut active = row.into_active_model();
        active.position_key = Set(key.clone());
        active.updated_at = Set(Utc::now());
        active.update(txn).await.map_err(db_err)?;
        remap.push((old_key, key.clone()));
        prev = Some(key);
    }

    Ok(remap)
}

/// Resequences all non-deleted tasks in `column_id` using evenly spaced fractional keys.
///
/// Analogous to `resequence_column` but operates on the `tasks` table.
/// Selects with a `FOR UPDATE` lock for the same reason as `resequence_column`:
/// to serialize concurrent resequencing of the same column.
/// Must run inside an existing transaction.
async fn resequence_tasks_in_column(
    txn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    column_id: ColumnId,
) -> Result<Vec<(String, String)>, DomainError> {
    let rows = task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(task::Column::ColumnId.eq(column_id.0))
        .filter(task::Column::DeletedAt.is_null())
        .order_by(task::Column::PositionKey, Order::Asc)
        .order_by(task::Column::Id, Order::Asc)
        .lock_exclusive()
        .all(txn)
        .await
        .map_err(db_err)?;

    let mut remap = Vec::with_capacity(rows.len());
    let mut prev: Option<String> = None;
    for row in rows {
        let old_key = row.position_key.clone();
        let key = position::between(prev.as_deref(), None);
        let mut active = row.into_active_model();
        active.position_key = Set(key.clone());
        active.updated_at = Set(Utc::now());
        active.update(txn).await.map_err(db_err)?;
        remap.push((old_key, key.clone()));
        prev = Some(key);
    }

    Ok(remap)
}

/// Resequences all non-deleted checklist items for `task_id` using evenly spaced fractional keys.
///
/// Selects with a `FOR UPDATE` lock for the same reason as `resequence_column`:
/// to serialize concurrent resequencing of the same task's checklist.
/// Must run inside an existing transaction.
async fn resequence_checklist_items(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    task_id: TaskId,
) -> Result<Vec<(String, String)>, DomainError> {
    let rows = task_checklist_item::Entity::find()
        .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(task_checklist_item::Column::TaskId.eq(task_id.0))
        .filter(task_checklist_item::Column::DeletedAt.is_null())
        .order_by(task_checklist_item::Column::PositionKey, Order::Asc)
        .order_by(task_checklist_item::Column::Id, Order::Asc)
        .lock_exclusive()
        .all(conn)
        .await
        .map_err(db_err)?;

    let mut remap = Vec::with_capacity(rows.len());
    let mut prev: Option<String> = None;
    for row in rows {
        let old_key = row.position_key.clone();
        let key = position::between(prev.as_deref(), None);
        let mut active = row.into_active_model();
        active.position_key = Set(key.clone());
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;
        remap.push((old_key, key.clone()));
        prev = Some(key);
    }

    Ok(remap)
}

/// Re-derives a `PositionBetween` against the post-resequence keys.
///
/// After a resequence, the client's raw anchor keys are stale. `remap` is the
/// ordered list of (old_key, new_key) pairs the resequence produced, in row
/// order. Each present anchor is translated to its neighbor's NEW key so
/// `try_between` operates on live anchors; retrying on the result lets the
/// rebalance actually create room instead of re-failing on the same keys.
///
/// Exhaustion only arises when the two anchors collide (`before == after`,
/// i.e. duplicate-keyed neighbors) or invert. When `before == after`, the
/// resequence has split those colliding rows into distinct keys: we translate
/// `before` to the FIRST occurrence's new key and `after` to the SECOND, so the
/// insert lands between the now-separated neighbors. A non-colliding anchor maps
/// to its single new key; an anchor with no surviving row becomes `None`.
fn remap_anchors(original: &PositionBetween, remap: &[(String, String)]) -> PositionBetween {
    let lookup = |old: &str| -> Vec<&String> {
        remap
            .iter()
            .filter(|(o, _)| o == old)
            .map(|(_, n)| n)
            .collect()
    };

    match (&original.before, &original.after) {
        (Some(b), Some(a)) if b == a => {
            // Equal anchors only become placeable if the colliding key was backed
            // by TWO distinct rows that the resequence has now split. With a single
            // backing row (or none), the slot stays genuinely unplaceable: keep the
            // anchors equal so the retry still fails into PositionExhausted.
            let news = lookup(b);
            match (news.first(), news.get(1)) {
                (Some(first), Some(second)) => PositionBetween {
                    before: Some((*first).clone()),
                    after: Some((*second).clone()),
                },
                (Some(only), None) => PositionBetween {
                    before: Some((*only).clone()),
                    after: Some((*only).clone()),
                },
                _ => PositionBetween {
                    before: original.before.clone(),
                    after: original.after.clone(),
                },
            }
        }
        _ => {
            let translate = |anchor: &Option<String>| -> Option<String> {
                anchor
                    .as_ref()
                    .and_then(|key| lookup(key).first().map(|s| (*s).clone()))
            };
            PositionBetween {
                before: translate(&original.before),
                after: translate(&original.after),
            }
        }
    }
}

/// Attempts a single-statement atomic move of `task_id` to `column_id` at `position`.
///
/// Uses a single `UPDATE … WHERE id AND workspace_id AND deleted_at IS NULL RETURNING *`
/// to avoid read-modify-write races (B1 fix). Returns `PositionExhausted` if the
/// fractional space between the two anchors is exhausted.
/// Resolves a destination column to the board and project it belongs to,
/// enforcing that the column is live and in the caller's workspace.
///
/// Cross-board moves are allowed: the target column may belong to any
/// board/project in the workspace, and the moved task adopts that column's board
/// and project. A non-existent column is a client error (422 InvalidInput); a
/// column whose board is missing is an internal inconsistency (404 board).
async fn resolve_column_target_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    column_id: ColumnId,
) -> Result<(BoardId, ProjectId), DomainError> {
    let column = board_column::Entity::find_by_id(column_id.0)
        .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(board_column::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::InvalidInput {
            message: "column does not exist in this workspace".into(),
        })?;

    let board = board::Entity::find_by_id(column.board_id)
        .filter(board::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(board::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "board",
            id: column.board_id,
        })?;

    Ok((BoardId(board.id), ProjectId(board.project_id)))
}

async fn try_move_to_in(
    txn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    id: TaskId,
    column_id: ColumnId,
    board_id: BoardId,
    project_id: ProjectId,
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

    // board_id and project_id are written too so a cross-board move keeps the
    // task's board/project consistent with its new column; for a same-board move
    // they equal the current values, leaving the row unchanged on those fields.
    let row = txn
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE tasks \
             SET column_id = $1, board_id = $2, project_id = $3, position_key = $4, updated_at = $5 \
             WHERE id = $6 AND workspace_id = $7 AND deleted_at IS NULL \
             RETURNING *",
            [
                column_id.0.into(),
                board_id.0.into(),
                project_id.0.into(),
                new_key.into(),
                now.into(),
                id.0.into(),
                ctx.workspace_id.0.into(),
            ],
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
        parent_task_id: row
            .try_get("", "parent_task_id")
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

/// Returns `true` when `key` is `Some` but fails to parse as a fractional-index
/// string. A `None` anchor (omitted) is always valid; a `Some` anchor that is
/// not a well-formed hex-encoded fractional index is a phantom key and must be
/// rejected before it silently collapses to `None` inside `try_between`.
fn anchor_is_invalid(key: Option<&str>) -> bool {
    match key {
        None => false,
        Some(s) => fractional_index::FractionalIndex::from_string(s).is_err(),
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

/// Classifies a `DbErr` from a constrained INSERT by Postgres SQLSTATE code.
///
/// 23505 (unique_violation): the row already exists — callers surface this as
/// `Forbidden` (which routes to 409 Conflict at the HTTP layer).
///
/// 23503 (foreign_key_violation): a referenced principal (user or api_key) does
/// not exist in this workspace — callers surface this as `NotFound` (→ 404).
///
/// Anything else falls through to `Internal` (→ 500) via the normal `db_err` path.
fn classify_insert_err(e: sea_orm::DbErr, principal_id: uuid::Uuid) -> DomainError {
    use sea_orm::SqlErr;

    match e.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => DomainError::Forbidden {
            message: "assignee already added to this task".into(),
        },
        Some(SqlErr::ForeignKeyConstraintViolation(_)) => DomainError::NotFound {
            entity: "principal",
            id: principal_id,
        },
        _ => db_err(e),
    }
}

/// Classifies a `DbErr` from a reference INSERT by Postgres SQLSTATE code.
///
/// 23505 (unique_violation): an identical reference already exists — callers
/// surface this as `Forbidden` (→ 409 Conflict).
///
/// 23503 (foreign_key_violation): the referenced target task or document does
/// not exist in this workspace — callers surface this as `NotFound` (→ 404).
///
/// Anything else falls through to `Internal` (→ 500) via the normal `db_err` path.
/// Resolves the reference's target id for error reporting.
///
/// `validate_reference` guarantees exactly one of the two targets is set before
/// the insert; the nil fallback is therefore unreachable and only keeps the
/// helper total.
fn reference_target_id(new: &NewTaskReference) -> uuid::Uuid {
    new.target_task_id
        .map(|id| id.0)
        .or_else(|| new.target_document_id.map(|id| id.0))
        .unwrap_or(uuid::Uuid::nil())
}

fn classify_reference_insert_err(e: sea_orm::DbErr, target_id: uuid::Uuid) -> DomainError {
    use sea_orm::SqlErr;

    match e.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => DomainError::Forbidden {
            message: "reference already exists".into(),
        },
        Some(SqlErr::ForeignKeyConstraintViolation(_)) => DomainError::NotFound {
            entity: "reference target",
            id: target_id,
        },
        _ => db_err(e),
    }
}
