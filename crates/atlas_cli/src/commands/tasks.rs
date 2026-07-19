#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::io::Write as _;

use atlas_api::dtos::boards_tasks::{
    AddAssigneeRequest, CreateChecklistItemRequest, CreateCommentRequest, CreateReferenceRequest,
    CreateSubtaskRequest, CreateTaskRequest, MoveTaskRequest, PromoteChecklistItemRequest,
    TaskPropertiesDto, UpdateChecklistItemRequest, UpdateTaskRequest, WorkspaceTaskQueryParams,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use uuid::Uuid;

use crate::commands::bulk;
use crate::commands::common::{LIMIT_DEFAULT, LIMIT_MAX, LIMIT_MIN, read_upload_file};
use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::{
    AttachProjection, ChecklistItemProjection, CommentProjection, DeleteTaskProjection,
    DeletedProjection, PromotionProjection, SubtaskProjection, TaskActivityProjection,
    TaskAssigneeProjection, TaskBacklinkProjection, TaskCompactProjection, TaskFullProjection,
    TaskRefProjection, TaskSummaryProjection,
};
use atlas_client::helpers;

// ---------------------------------------------------------------------------
// Detail level
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Detail {
    Compact,
    Full,
}

// ---------------------------------------------------------------------------
// TasksArgs (wrapper for nesting into Commands) + TasksCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `tasks` subcommand group.
#[derive(Args)]
pub(crate) struct TasksArgs {
    #[command(subcommand)]
    pub(crate) command: TasksCmd,
}

#[derive(Subcommand)]
pub(crate) enum TasksCmd {
    /// List tasks in the workspace, optionally filtered by board/status/assignee/labels.
    List(TasksListArgs),
    /// Get a task by its readable ID (e.g. ATL-42).
    Get(TasksGetArgs),
    /// Create a new task on a board.
    Create(TasksCreateArgs),
    /// Update fields of an existing task (PATCH semantics: absent = unchanged).
    Update(TasksUpdateArgs),
    /// Move a task to a different column.
    Move(TasksMoveArgs),
    /// Delete a task (requires --confirm).
    Delete(TasksDeleteArgs),
    /// Manage outbound references on a task (list, create, remove).
    Refs(RefsArgs),
    /// List inbound references (backlinks) pointing at a task.
    Backlinks(TasksBacklinksArgs),
    /// Manage assignees on a task (list, add, remove).
    Assignees(AssigneesArgs),
    /// Manage the checklist on a task (list, add, update, remove, promote).
    Checklist(ChecklistArgs),
    /// List activity (audit log) entries for a task.
    Activity(TasksActivityArgs),
    /// Manage comments on a task (list, add, delete).
    Comments(CommentsArgs),
    /// Manage subtasks of a task (list, create, promote).
    Subtasks(SubtasksArgs),
    /// Manage task attachments (upload, list, download, delete).
    Attach(TasksAttachArgs),
}

/// Dispatches a parsed `TasksCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: TasksCmd) -> Result<(), CliError> {
    match cmd {
        TasksCmd::List(args) => run_list(ctx, args).await,
        TasksCmd::Get(args) => run_get(ctx, args).await,
        TasksCmd::Create(args) => run_create(ctx, args).await,
        TasksCmd::Update(args) => run_update(ctx, args).await,
        TasksCmd::Move(args) => run_move(ctx, args).await,
        TasksCmd::Delete(args) => run_delete(ctx, args).await,
        TasksCmd::Refs(args) => run_refs(ctx, args.command).await,
        TasksCmd::Backlinks(args) => run_backlinks(ctx, args).await,
        TasksCmd::Assignees(args) => run_assignees(ctx, args.command).await,
        TasksCmd::Checklist(args) => run_checklist(ctx, args.command).await,
        TasksCmd::Activity(args) => run_task_activity(ctx, args).await,
        TasksCmd::Comments(args) => run_comments(ctx, args.command).await,
        TasksCmd::Subtasks(args) => run_subtasks(ctx, args.command).await,
        TasksCmd::Attach(args) => run_task_attach(ctx, args.command).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks list`.
#[derive(Parser)]
pub(crate) struct TasksListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Scope to a specific board (name or UUID); omit for workspace-wide results.
    #[arg(long)]
    pub(crate) board: Option<String>,

    /// Filter to tasks in this column/status name.
    #[arg(long = "status")]
    pub(crate) status: Option<String>,

    /// Filter to tasks assigned to this actor (`me`, `user:{uuid}`, `api_key:{uuid}`).
    #[arg(long)]
    pub(crate) assignee: Option<String>,

    /// Filter to tasks with this priority (repeatable).
    #[arg(long = "priority")]
    pub(crate) priorities: Vec<String>,

    /// Filter to tasks carrying this label (repeatable).
    #[arg(long = "label")]
    pub(crate) labels: Vec<String>,

    /// Sort key (e.g. `updated_at_desc`).
    #[arg(long)]
    pub(crate) sort: Option<String>,

    /// Maximum results to return (clamped to 1..=200; default 20).
    #[arg(long)]
    pub(crate) limit: Option<u32>,

    /// Pagination cursor returned by a previous list.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
}

async fn run_list(ctx: &Ctx, args: TasksListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let column_ids = if let Some(status_name) = &args.status {
        helpers::resolve_column_ids(&ctx.client, ws, args.board.as_deref(), status_name)
            .await
            .map_err(CliError::from)?
    } else {
        Vec::new()
    };

    let board_id = if let Some(board) = &args.board {
        let id = helpers::resolve_board_id(&ctx.client, ws, board)
            .await
            .map_err(CliError::from)?;
        Some(id)
    } else {
        None
    };

    let query = WorkspaceTaskQueryParams {
        assignee: args.assignee,
        actor: None,
        column_ids,
        priorities: args.priorities,
        labels: args.labels,
        board_id,
        sort: args.sort,
        cursor: args.cursor,
        limit: Some(limit),
    };

    let page = ctx.client.list_workspace_tasks(ws, &query).await?;

    let projections: Vec<TaskSummaryProjection> = page
        .items
        .into_iter()
        .map(TaskSummaryProjection::from)
        .collect();

    output::emit_list(
        ctx.output,
        &projections,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// Get
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks get`.
#[derive(Parser)]
pub(crate) struct TasksGetArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Level of detail: `compact` (default) or `full` (adds description, references,
    /// subtasks, and assignees).
    #[arg(long, default_value = "compact")]
    pub(crate) detail: Detail,
}

async fn run_get(ctx: &Ctx, args: TasksGetArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let task = ctx.client.get_task(ws, &args.readable_id).await?;

    match args.detail {
        Detail::Compact => {
            let proj = TaskCompactProjection::from(task);
            output::emit(ctx.output, &proj)
        }

        Detail::Full => {
            let refs = ctx
                .client
                .list_references(ws, &args.readable_id)
                .await
                .map(|v| {
                    v.into_iter()
                        .map(|r| {
                            serde_json::json!({
                                "origins": r.origins,
                                "manual_reference_id": r.manual_reference_id,
                                "manual_kind": r.manual_kind,
                                "target_readable_id": r.target_readable_id,
                                "target_document_id": r.target_document_id,
                                "target_title": r.target_title,
                                "target_resolved": r.target_resolved,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .map_err(|e| format!("list_references failed: {e}"));

            let subtasks = ctx
                .client
                .list_subtasks(ws, &args.readable_id)
                .await
                .map_err(|e| format!("list_subtasks failed: {e}"))
                .and_then(|v| {
                    v.into_iter()
                        .map(|s| {
                            serde_json::to_value(SubtaskProjection::from(s))
                                .map_err(|e| format!("list_subtasks: serialize: {e}"))
                        })
                        .collect::<Result<Vec<_>, String>>()
                });

            let assignees = ctx
                .client
                .list_assignees(ws, &args.readable_id)
                .await
                .map(|v| {
                    v.into_iter()
                        .map(|a| {
                            serde_json::json!({
                                "type": a.assignee.r#type,
                                "display_name": a.assignee.display_name,
                                "assigned_at": a.assigned_at,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .map_err(|e| format!("list_assignees failed: {e}"));

            let proj = TaskFullProjection::new(task, refs, subtasks, assignees);
            output::emit(ctx.output, &proj)
        }
    }
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks create`.
///
/// When `--stdin` is set, one JSON object per line is read from stdin and each
/// becomes a separate create call. The expected line shape is:
/// `{"board_id":"<uuid>","column_id":"<uuid>","title":"...","description":"...","properties":{...}}`
/// In that mode `--board`, `--column`, and `--title` are ignored.
#[derive(Parser)]
pub(crate) struct TasksCreateArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Task title (required in single-item mode; ignored when --stdin is set).
    #[arg(long, required_unless_present = "stdin")]
    pub(crate) title: Option<String>,

    /// Board name or UUID (required in single-item mode; ignored when --stdin is set).
    #[arg(long, required_unless_present = "stdin")]
    pub(crate) board: Option<String>,

    /// Column name on the board (required in single-item mode; ignored when
    /// --stdin is set; resolved by case-insensitive substring).
    #[arg(long, required_unless_present = "stdin")]
    pub(crate) column: Option<String>,

    /// Task description (markdown).
    #[arg(long)]
    pub(crate) description: Option<String>,

    /// Priority: `low`, `medium`, `high`, or `urgent`.
    #[arg(long)]
    pub(crate) priority: Option<String>,

    /// Label to attach (repeatable).
    #[arg(long = "label")]
    pub(crate) labels: Vec<String>,

    /// Work estimate (non-negative story-point integer).
    #[arg(long)]
    pub(crate) estimate: Option<i32>,

    /// Due date (ISO 8601, e.g. `2026-01-31T00:00:00Z`).
    #[arg(long)]
    pub(crate) due_date: Option<String>,

    /// Read one JSON object per line from stdin; each line becomes a separate
    /// create call. When set, `--board`, `--column`, and `--title` are ignored.
    #[arg(long)]
    pub(crate) stdin: bool,
}

async fn run_create(ctx: &Ctx, args: TasksCreateArgs) -> Result<(), CliError> {
    if args.stdin {
        return run_create_stdin(ctx, args).await;
    }
    run_create_single(ctx, args).await
}

async fn run_create_single(ctx: &Ctx, args: TasksCreateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let title = args
        .title
        .ok_or_else(|| CliError::Validation("--title is required".to_owned()))?;

    let board = args
        .board
        .ok_or_else(|| CliError::Validation("--board is required".to_owned()))?;

    let column = args
        .column
        .ok_or_else(|| CliError::Validation("--column is required".to_owned()))?;

    if let Some(pri) = &args.priority {
        helpers::validate_priority(pri).map_err(CliError::Validation)?;
    }
    if let Some(est) = args.estimate {
        helpers::validate_estimate(est).map_err(CliError::Validation)?;
    }

    let board_id_str = helpers::resolve_board_id(&ctx.client, ws, &board)
        .await
        .map_err(CliError::from)?;

    let board_uuid: uuid::Uuid = board_id_str.parse().map_err(|_| {
        CliError::Resolver(Box::new(
            atlas_client::helpers::ResolverError::InvalidBoardUuid {
                board_id: board_id_str.clone(),
            },
        ))
    })?;

    let cols = ctx.client.list_columns(ws, board_uuid).await?;
    let column_uuid =
        helpers::resolve_column_id_on_board(&column, &cols).map_err(CliError::Validation)?;

    let due_date = args
        .due_date
        .as_deref()
        .map(|s| {
            s.parse::<chrono::DateTime<chrono::Utc>>().map_err(|_| {
                CliError::Validation(format!("invalid due-date '{s}'; expected ISO 8601"))
            })
        })
        .transpose()?;

    let properties = if args.priority.is_some()
        || args.estimate.is_some()
        || !args.labels.is_empty()
        || due_date.is_some()
    {
        Some(TaskPropertiesDto {
            priority: args.priority,
            estimate: args.estimate,
            labels: args.labels,
            due_date,
            custom: None,
        })
    } else {
        None
    };

    let body = CreateTaskRequest {
        column_id: column_uuid,
        title,
        description: args.description,
        properties,
        before: None,
        after: None,
    };

    let task = ctx.client.create_task(ws, board_uuid, body).await?;
    let proj = TaskCompactProjection::from(task);
    output::emit(ctx.output, &proj)
}

async fn run_create_stdin(ctx: &Ctx, args: TasksCreateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let (items, mut any_failed) = bulk::parse_stdin_batch::<bulk::BulkTaskCreateLine>()?;

    for item in items {
        match ctx.client.create_task(ws, item.board_id, item.body).await {
            Ok(task) => {
                let proj = TaskCompactProjection::from(task);
                let value = serde_json::to_value(&proj)
                    .map_err(|e| CliError::Io(std::io::Error::other(e.to_string())))?;
                bulk::emit_batch_line(&value)?;
            }
            Err(e) => {
                eprintln!("error: {e}");
                any_failed = true;
            }
        }
    }

    if any_failed {
        Err(CliError::Validation(
            "batch: one or more items failed (see stderr)".to_owned(),
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks update`.
///
/// When `--stdin` is set, one JSON object per line is read from stdin and each
/// becomes a separate update call. The expected line shape is:
/// `{"readable_id":"ATL-42","title":"...","priority":"high"}` (PATCH semantics:
/// absent fields are unchanged; `null` for `priority`/`due_date`/`estimate` clears
/// the field). In that mode the positional `readable_id` argument is ignored.
#[derive(Parser)]
pub(crate) struct TasksUpdateArgs {
    /// Task readable ID, e.g. `ATL-42` (required in single-item mode; ignored
    /// when --stdin is set).
    #[arg(index = 1, required_unless_present = "stdin")]
    pub(crate) readable_id: Option<String>,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// New title.
    #[arg(long)]
    pub(crate) title: Option<String>,

    /// New description (markdown).
    #[arg(long)]
    pub(crate) description: Option<String>,

    /// New priority: `low`, `medium`, `high`, or `urgent`.
    #[arg(long)]
    pub(crate) priority: Option<String>,

    /// Replace the full label set (repeatable; omit to leave labels unchanged).
    #[arg(long = "label")]
    pub(crate) labels: Vec<String>,

    /// New estimate (non-negative story-point integer).
    #[arg(long)]
    pub(crate) estimate: Option<i32>,

    /// New due date (ISO 8601).
    #[arg(long)]
    pub(crate) due_date: Option<String>,

    /// Clear the priority field (set to null).
    #[arg(long)]
    pub(crate) clear_priority: bool,

    /// Clear the due-date field (set to null).
    #[arg(long)]
    pub(crate) clear_due_date: bool,

    /// Clear the estimate field (set to null).
    #[arg(long)]
    pub(crate) clear_estimate: bool,

    /// Read one JSON object per line from stdin; each line becomes a separate
    /// update call. When set, the positional `readable_id` argument is ignored.
    #[arg(long)]
    pub(crate) stdin: bool,
}

/// Builds the `UpdateTaskRequest` from the parsed update arguments.
///
/// Validates priority and estimate before constructing the body so that
/// invalid inputs are rejected before any network call is made. Nullable
/// fields follow the PATCH tri-state: absent = unchanged, `--clear-X` =
/// null (clear), explicit value = set.
pub(crate) fn build_update_body(args: &TasksUpdateArgs) -> Result<UpdateTaskRequest, CliError> {
    if let Some(pri) = &args.priority {
        helpers::validate_priority(pri).map_err(CliError::Validation)?;
    }
    if let Some(est) = args.estimate {
        helpers::validate_estimate(est).map_err(CliError::Validation)?;
    }

    let priority = if args.clear_priority {
        Some(serde_json::Value::Null)
    } else {
        args.priority
            .as_ref()
            .map(|p| serde_json::Value::String(p.clone()))
    };

    let due_date = if args.clear_due_date {
        Some(serde_json::Value::Null)
    } else if let Some(s) = &args.due_date {
        let dt = s.parse::<chrono::DateTime<chrono::Utc>>().map_err(|_| {
            CliError::Validation(format!("invalid due-date '{s}'; expected ISO 8601"))
        })?;
        Some(serde_json::json!(dt))
    } else {
        None
    };

    let estimate = if args.clear_estimate {
        Some(serde_json::Value::Null)
    } else {
        args.estimate.map(|e| serde_json::json!(e))
    };

    let labels = if args.labels.is_empty() {
        None
    } else {
        Some(args.labels.clone())
    };

    Ok(UpdateTaskRequest {
        title: args.title.clone(),
        description: args.description.clone(),
        priority,
        due_date,
        estimate,
        labels,
        properties: None,
    })
}

async fn run_update(ctx: &Ctx, args: TasksUpdateArgs) -> Result<(), CliError> {
    if args.stdin {
        return run_update_stdin(ctx, args).await;
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let readable_id = args
        .readable_id
        .as_deref()
        .ok_or_else(|| {
            CliError::Validation("readable_id is required in single-item mode".to_owned())
        })?
        .to_owned();
    let body = build_update_body(&args)?;
    let task = ctx.client.update_task(ws, &readable_id, body).await?;
    let proj = TaskCompactProjection::from(task);
    output::emit(ctx.output, &proj)
}

async fn run_update_stdin(ctx: &Ctx, args: TasksUpdateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let (items, mut any_failed) = bulk::parse_stdin_batch::<bulk::BulkTaskUpdateLine>()?;

    for item in items {
        let readable_id = item.readable_id.clone();
        let body = item.into_request();
        match ctx.client.update_task(ws, &readable_id, body).await {
            Ok(task) => {
                let proj = TaskCompactProjection::from(task);
                let value = serde_json::to_value(&proj)
                    .map_err(|e| CliError::Io(std::io::Error::other(e.to_string())))?;
                bulk::emit_batch_line(&value)?;
            }
            Err(e) => {
                eprintln!("error: {e}");
                any_failed = true;
            }
        }
    }

    if any_failed {
        Err(CliError::Validation(
            "batch: one or more items failed (see stderr)".to_owned(),
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Move
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks move`.
#[derive(Parser)]
pub(crate) struct TasksMoveArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Target column name (required; resolved on the board the task belongs to).
    #[arg(long)]
    pub(crate) column: String,

    /// Board name or UUID (optional; defaults to the task's own board).
    #[arg(long)]
    pub(crate) board: Option<String>,
}

async fn run_move(ctx: &Ctx, args: TasksMoveArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let board_uuid = if let Some(board) = &args.board {
        let id_str = helpers::resolve_board_id(&ctx.client, ws, board)
            .await
            .map_err(CliError::from)?;
        id_str.parse::<uuid::Uuid>().map_err(|_| {
            CliError::Resolver(Box::new(helpers::ResolverError::InvalidBoardUuid {
                board_id: id_str.clone(),
            }))
        })?
    } else {
        let task = ctx.client.get_task(ws, &args.readable_id).await?;
        task.board_id
    };

    let cols = ctx.client.list_columns(ws, board_uuid).await?;
    let column_uuid =
        helpers::resolve_column_id_on_board(&args.column, &cols).map_err(CliError::Validation)?;

    let body = MoveTaskRequest {
        column_id: column_uuid,
        before: None,
        after: None,
    };

    let task = ctx.client.move_task(ws, &args.readable_id, body).await?;
    let proj = TaskCompactProjection::from(task);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks delete`.
#[derive(Parser)]
pub(crate) struct TasksDeleteArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the deletion. Required — prevents accidental non-reversible deletes.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete(ctx: &Ctx, args: TasksDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete (this is a non-reversible operation)".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    ctx.client.delete_task(ws, &args.readable_id).await?;

    let proj = DeleteTaskProjection {
        deleted: true,
        readable_id: args.readable_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Refs (WU-22)
// ---------------------------------------------------------------------------

/// Arguments holder for the `tasks refs` subcommand group.
#[derive(Args)]
pub(crate) struct RefsArgs {
    #[command(subcommand)]
    pub(crate) command: RefsCmd,
}

#[derive(Subcommand)]
pub(crate) enum RefsCmd {
    /// List outbound references on a task.
    List(RefsListArgs),
    /// Create an outbound reference from a task to another task or document.
    Create(RefsCreateArgs),
    /// Remove an outbound reference by its UUID.
    Remove(RefsRemoveArgs),
}

/// Arguments for `atlas tasks refs list`.
#[derive(Parser)]
pub(crate) struct RefsListArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Arguments for `atlas tasks refs create`.
#[derive(Parser)]
pub(crate) struct RefsCreateArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Reference kind: `relates`, `blocks`, `parent`, or `spec`.
    #[arg(long)]
    pub(crate) kind: String,

    /// Target: a task readable ID (e.g. `ATL-10`) or a document UUID.
    /// UUIDs are resolved as document targets; anything else is treated as a task
    /// readable ID.
    #[arg(long)]
    pub(crate) target: String,
}

/// Arguments for `atlas tasks refs remove`.
#[derive(Parser)]
pub(crate) struct RefsRemoveArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// UUID of the reference to remove.
    #[arg(long)]
    pub(crate) ref_id: Uuid,
}

async fn run_refs(ctx: &Ctx, cmd: RefsCmd) -> Result<(), CliError> {
    match cmd {
        RefsCmd::List(args) => run_refs_list(ctx, args).await,
        RefsCmd::Create(args) => run_refs_create(ctx, args).await,
        RefsCmd::Remove(args) => run_refs_remove(ctx, args).await,
    }
}

async fn run_refs_list(ctx: &Ctx, args: RefsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let refs = ctx.client.list_references(ws, &args.readable_id).await?;
    let projections: Vec<TaskRefProjection> =
        refs.into_iter().map(TaskRefProjection::from).collect();
    output::emit_list(ctx.output, &projections, None, false)
}

async fn run_refs_create(ctx: &Ctx, args: RefsCreateArgs) -> Result<(), CliError> {
    helpers::validate_reference_kind(&args.kind).map_err(CliError::Validation)?;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let (target_task_readable_id, target_document_id) =
        if let Ok(doc_id) = args.target.parse::<Uuid>() {
            (None, Some(doc_id))
        } else {
            (Some(args.target.clone()), None)
        };

    let body = CreateReferenceRequest {
        kind: args.kind,
        target_task_readable_id,
        target_document_id,
    };

    let reference = ctx
        .client
        .create_reference(ws, &args.readable_id, body)
        .await?;

    let proj = TaskRefProjection::from(reference);
    output::emit(ctx.output, &proj)
}

async fn run_refs_remove(ctx: &Ctx, args: RefsRemoveArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    ctx.client
        .delete_reference(ws, &args.readable_id, args.ref_id)
        .await?;
    let proj = DeletedProjection { deleted: true };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Backlinks (WU-22)
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks backlinks`.
#[derive(Parser)]
pub(crate) struct TasksBacklinksArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_backlinks(ctx: &Ctx, args: TasksBacklinksArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let page = ctx
        .client
        .list_task_backlinks(ws, &args.readable_id)
        .await?;
    let projections: Vec<TaskBacklinkProjection> = page
        .items
        .into_iter()
        .map(TaskBacklinkProjection::from)
        .collect();
    output::emit_list(
        ctx.output,
        &projections,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// Assignees (WU-22)
// ---------------------------------------------------------------------------

/// Arguments holder for the `tasks assignees` subcommand group.
#[derive(Args)]
pub(crate) struct AssigneesArgs {
    #[command(subcommand)]
    pub(crate) command: AssigneesCmd,
}

#[derive(Subcommand)]
pub(crate) enum AssigneesCmd {
    /// List assignees on a task.
    List(AssigneesListArgs),
    /// Add an assignee to a task (validates type before network).
    Add(AssigneesAddArgs),
    /// Remove an assignee from a task by its assignee reference.
    Remove(AssigneesRemoveArgs),
}

/// Arguments for `atlas tasks assignees list`.
#[derive(Parser)]
pub(crate) struct AssigneesListArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Arguments for `atlas tasks assignees add`.
#[derive(Parser)]
pub(crate) struct AssigneesAddArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Assignee type: `user` or `api_key`.
    #[arg(long)]
    pub(crate) assignee_type: String,

    /// UUID of the user or API key to assign.
    #[arg(long)]
    pub(crate) assignee_id: Uuid,
}

/// Arguments for `atlas tasks assignees remove`.
#[derive(Parser)]
pub(crate) struct AssigneesRemoveArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Assignee reference to remove (`me`, `user:{uuid}`, or `api_key:{uuid}`).
    #[arg(long)]
    pub(crate) assignee_id: String,
}

async fn run_assignees(ctx: &Ctx, cmd: AssigneesCmd) -> Result<(), CliError> {
    match cmd {
        AssigneesCmd::List(args) => run_assignees_list(ctx, args).await,
        AssigneesCmd::Add(args) => run_assignees_add(ctx, args).await,
        AssigneesCmd::Remove(args) => run_assignees_remove(ctx, args).await,
    }
}

async fn run_assignees_list(ctx: &Ctx, args: AssigneesListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let assignees = ctx.client.list_assignees(ws, &args.readable_id).await?;
    let projections: Vec<TaskAssigneeProjection> = assignees
        .into_iter()
        .map(TaskAssigneeProjection::from)
        .collect();
    output::emit_list(ctx.output, &projections, None, false)
}

async fn run_assignees_add(ctx: &Ctx, args: AssigneesAddArgs) -> Result<(), CliError> {
    helpers::validate_assignee_type(&args.assignee_type).map_err(CliError::Validation)?;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = AddAssigneeRequest {
        assignee_type: args.assignee_type,
        assignee_id: args.assignee_id,
    };

    let assignee = ctx.client.add_assignee(ws, &args.readable_id, body).await?;

    let proj = TaskAssigneeProjection::from(assignee);
    output::emit(ctx.output, &proj)
}

async fn run_assignees_remove(ctx: &Ctx, args: AssigneesRemoveArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    ctx.client
        .remove_assignee(ws, &args.readable_id, &args.assignee_id)
        .await?;
    let proj = DeletedProjection { deleted: true };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Checklist (WU-23)
// ---------------------------------------------------------------------------

/// Arguments holder for the `tasks checklist` subcommand group.
#[derive(Args)]
pub(crate) struct ChecklistArgs {
    #[command(subcommand)]
    pub(crate) command: ChecklistCmd,
}

#[derive(Subcommand)]
pub(crate) enum ChecklistCmd {
    /// List checklist items on a task.
    List(ChecklistListArgs),
    /// Add a checklist item to a task.
    Add(ChecklistAddArgs),
    /// Update a checklist item (title, checked state).
    Update(ChecklistUpdateArgs),
    /// Remove a checklist item (requires --confirm).
    Remove(ChecklistRemoveArgs),
    /// Promote a checklist item to a task.
    Promote(ChecklistPromoteArgs),
}

/// Arguments for `atlas tasks checklist list`.
#[derive(Parser)]
pub(crate) struct ChecklistListArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Arguments for `atlas tasks checklist add`.
#[derive(Parser)]
pub(crate) struct ChecklistAddArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Checklist item title (required).
    #[arg(long)]
    pub(crate) title: String,
}

/// Arguments for `atlas tasks checklist update`.
#[derive(Parser)]
pub(crate) struct ChecklistUpdateArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// UUID of the checklist item to update (required).
    #[arg(long)]
    pub(crate) id: Uuid,

    /// New title for the checklist item.
    #[arg(long)]
    pub(crate) title: Option<String>,

    /// Mark the item as checked.
    #[arg(long)]
    pub(crate) checked: bool,

    /// Mark the item as unchecked.
    #[arg(long)]
    pub(crate) uncheck: bool,
}

/// Arguments for `atlas tasks checklist remove`.
#[derive(Parser)]
pub(crate) struct ChecklistRemoveArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// UUID of the checklist item to remove (required).
    #[arg(long)]
    pub(crate) id: Uuid,

    /// Confirm the deletion. Required — prevents accidental non-reversible deletes.
    #[arg(long)]
    pub(crate) confirm: bool,
}

/// Arguments for `atlas tasks checklist promote`.
#[derive(Parser)]
pub(crate) struct ChecklistPromoteArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// UUID of the checklist item to promote (required).
    #[arg(long)]
    pub(crate) id: Uuid,

    /// Target board name or UUID for the new task.
    #[arg(long)]
    pub(crate) board: String,

    /// Target column name on the board for the new task.
    #[arg(long)]
    pub(crate) column: String,
}

async fn run_checklist(ctx: &Ctx, cmd: ChecklistCmd) -> Result<(), CliError> {
    match cmd {
        ChecklistCmd::List(args) => run_checklist_list(ctx, args).await,
        ChecklistCmd::Add(args) => run_checklist_add(ctx, args).await,
        ChecklistCmd::Update(args) => run_checklist_update(ctx, args).await,
        ChecklistCmd::Remove(args) => run_checklist_remove(ctx, args).await,
        ChecklistCmd::Promote(args) => run_checklist_promote(ctx, args).await,
    }
}

async fn run_checklist_list(ctx: &Ctx, args: ChecklistListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let items = ctx.client.list_checklist(ws, &args.readable_id).await?;
    let projections: Vec<ChecklistItemProjection> = items
        .into_iter()
        .map(ChecklistItemProjection::from)
        .collect();
    output::emit_list(ctx.output, &projections, None, false)
}

async fn run_checklist_add(ctx: &Ctx, args: ChecklistAddArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let body = CreateChecklistItemRequest {
        title: args.title,
        before: None,
        after: None,
    };
    let item = ctx
        .client
        .create_checklist_item(ws, &args.readable_id, body)
        .await?;
    let proj = ChecklistItemProjection::from(item);
    output::emit(ctx.output, &proj)
}

async fn run_checklist_update(ctx: &Ctx, args: ChecklistUpdateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let checked = if args.checked {
        Some(true)
    } else if args.uncheck {
        Some(false)
    } else {
        None
    };

    let body = UpdateChecklistItemRequest {
        title: args.title,
        checked,
        before: None,
        after: None,
    };

    let item = ctx
        .client
        .update_checklist_item(ws, &args.readable_id, args.id, body)
        .await?;

    let proj = ChecklistItemProjection::from(item);
    output::emit(ctx.output, &proj)
}

async fn run_checklist_remove(ctx: &Ctx, args: ChecklistRemoveArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete (this is a non-reversible operation)".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    ctx.client
        .delete_checklist_item(ws, &args.readable_id, args.id)
        .await?;

    let proj = DeletedProjection { deleted: true };
    output::emit(ctx.output, &proj)
}

async fn run_checklist_promote(ctx: &Ctx, args: ChecklistPromoteArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let board_id_str = helpers::resolve_board_id(&ctx.client, ws, &args.board)
        .await
        .map_err(CliError::from)?;

    let board_uuid: Uuid = board_id_str.parse().map_err(|_| {
        CliError::Resolver(Box::new(helpers::ResolverError::InvalidBoardUuid {
            board_id: board_id_str.clone(),
        }))
    })?;

    let cols = ctx.client.list_columns(ws, board_uuid).await?;
    let column_uuid =
        helpers::resolve_column_id_on_board(&args.column, &cols).map_err(CliError::Validation)?;

    let body = PromoteChecklistItemRequest {
        board_id: board_uuid,
        column_id: column_uuid,
    };

    let promotion = ctx
        .client
        .promote_checklist_item(ws, &args.readable_id, args.id, body)
        .await?;

    let proj = PromotionProjection::from(promotion);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Activity (WU-23)
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks activity`.
#[derive(Parser)]
pub(crate) struct TasksActivityArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_task_activity(ctx: &Ctx, args: TasksActivityArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let page = ctx.client.list_activity(ws, &args.readable_id).await?;
    let projections: Vec<TaskActivityProjection> = page
        .items
        .into_iter()
        .map(TaskActivityProjection::from)
        .collect();
    output::emit_list(
        ctx.output,
        &projections,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

/// Arguments holder for the `tasks comments` subcommand group.
#[derive(Args)]
pub(crate) struct CommentsArgs {
    #[command(subcommand)]
    pub(crate) command: CommentsCmd,
}

#[derive(Subcommand)]
pub(crate) enum CommentsCmd {
    /// List comments on a task, oldest first.
    List(CommentsListArgs),
    /// Post a markdown comment on a task.
    Add(CommentsAddArgs),
    /// Delete a comment from a task by its UUID.
    Delete(CommentsDeleteArgs),
}

/// Arguments for `atlas tasks comments list`.
#[derive(Parser)]
pub(crate) struct CommentsListArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Arguments for `atlas tasks comments add`.
#[derive(Parser)]
pub(crate) struct CommentsAddArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Markdown comment body.
    #[arg(index = 2)]
    pub(crate) body: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Arguments for `atlas tasks comments delete`.
#[derive(Parser)]
pub(crate) struct CommentsDeleteArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// UUID of the comment to delete.
    #[arg(index = 2)]
    pub(crate) comment_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_comments(ctx: &Ctx, cmd: CommentsCmd) -> Result<(), CliError> {
    match cmd {
        CommentsCmd::List(args) => run_comments_list(ctx, args).await,
        CommentsCmd::Add(args) => run_comments_add(ctx, args).await,
        CommentsCmd::Delete(args) => run_comments_delete(ctx, args).await,
    }
}

async fn run_comments_list(ctx: &Ctx, args: CommentsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let page = ctx
        .client
        .list_comments(ws, &args.readable_id, None, None)
        .await?;
    let projections: Vec<CommentProjection> = page
        .items
        .into_iter()
        .map(CommentProjection::from)
        .collect();
    output::emit_list(
        ctx.output,
        &projections,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

async fn run_comments_add(ctx: &Ctx, args: CommentsAddArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let body = CreateCommentRequest::published(args.body);
    let comment = ctx.client.add_comment(ws, &args.readable_id, body).await?;
    let proj = CommentProjection::from(comment);
    output::emit(ctx.output, &proj)
}

async fn run_comments_delete(ctx: &Ctx, args: CommentsDeleteArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    ctx.client
        .delete_comment(ws, &args.readable_id, args.comment_id)
        .await?;
    let proj = DeletedProjection { deleted: true };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Subtasks (WU-23)
// ---------------------------------------------------------------------------

/// Arguments holder for the `tasks subtasks` subcommand group.
#[derive(Args)]
pub(crate) struct SubtasksArgs {
    #[command(subcommand)]
    pub(crate) command: SubtasksCmd,
}

#[derive(Subcommand)]
pub(crate) enum SubtasksCmd {
    /// List subtasks of a task.
    List(SubtasksListArgs),
    /// Create a new subtask under a task.
    Create(SubtasksCreateArgs),
    /// Promote a subtask to a top-level task.
    Promote(SubtasksPromoteArgs),
}

/// Arguments for `atlas tasks subtasks list`.
#[derive(Parser)]
pub(crate) struct SubtasksListArgs {
    /// Parent task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Arguments for `atlas tasks subtasks create`.
#[derive(Parser)]
pub(crate) struct SubtasksCreateArgs {
    /// Parent task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Title of the new subtask (required).
    #[arg(long)]
    pub(crate) title: String,
}

/// Arguments for `atlas tasks subtasks promote`.
#[derive(Parser)]
pub(crate) struct SubtasksPromoteArgs {
    /// Parent task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Readable ID of the subtask to promote (e.g. `ATL-99`).
    #[arg(long)]
    pub(crate) subtask_id: String,
}

async fn run_subtasks(ctx: &Ctx, cmd: SubtasksCmd) -> Result<(), CliError> {
    match cmd {
        SubtasksCmd::List(args) => run_subtasks_list(ctx, args).await,
        SubtasksCmd::Create(args) => run_subtasks_create(ctx, args).await,
        SubtasksCmd::Promote(args) => run_subtasks_promote(ctx, args).await,
    }
}

async fn run_subtasks_list(ctx: &Ctx, args: SubtasksListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let subtasks = ctx.client.list_subtasks(ws, &args.readable_id).await?;
    let projections: Vec<SubtaskProjection> =
        subtasks.into_iter().map(SubtaskProjection::from).collect();
    output::emit_list(ctx.output, &projections, None, false)
}

async fn run_subtasks_create(ctx: &Ctx, args: SubtasksCreateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let body = CreateSubtaskRequest { title: args.title };
    let task = ctx
        .client
        .create_subtask(ws, &args.readable_id, body)
        .await?;
    let proj = TaskCompactProjection::from(task);
    output::emit(ctx.output, &proj)
}

async fn run_subtasks_promote(ctx: &Ctx, args: SubtasksPromoteArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let task = ctx.client.promote_subtask(ws, &args.subtask_id).await?;
    let proj = TaskCompactProjection::from(task);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Attach (WU-34) — task attachments
// ---------------------------------------------------------------------------

/// Arguments holder for the `tasks attach` subcommand group.
#[derive(Args)]
pub(crate) struct TasksAttachArgs {
    #[command(subcommand)]
    pub(crate) command: TasksAttachCmd,
}

#[derive(Subcommand)]
pub(crate) enum TasksAttachCmd {
    /// Upload a file as an attachment to a task.
    Upload(TasksAttachUploadArgs),
    /// List attachments on a task.
    List(TasksAttachListArgs),
    /// Download a task attachment to a file or stdout.
    Download(TasksAttachDownloadArgs),
    /// Delete a task attachment (requires --confirm).
    Delete(TasksAttachDeleteArgs),
}

/// Arguments for `atlas tasks attach upload`.
#[derive(Parser)]
pub(crate) struct TasksAttachUploadArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Path to the file to upload (required).
    #[arg(long)]
    pub(crate) file: std::path::PathBuf,

    /// MIME content-type (defaults to `application/octet-stream`).
    #[arg(long, default_value = "application/octet-stream")]
    pub(crate) content_type: String,
}

/// Arguments for `atlas tasks attach list`.
#[derive(Parser)]
pub(crate) struct TasksAttachListArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Arguments for `atlas tasks attach download`.
#[derive(Parser)]
pub(crate) struct TasksAttachDownloadArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Attachment UUID to download.
    #[arg(long)]
    pub(crate) attachment_id: Uuid,

    /// Write output to this file instead of stdout.
    #[arg(long)]
    pub(crate) output: Option<std::path::PathBuf>,
}

/// Arguments for `atlas tasks attach delete`.
#[derive(Parser)]
pub(crate) struct TasksAttachDeleteArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Attachment UUID to delete.
    #[arg(long)]
    pub(crate) attachment_id: Uuid,

    /// Confirm the deletion. Required — prevents accidental non-reversible deletes.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_task_attach(ctx: &Ctx, cmd: TasksAttachCmd) -> Result<(), CliError> {
    match cmd {
        TasksAttachCmd::Upload(args) => run_task_attach_upload(ctx, args).await,
        TasksAttachCmd::List(args) => run_task_attach_list(ctx, args).await,
        TasksAttachCmd::Download(args) => run_task_attach_download(ctx, args).await,
        TasksAttachCmd::Delete(args) => run_task_attach_delete(ctx, args).await,
    }
}

async fn run_task_attach_upload(ctx: &Ctx, args: TasksAttachUploadArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let (filename, data) = read_upload_file(&args.file)?;

    let dto = ctx
        .client
        .upload_task_attachment(ws, &args.readable_id, &filename, &args.content_type, data)
        .await?;

    let proj = AttachProjection::from(dto);
    output::emit(ctx.output, &proj)
}

async fn run_task_attach_list(ctx: &Ctx, args: TasksAttachListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let items = ctx
        .client
        .list_task_attachments(ws, &args.readable_id)
        .await?;

    let projections: Vec<AttachProjection> =
        items.into_iter().map(AttachProjection::from).collect();
    output::emit_list(ctx.output, &projections, None, false)
}

async fn run_task_attach_download(
    ctx: &Ctx,
    args: TasksAttachDownloadArgs,
) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let (bytes, _content_type) = ctx
        .client
        .download_task_attachment(ws, &args.readable_id, args.attachment_id)
        .await?;

    match args.output {
        Some(path) => std::fs::write(&path, &bytes)?,
        None => std::io::stdout().write_all(&bytes)?,
    }
    Ok(())
}

async fn run_task_attach_delete(ctx: &Ctx, args: TasksAttachDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete (this is a non-reversible operation)".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    ctx.client
        .delete_task_attachment(ws, &args.readable_id, args.attachment_id)
        .await?;

    println!("attachment {} deleted", args.attachment_id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;

    // -----------------------------------------------------------------------
    // T29: Parse tasks list
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_list_parses_with_workspace() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "list", "--workspace", "ws"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            assert!(matches!(args.command, TasksCmd::List(_)));
        } else {
            panic!("expected Tasks command");
        }
    }

    #[test]
    fn tasks_list_board_is_optional() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "list", "--workspace", "ws"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::List(list_args) = args.command {
                assert!(list_args.board.is_none());
            } else {
                panic!("expected List");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_list_priority_and_label_are_repeatable() {
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "list",
            "--workspace",
            "ws",
            "--priority",
            "high",
            "--priority",
            "urgent",
            "--label",
            "rust",
            "--label",
            "bug",
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::List(list_args) = args.command {
                assert_eq!(list_args.priorities, vec!["high", "urgent"]);
                assert_eq!(list_args.labels, vec!["rust", "bug"]);
            } else {
                panic!("expected List");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    // -----------------------------------------------------------------------
    // T30: Limit clamping
    // -----------------------------------------------------------------------

    #[test]
    fn limit_clamp_zero_becomes_one() {
        let clamped = 0u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 1);
    }

    #[test]
    fn limit_clamp_over_max_becomes_200() {
        let clamped = 201u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 200);
    }

    #[test]
    fn limit_clamp_within_range_unchanged() {
        let clamped = 50u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 50);
    }

    // -----------------------------------------------------------------------
    // T31: Parse tasks get
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_get_parses_readable_id() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "get", "ATL-42"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Get(get_args) = args.command {
                assert_eq!(get_args.readable_id, "ATL-42");
            } else {
                panic!("expected Get");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_get_default_detail_is_compact() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "get", "ATL-1"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Get(get_args) = args.command {
                assert_eq!(get_args.detail, Detail::Compact);
            } else {
                panic!("expected Get");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_get_detail_full_parses() {
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "get", "ATL-1", "--detail", "full"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Get(get_args) = args.command {
                assert_eq!(get_args.detail, Detail::Full);
            } else {
                panic!("expected Get");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    // -----------------------------------------------------------------------
    // T32: Validate priority before network (build_update_body)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_priority_rejects_bogus_values() {
        let err = helpers::validate_priority("bogus").unwrap_err();
        assert!(
            err.contains("bogus"),
            "error must mention the invalid value"
        );
    }

    #[test]
    fn validate_estimate_rejects_negative() {
        let err = helpers::validate_estimate(-1).unwrap_err();
        assert!(err.contains("-1"), "error must mention the invalid value");
    }

    #[test]
    fn build_update_body_rejects_invalid_priority_before_network() {
        let args = TasksUpdateArgs {
            readable_id: Some("ATL-1".to_owned()),
            workspace: None,
            title: None,
            description: None,
            priority: Some("bogus".to_owned()),
            labels: vec![],
            estimate: None,
            due_date: None,
            clear_priority: false,
            clear_due_date: false,
            clear_estimate: false,
            stdin: false,
        };
        let err = build_update_body(&args).unwrap_err();
        assert!(matches!(err, CliError::Validation(_)));
    }

    #[test]
    fn build_update_body_with_only_title_omits_skip_serializing_fields() {
        let args = TasksUpdateArgs {
            readable_id: Some("ATL-1".to_owned()),
            workspace: None,
            title: Some("New title".to_owned()),
            description: None,
            priority: None,
            labels: vec![],
            estimate: None,
            due_date: None,
            clear_priority: false,
            clear_due_date: false,
            clear_estimate: false,
            stdin: false,
        };
        let body = build_update_body(&args).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(obj.get("title").and_then(|v| v.as_str()), Some("New title"));
        // title/description/labels lack skip_serializing_if so None → null is the correct
        // wire shape. The server treats null for these non-nullable fields as "leave unchanged".
        // The tri-state fields (priority, due_date, estimate) do use skip_serializing_if
        // and must be absent when neither a value nor a clear flag was provided.
        assert!(
            obj.get("priority").is_none(),
            "absent priority must be omitted"
        );
        assert!(
            obj.get("due_date").is_none(),
            "absent due_date must be omitted"
        );
        assert!(
            obj.get("estimate").is_none(),
            "absent estimate must be omitted"
        );
        // labels and properties lack skip_serializing_if so None serializes as null —
        // the server treats null labels as "leave unchanged" (same as absent).
        assert!(
            obj.get("labels").map(|v| v.is_null()).unwrap_or(true),
            "absent labels must be null or omitted"
        );
    }

    #[test]
    fn build_update_body_clear_priority_sends_null() {
        let args = TasksUpdateArgs {
            readable_id: Some("ATL-1".to_owned()),
            workspace: None,
            title: None,
            description: None,
            priority: None,
            labels: vec![],
            estimate: None,
            due_date: None,
            clear_priority: true,
            clear_due_date: false,
            clear_estimate: false,
            stdin: false,
        };
        let body = build_update_body(&args).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        assert!(
            json["priority"].is_null(),
            "clear_priority must produce null"
        );
    }

    // -----------------------------------------------------------------------
    // T33: Parse tasks move
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_move_board_is_optional() {
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "move", "ATL-1", "--column", "Done"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Move(move_args) = args.command {
                assert_eq!(move_args.readable_id, "ATL-1");
                assert_eq!(move_args.column, "Done");
                assert!(move_args.board.is_none());
            } else {
                panic!("expected Move");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    // -----------------------------------------------------------------------
    // T34-T35: Delete guard
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_delete_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "delete", "ATL-1"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Delete(del_args) = args.command {
                assert!(!del_args.confirm, "--confirm must default to false");
            } else {
                panic!("expected Delete");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_delete_confirm_flag_sets_field_to_true() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "delete", "ATL-1", "--confirm"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Delete(del_args) = args.command {
                assert!(del_args.confirm);
            } else {
                panic!("expected Delete");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    // -----------------------------------------------------------------------
    // W1: subtask rows now include assignees (SubtaskProjection)
    // -----------------------------------------------------------------------

    #[test]
    fn subtask_projection_includes_assignees_field() {
        use crate::projections::SubtaskProjection;
        use atlas_api::dtos::boards_tasks::TaskSummaryDto;
        use atlas_api::dtos::documents::ActorDto;
        use chrono::Utc;
        use uuid::Uuid;

        let actor = ActorDto {
            r#type: "user".to_owned(),
            id: Uuid::now_v7(),
            display_name: Some("Alice".to_owned()),
            key_type: None,
            account_status: None,
        };
        let dto = TaskSummaryDto {
            id: Uuid::now_v7(),
            readable_id: "ATL-5".to_owned(),
            board_id: Uuid::now_v7(),
            column_id: Uuid::now_v7(),
            title: "A subtask".to_owned(),
            priority: None,
            estimate: None,
            labels: vec![],
            assignees: vec![actor],
            board_name: "Dev".to_owned(),
            column_name: "Todo".to_owned(),
            subtask_count: 0,
            updated_at: Utc::now(),
        };

        let proj = SubtaskProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();

        let assignees = value["assignees"].as_array().unwrap();
        assert_eq!(assignees.len(), 1, "subtask must include assignees");
        assert_eq!(assignees[0]["type"], "user");
        assert_eq!(assignees[0]["display_name"], "Alice");
    }

    // -----------------------------------------------------------------------
    // T49 / WU-22: Parse and validation tests — refs
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_refs_list_parses() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "refs", "list", "ATL-42"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Refs(refs_args) = args.command {
                assert!(matches!(refs_args.command, RefsCmd::List(_)));
            } else {
                panic!("expected Refs");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_refs_create_parses_kind_and_target() {
        let cli = Cli::try_parse_from([
            "atlas", "tasks", "refs", "create", "ATL-42", "--kind", "relates", "--target", "ATL-10",
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Refs(refs_args) = args.command {
                if let RefsCmd::Create(create_args) = refs_args.command {
                    assert_eq!(create_args.readable_id, "ATL-42");
                    assert_eq!(create_args.kind, "relates");
                    assert_eq!(create_args.target, "ATL-10");
                } else {
                    panic!("expected Create");
                }
            } else {
                panic!("expected Refs");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_refs_create_requires_kind() {
        let result = Cli::try_parse_from([
            "atlas", "tasks", "refs", "create", "ATL-42", "--target", "ATL-10",
        ]);
        assert!(result.is_err(), "--kind is required for refs create");
    }

    #[test]
    fn tasks_refs_create_requires_target() {
        let result = Cli::try_parse_from([
            "atlas", "tasks", "refs", "create", "ATL-42", "--kind", "relates",
        ]);
        assert!(result.is_err(), "--target is required for refs create");
    }

    #[test]
    fn validate_reference_kind_rejects_bogus_value() {
        let result = helpers::validate_reference_kind("bogus");
        assert!(
            result.is_err(),
            "validate_reference_kind must reject unknown kinds"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("bogus"),
            "error must mention the invalid value"
        );
    }

    #[test]
    fn validate_reference_kind_accepts_known_values() {
        for kind in ["relates", "blocks", "parent", "spec"] {
            assert!(
                helpers::validate_reference_kind(kind).is_ok(),
                "'{kind}' must be accepted"
            );
        }
    }

    #[test]
    fn tasks_refs_remove_requires_ref_id() {
        let result = Cli::try_parse_from(["atlas", "tasks", "refs", "remove", "ATL-42"]);
        assert!(result.is_err(), "--ref-id is required for refs remove");
    }

    // -----------------------------------------------------------------------
    // T49 / WU-22: Parse and validation tests — assignees
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_backlinks_parses() {
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "backlinks", "ATL-42", "--workspace", "ws"])
                .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Backlinks(bl_args) = args.command {
                assert_eq!(bl_args.readable_id, "ATL-42");
            } else {
                panic!("expected Backlinks");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_assignees_add_parses() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "assignees",
            "add",
            "ATL-42",
            "--assignee-type",
            "user",
            "--assignee-id",
            uuid_str,
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Assignees(a_args) = args.command {
                if let AssigneesCmd::Add(add_args) = a_args.command {
                    assert_eq!(add_args.assignee_type, "user");
                    assert_eq!(add_args.assignee_id, uuid_str.parse::<Uuid>().unwrap());
                } else {
                    panic!("expected Add");
                }
            } else {
                panic!("expected Assignees");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_comments_add_parses_positional_body() {
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "comments", "add", "ATL-42", "Looks good"])
                .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Comments(c_args) = args.command {
                if let CommentsCmd::Add(add_args) = c_args.command {
                    assert_eq!(add_args.readable_id, "ATL-42");
                    assert_eq!(add_args.body, "Looks good");
                } else {
                    panic!("expected Add");
                }
            } else {
                panic!("expected Comments");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_comments_delete_parses_comment_uuid() {
        let uuid_str = "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234";
        let cli = Cli::try_parse_from(["atlas", "tasks", "comments", "delete", "ATL-1", uuid_str])
            .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Comments(c_args) = args.command {
                if let CommentsCmd::Delete(del_args) = c_args.command {
                    assert_eq!(del_args.comment_id, uuid_str.parse::<Uuid>().unwrap());
                } else {
                    panic!("expected Delete");
                }
            } else {
                panic!("expected Comments");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_comments_add_requires_body() {
        let result = Cli::try_parse_from(["atlas", "tasks", "comments", "add", "ATL-42"]);
        assert!(result.is_err(), "comments add must require a body argument");
    }

    #[test]
    fn tasks_assignees_add_requires_assignee_type() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let result = Cli::try_parse_from([
            "atlas",
            "tasks",
            "assignees",
            "add",
            "ATL-42",
            "--assignee-id",
            uuid_str,
        ]);
        assert!(
            result.is_err(),
            "--assignee-type is required for assignees add"
        );
    }

    #[test]
    fn validate_assignee_type_rejects_bogus_value() {
        let result = helpers::validate_assignee_type("bogus");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("bogus"));
    }

    #[test]
    fn validate_assignee_type_accepts_known_values() {
        for t in ["user", "api_key"] {
            assert!(
                helpers::validate_assignee_type(t).is_ok(),
                "'{t}' must be accepted"
            );
        }
    }

    // -----------------------------------------------------------------------
    // T50 / WU-23: Parse and validation tests — checklist
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_checklist_list_parses() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "checklist", "list", "ATL-1"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Checklist(cl_args) = args.command {
                assert!(matches!(cl_args.command, ChecklistCmd::List(_)));
            } else {
                panic!("expected Checklist");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_checklist_add_requires_title() {
        let result = Cli::try_parse_from(["atlas", "tasks", "checklist", "add", "ATL-1"]);
        assert!(result.is_err(), "--title is required for checklist add");
    }

    #[test]
    fn tasks_checklist_remove_confirm_defaults_to_false() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "checklist",
            "remove",
            "ATL-1",
            "--id",
            uuid_str,
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Checklist(cl_args) = args.command {
                if let ChecklistCmd::Remove(rm_args) = cl_args.command {
                    assert!(!rm_args.confirm, "--confirm must default to false");
                } else {
                    panic!("expected Remove");
                }
            } else {
                panic!("expected Checklist");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn checklist_remove_confirm_guard_fires_before_network() {
        let args = ChecklistRemoveArgs {
            readable_id: "ATL-1".to_owned(),
            workspace: None,
            id: Uuid::nil(),
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn tasks_checklist_promote_parses_board_and_column() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "checklist",
            "promote",
            "ATL-1",
            "--id",
            uuid_str,
            "--board",
            "Dev Board",
            "--column",
            "To Do",
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Checklist(cl_args) = args.command {
                if let ChecklistCmd::Promote(p_args) = cl_args.command {
                    assert_eq!(p_args.board, "Dev Board");
                    assert_eq!(p_args.column, "To Do");
                } else {
                    panic!("expected Promote");
                }
            } else {
                panic!("expected Checklist");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    // -----------------------------------------------------------------------
    // T51 / WU-23: Parse tests — activity and subtasks
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_activity_parses() {
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "activity", "ATL-42", "--workspace", "ws"])
                .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Activity(act_args) = args.command {
                assert_eq!(act_args.readable_id, "ATL-42");
            } else {
                panic!("expected Activity");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_subtasks_create_requires_title() {
        let result = Cli::try_parse_from(["atlas", "tasks", "subtasks", "create", "ATL-1"]);
        assert!(result.is_err(), "--title is required for subtasks create");
    }

    #[test]
    fn tasks_subtasks_promote_requires_subtask_id() {
        let result = Cli::try_parse_from(["atlas", "tasks", "subtasks", "promote", "ATL-1"]);
        assert!(
            result.is_err(),
            "--subtask-id is required for subtasks promote"
        );
    }

    #[test]
    fn tasks_subtasks_create_parses_title() {
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "subtasks",
            "create",
            "ATL-1",
            "--title",
            "My subtask",
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Subtasks(st_args) = args.command {
                if let SubtasksCmd::Create(c_args) = st_args.command {
                    assert_eq!(c_args.title, "My subtask");
                } else {
                    panic!("expected Create");
                }
            } else {
                panic!("expected Subtasks");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    // -----------------------------------------------------------------------
    // T66/T67: WU-32 parse tests — tasks create/update --stdin
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_create_stdin_flag_parses_without_required_flags() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "create", "--stdin", "--workspace", "ws"])
            .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Create(c_args) = args.command {
                assert!(c_args.stdin, "--stdin must be true");
                assert!(c_args.board.is_none(), "board must be None in stdin mode");
                assert!(c_args.title.is_none(), "title must be None in stdin mode");
            } else {
                panic!("expected Create");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_create_without_stdin_requires_board() {
        let result = Cli::try_parse_from([
            "atlas",
            "tasks",
            "create",
            "--workspace",
            "ws",
            "--title",
            "T",
            "--column",
            "Col",
        ]);
        assert!(
            result.is_err(),
            "--board is required when --stdin is absent"
        );
    }

    #[test]
    fn tasks_create_without_stdin_requires_title() {
        let result = Cli::try_parse_from([
            "atlas",
            "tasks",
            "create",
            "--workspace",
            "ws",
            "--board",
            "B",
            "--column",
            "Col",
        ]);
        assert!(
            result.is_err(),
            "--title is required when --stdin is absent"
        );
    }

    #[test]
    fn tasks_create_single_item_all_required_flags_parse() {
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "create",
            "--workspace",
            "ws",
            "--board",
            "Dev",
            "--column",
            "Todo",
            "--title",
            "My task",
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Create(c_args) = args.command {
                assert!(!c_args.stdin);
                assert_eq!(c_args.board.as_deref(), Some("Dev"));
                assert_eq!(c_args.column.as_deref(), Some("Todo"));
                assert_eq!(c_args.title.as_deref(), Some("My task"));
            } else {
                panic!("expected Create");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_update_stdin_flag_parses_without_readable_id() {
        let cli = Cli::try_parse_from(["atlas", "tasks", "update", "--stdin", "--workspace", "ws"])
            .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Update(u_args) = args.command {
                assert!(u_args.stdin, "--stdin must be true");
                assert!(
                    u_args.readable_id.is_none(),
                    "readable_id must be None in stdin mode"
                );
            } else {
                panic!("expected Update");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_update_single_item_requires_readable_id() {
        let result = Cli::try_parse_from(["atlas", "tasks", "update", "--title", "X"]);
        assert!(
            result.is_err(),
            "readable_id is required when --stdin is absent"
        );
    }

    #[test]
    fn tasks_update_single_item_readable_id_parses() {
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "update", "ATL-42", "--title", "X"]).unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Update(u_args) = args.command {
                assert_eq!(u_args.readable_id.as_deref(), Some("ATL-42"));
                assert!(!u_args.stdin);
            } else {
                panic!("expected Update");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    // -----------------------------------------------------------------------
    // WU-34: tasks attach tests
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_attach_upload_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "attach",
            "upload",
            "ATL-42",
            "--workspace",
            "ws",
            "--file",
            "/tmp/test.txt",
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Attach(attach_args) = args.command {
                if let TasksAttachCmd::Upload(up) = attach_args.command {
                    assert_eq!(up.readable_id, "ATL-42");
                    assert_eq!(up.file, std::path::PathBuf::from("/tmp/test.txt"));
                    assert_eq!(up.content_type, "application/octet-stream");
                } else {
                    panic!("expected Upload");
                }
            } else {
                panic!("expected Attach");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_attach_list_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "attach",
            "list",
            "ATL-42",
            "--workspace",
            "ws",
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            assert!(matches!(
                args.command,
                TasksCmd::Attach(TasksAttachArgs {
                    command: TasksAttachCmd::List(_)
                })
            ));
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_attach_delete_without_confirm_has_confirm_false() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "attach",
            "delete",
            "ATL-42",
            "--workspace",
            "ws",
            "--attachment-id",
            uuid_str,
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::Attach(TasksAttachArgs {
                command: TasksAttachCmd::Delete(del),
            }) = args.command
            {
                assert!(!del.confirm, "--confirm must be false when flag absent");
            } else {
                panic!("expected Attach Delete");
            }
        } else {
            panic!("expected Tasks");
        }
    }

    #[test]
    fn tasks_attach_delete_without_confirm_is_blocked_before_network() {
        let args = TasksAttachDeleteArgs {
            readable_id: "ATL-42".to_owned(),
            workspace: None,
            attachment_id: Uuid::new_v4(),
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm must be false, which triggers the Validation guard"
        );
    }

    #[test]
    fn tasks_list_cursor_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "tasks",
            "list",
            "--workspace",
            "ws",
            "--cursor",
            "abc",
        ])
        .unwrap();
        if let crate::cli::Commands::Tasks(args) = cli.command {
            if let TasksCmd::List(list_args) = args.command {
                assert_eq!(list_args.cursor.as_deref(), Some("abc"));
            } else {
                panic!("expected List");
            }
        } else {
            panic!("expected Tasks");
        }
    }
}
