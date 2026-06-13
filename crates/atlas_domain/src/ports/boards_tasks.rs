use crate::{
    DomainError, WorkspaceCtx,
    entities::boards_tasks::{
        Board, BoardColumn, NewBoard, NewTask, NewTaskReference, PositionBetween, Task, TaskPatch,
        TaskReference,
    },
    ids::{BoardId, ColumnId, TaskId, TaskReferenceId},
};
use async_trait::async_trait;

#[async_trait]
pub trait BoardRepo: Send + Sync {
    async fn create_board(&self, ctx: &WorkspaceCtx, new: NewBoard) -> Result<Board, DomainError>;

    async fn find_board(
        &self,
        ctx: &WorkspaceCtx,
        id: BoardId,
    ) -> Result<Option<Board>, DomainError>;

    async fn list_boards(&self, ctx: &WorkspaceCtx) -> Result<Vec<Board>, DomainError>;

    async fn add_column(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
        name: String,
        position: PositionBetween,
    ) -> Result<BoardColumn, DomainError>;

    async fn list_columns(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
    ) -> Result<Vec<BoardColumn>, DomainError>;

    async fn move_column(
        &self,
        ctx: &WorkspaceCtx,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<(), DomainError>;

    async fn soft_delete_board(&self, ctx: &WorkspaceCtx, id: BoardId) -> Result<(), DomainError>;

    async fn soft_delete_column(&self, ctx: &WorkspaceCtx, id: ColumnId)
    -> Result<(), DomainError>;
}

#[async_trait]
pub trait TaskRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewTask) -> Result<Task, DomainError>;

    async fn find(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<Option<Task>, DomainError>;

    async fn list_by_column(
        &self,
        ctx: &WorkspaceCtx,
        column_id: ColumnId,
    ) -> Result<Vec<Task>, DomainError>;

    async fn patch(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskId,
        patch: TaskPatch,
    ) -> Result<Task, DomainError>;

    async fn move_to(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskId,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<Task, DomainError>;

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<(), DomainError>;
}

#[async_trait]
pub trait TaskReferenceRepo: Send + Sync {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskReference,
    ) -> Result<TaskReference, DomainError>;

    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskReference>, DomainError>;

    async fn delete(&self, ctx: &WorkspaceCtx, id: TaskReferenceId) -> Result<(), DomainError>;
}
