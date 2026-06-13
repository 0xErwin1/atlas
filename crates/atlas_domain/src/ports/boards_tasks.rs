use crate::{
    DomainError, WorkspaceCtx,
    entities::boards_tasks::{
        ActivityKind, Board, BoardColumn, NewBoard, NewTask, NewTaskActivity, NewTaskAssignee,
        NewTaskChecklistItem, NewTaskReference, PositionBetween, Task, TaskActivity,
        TaskAssignee, TaskChecklistItem, TaskChecklistItemPatch, TaskPatch, TaskReference,
    },
    ids::{BoardId, ChecklistItemId, ColumnId, ProjectId, TaskId, TaskReferenceId},
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

    /// Lists boards belonging to `project_id` within the workspace.
    ///
    /// Fixes B4: the previous signature was workspace-wide with no project scope.
    async fn list_boards(
        &self,
        ctx: &WorkspaceCtx,
        project_id: ProjectId,
    ) -> Result<Vec<Board>, DomainError>;

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

    async fn find_by_readable_id(
        &self,
        ctx: &WorkspaceCtx,
        readable_id: &str,
    ) -> Result<Option<Task>, DomainError>;

    async fn list_by_column(
        &self,
        ctx: &WorkspaceCtx,
        column_id: ColumnId,
    ) -> Result<Vec<Task>, DomainError>;

    async fn list_by_board(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
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

    async fn list_inbound(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskReference>, DomainError>;

    async fn delete(&self, ctx: &WorkspaceCtx, id: TaskReferenceId) -> Result<(), DomainError>;
}

#[async_trait]
pub trait TaskAssigneeRepo: Send + Sync {
    async fn add(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskAssignee,
    ) -> Result<TaskAssignee, DomainError>;

    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskAssignee>, DomainError>;

    async fn remove(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        assignee: crate::entities::boards_tasks::AssigneeRef,
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait TaskChecklistRepo: Send + Sync {
    async fn add_item(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskChecklistItem,
    ) -> Result<TaskChecklistItem, DomainError>;

    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Vec<TaskChecklistItem>, DomainError>;

    async fn patch_item(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
        patch: TaskChecklistItemPatch,
    ) -> Result<TaskChecklistItem, DomainError>;

    async fn soft_delete_item(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
    ) -> Result<(), DomainError>;

    async fn mark_promoted(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
        promoted_task_id: TaskId,
    ) -> Result<TaskChecklistItem, DomainError>;
}

#[async_trait]
pub trait TaskActivityRepo: Send + Sync {
    /// Appends one activity entry inside an existing transaction.
    ///
    /// This is the ONLY write path for task_activity; there is no direct
    /// route to create activity entries.
    async fn append(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskActivity,
    ) -> Result<TaskActivity, DomainError>;

    /// Lists activity for a task, newest-first, with cursor-based pagination.
    ///
    /// `after_id` is the exclusive lower bound (id of the last seen entry).
    async fn list_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        after_id: Option<crate::ids::TaskActivityId>,
        limit: u64,
    ) -> Result<Vec<TaskActivity>, DomainError>;

    /// Returns the `kind` of the last activity entry for `task_id`, used to
    /// guard idempotency (e.g. detecting double-assign or double-promote).
    async fn last_kind_for_task(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
    ) -> Result<Option<ActivityKind>, DomainError>;
}
