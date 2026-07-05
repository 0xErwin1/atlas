use crate::{
    DomainError, WorkspaceCtx,
    entities::boards_tasks::{
        ActivityKind, Board, BoardColumn, ColumnPatch, NewBoard, NewTask, NewTaskActivity,
        NewTaskAssignee, NewTaskChecklistItem, NewTaskReference, PositionBetween, Task,
        TaskActivity, TaskAssignee, TaskChecklistItem, TaskChecklistItemPatch, TaskPatch,
        TaskReference,
    },
    entities::task_views::{ActorTypeFilter, TaskViewFilters},
    ids::{BoardId, ChecklistItemId, ColumnId, ProjectId, TaskActivityId, TaskId, TaskReferenceId},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};

/// Resolved access scope for workspace activity queries.
///
/// Computed handler-side by calling the real `permissions::resolve()` per project
/// over a single grant load. The repo is dumb: it filters by the id sets + admin
/// flag. The authz decision lives entirely in the handler, made by the real
/// resolve(). This struct must NOT be relaxed to a plain workspace filter — it is
/// the privacy guard that prevents leaking activity from projects/boards the caller
/// cannot see.
#[derive(Debug, Clone)]
pub struct WorkspaceActivityScope {
    /// When true the repo returns all workspace activity, bypassing id-set filters.
    /// Set for Owner/Admin members and break-glass (root/system_admin) callers.
    pub is_admin: bool,
    /// Projects whose activity the caller may see, computed by resolve() per project.
    pub project_ids: Vec<ProjectId>,
    /// Boards whose task activity the caller may see via a direct board grant in a
    /// private project (board-only grants reach tasks even without a project grant).
    pub board_ids: Vec<BoardId>,
}

/// Optional filters for the workspace activity feed.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceActivityFilters {
    pub actor_type: Option<ActorTypeFilter>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

/// A workspace activity row: one activity entry plus the task's readable id
/// (obtained via the JOIN tasks → task_activity in the feed query).
#[derive(Debug, Clone)]
pub struct WorkspaceActivityRow {
    pub activity: TaskActivity,
    pub task_readable_id: String,
}

/// Opaque domain cursor for workspace-scoped task listing.
///
/// Carries the sort-column value and the task id as a tiebreak so the SQL
/// keyset predicate `(sort_col, id) < ($val, $id)` can produce a stable,
/// non-overlapping page sequence regardless of which column the user sorted by.
///
/// This is a domain type: it does NOT depend on the api-layer `SearchCursor`.
/// The route handler encodes/decodes the wire cursor and passes the decoded
/// domain cursor to the port method.
#[derive(Debug, Clone)]
pub struct TaskListCursor {
    /// Serialised sort-column value (epoch micros for temporal sorts, the
    /// string representation for text/priority sorts).
    pub sort_value: serde_json::Value,
    /// UUID tiebreak — the id of the last item on the previous page.
    pub id: TaskId,
}

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
        color: Option<String>,
        position: PositionBetween,
    ) -> Result<BoardColumn, DomainError>;

    async fn list_columns(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
    ) -> Result<Vec<BoardColumn>, DomainError>;

    /// Reorders a column. `board_id` is the board authorized by the caller; the
    /// column must belong to it (intra-workspace IDOR guard), and the new key is
    /// derived from that board's columns rather than the looked-up row's board.
    async fn move_column(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<(), DomainError>;

    async fn patch_board(
        &self,
        ctx: &WorkspaceCtx,
        id: BoardId,
        name: String,
    ) -> Result<Board, DomainError>;

    /// Patches a column's name and/or color. `board_id` is the authorized board;
    /// a column from a different board in the same workspace resolves to `NotFound`.
    ///
    /// `patch.name`: `None` = leave name unchanged; `Some(v)` = rename.
    /// `patch.color`: `None` = leave color unchanged; `Some(None)` = clear; `Some(Some(v))` = set.
    async fn patch_column(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
        id: ColumnId,
        patch: ColumnPatch,
    ) -> Result<BoardColumn, DomainError>;

    async fn soft_delete_board(&self, ctx: &WorkspaceCtx, id: BoardId) -> Result<(), DomainError>;

    /// Soft-deletes a column scoped to the authorized `board_id`; a mismatch
    /// resolves to `NotFound`.
    async fn soft_delete_column(
        &self,
        ctx: &WorkspaceCtx,
        board_id: BoardId,
        id: ColumnId,
    ) -> Result<(), DomainError>;
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

    /// Lists the sub-tasks (children) of `parent_task_id`, ordered by position.
    /// Sub-tasks are full tasks excluded from the board listings.
    async fn list_children(
        &self,
        ctx: &WorkspaceCtx,
        parent_task_id: TaskId,
    ) -> Result<Vec<Task>, DomainError>;

    /// Counts the direct sub-tasks of each given parent in a single query.
    ///
    /// Mirrors `list_children`'s predicates (workspace-scoped, `deleted_at IS
    /// NULL`) so the count matches what a sub-task listing would return. Parents
    /// with no children are omitted from the result; callers default those to 0.
    async fn count_children_for_parents(
        &self,
        ctx: &WorkspaceCtx,
        parent_task_ids: &[TaskId],
    ) -> Result<Vec<(TaskId, i64)>, DomainError>;

    /// Clears a task's parent so it becomes a top-level board task again
    /// ("promote"). Returns the updated task.
    async fn detach(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<Task, DomainError>;

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

    /// Lists top-level workspace tasks matching `filters`, ordered by the sort
    /// specified in the filters (default: `UpdatedDesc`).
    ///
    /// Callers should pass `limit + 1` and check `len() > limit` to derive
    /// `has_more`; this method returns exactly what the DB returns (no
    /// truncation), so the overfetch idiom is correct here.
    ///
    /// The keyset cursor `after` positions the result window after the last
    /// seen item. Pass `None` for the first page.
    ///
    /// Always-applied predicates:
    /// - `workspace_id = ctx.workspace_id`
    /// - `parent_task_id IS NULL` (top-level only)
    /// - `deleted_at IS NULL`
    async fn list_by_workspace_filtered(
        &self,
        ctx: &WorkspaceCtx,
        filters: &TaskViewFilters,
        after: Option<TaskListCursor>,
        limit: u64,
    ) -> Result<Vec<Task>, DomainError>;
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

    /// Batch variant of `list_for_task`: all assignees for the given tasks in a
    /// single query, so a board listing avoids one query per card.
    async fn list_for_tasks(
        &self,
        ctx: &WorkspaceCtx,
        task_ids: &[TaskId],
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

    /// Lists activity for the whole workspace, access-filtered by the resolved
    /// scope.
    ///
    /// The accessible project/board id sets in `scope` are computed handler-side
    /// by the real `permissions::resolve()` (one call per project over a single
    /// grant load). The repo applies them as `t.project_id = ANY($projects) OR
    /// t.board_id = ANY($boards)` plus the admin bypass. This must NOT be relaxed
    /// to a plain workspace filter — that would leak private-board activity to every
    /// member.
    ///
    /// Keyset cursor: `after` is the exclusive upper bound `(created_at, id)` for
    /// descending order. Returns up to `limit` rows; caller overfetches by 1 to
    /// determine `has_more`.
    async fn list_for_workspace(
        &self,
        ctx: &WorkspaceCtx,
        scope: WorkspaceActivityScope,
        filters: WorkspaceActivityFilters,
        after: Option<(DateTime<Utc>, TaskActivityId)>,
        limit: u64,
    ) -> Result<Vec<WorkspaceActivityRow>, DomainError>;
}
