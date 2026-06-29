#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use atlas_api::dtos::boards_tasks::{
    CreateTaskRequest, MoveTaskRequest, TaskPropertiesDto, UpdateTaskRequest,
    WorkspaceTaskQueryParams,
};
use clap::{Args, Parser, Subcommand, ValueEnum};

use atlas_client::helpers;
use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::{
    DeleteTaskProjection, TaskCompactProjection, TaskFullProjection, TaskSummaryProjection,
};

const LIMIT_MIN: u32 = 1;
const LIMIT_MAX: u32 = 200;
const LIMIT_DEFAULT: u32 = 20;

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
        cursor: None,
        limit: Some(limit),
    };

    let page = ctx.client.list_workspace_tasks(ws, &query).await?;

    let projections: Vec<TaskSummaryProjection> = page
        .items
        .into_iter()
        .map(TaskSummaryProjection::from)
        .collect();

    output::emit_list(ctx.output, &projections, page.next_cursor.as_deref(), page.has_more)
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
                        .map(|r| serde_json::json!({
                            "kind": r.kind,
                            "target_readable_id": r.target_readable_id,
                            "target_document_id": r.target_document_id,
                            "target_title": r.target_title,
                            "target_resolved": r.target_resolved,
                        }))
                        .collect::<Vec<_>>()
                })
                .map_err(|e| format!("list_references failed: {e}"));

            let subtasks = ctx
                .client
                .list_subtasks(ws, &args.readable_id)
                .await
                .map(|v| {
                    v.into_iter()
                        .map(|s| serde_json::json!({
                            "readable_id": s.readable_id,
                            "title": s.title,
                            "board_name": s.board_name,
                            "column_name": s.column_name,
                            "priority": s.priority,
                            "labels": s.labels,
                            "estimate": s.estimate,
                            "updated_at": s.updated_at,
                        }))
                        .collect::<Vec<_>>()
                })
                .map_err(|e| format!("list_subtasks failed: {e}"));

            let assignees = ctx
                .client
                .list_assignees(ws, &args.readable_id)
                .await
                .map(|v| {
                    v.into_iter()
                        .map(|a| serde_json::json!({
                            "type": a.assignee.r#type,
                            "display_name": a.assignee.display_name,
                            "assigned_at": a.assigned_at,
                        }))
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
#[derive(Parser)]
pub(crate) struct TasksCreateArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Task title (required).
    #[arg(long)]
    pub(crate) title: String,

    /// Board name or UUID (required).
    #[arg(long)]
    pub(crate) board: String,

    /// Column name on the board (required; resolved by case-insensitive substring).
    #[arg(long)]
    pub(crate) column: String,

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
}

async fn run_create(ctx: &Ctx, args: TasksCreateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    if let Some(pri) = &args.priority {
        helpers::validate_priority(pri).map_err(CliError::Validation)?;
    }
    if let Some(est) = args.estimate {
        helpers::validate_estimate(est).map_err(CliError::Validation)?;
    }

    let board_id_str = helpers::resolve_board_id(&ctx.client, ws, &args.board)
        .await
        .map_err(CliError::from)?;

    let board_uuid: uuid::Uuid = board_id_str.parse().map_err(|_| {
        CliError::Resolver(Box::new(atlas_client::helpers::ResolverError::InvalidBoardUuid {
            board_id: board_id_str.clone(),
        }))
    })?;

    let cols = ctx.client.list_columns(ws, board_uuid).await?;
    let column_uuid =
        helpers::resolve_column_id_on_board(&args.column, &cols).map_err(CliError::Validation)?;

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
        title: args.title,
        description: args.description,
        properties,
        before: None,
        after: None,
    };

    let task = ctx.client.create_task(ws, board_uuid, body).await?;
    let proj = TaskCompactProjection::from(task);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Arguments for `atlas tasks update`.
#[derive(Parser)]
pub(crate) struct TasksUpdateArgs {
    /// Task readable ID, e.g. `ATL-42`.
    #[arg(index = 1)]
    pub(crate) readable_id: String,

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
}

/// Builds the `UpdateTaskRequest` from the parsed update arguments.
///
/// Validates priority and estimate before constructing the body so that
/// invalid inputs are rejected before any network call is made. Nullable
/// fields follow the PATCH tri-state: absent = unchanged, `--clear-X` =
/// null (clear), explicit value = set.
pub(crate) fn build_update_body(
    args: &TasksUpdateArgs,
) -> Result<UpdateTaskRequest, CliError> {
    if let Some(pri) = &args.priority {
        helpers::validate_priority(pri).map_err(CliError::Validation)?;
    }
    if let Some(est) = args.estimate {
        helpers::validate_estimate(est).map_err(CliError::Validation)?;
    }

    let priority = if args.clear_priority {
        Some(serde_json::Value::Null)
    } else {
        args.priority.as_ref().map(|p| serde_json::Value::String(p.clone()))
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
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let readable_id = args.readable_id.clone();
    let body = build_update_body(&args)?;
    let task = ctx.client.update_task(ws, &readable_id, body).await?;
    let proj = TaskCompactProjection::from(task);
    output::emit(ctx.output, &proj)
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
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "list", "--workspace", "ws"]).unwrap();
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
            "atlas", "tasks", "list", "--workspace", "ws",
            "--priority", "high", "--priority", "urgent",
            "--label", "rust", "--label", "bug",
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
        assert!(err.contains("bogus"), "error must mention the invalid value");
    }

    #[test]
    fn validate_estimate_rejects_negative() {
        let err = helpers::validate_estimate(-1).unwrap_err();
        assert!(err.contains("-1"), "error must mention the invalid value");
    }

    #[test]
    fn build_update_body_rejects_invalid_priority_before_network() {
        let args = TasksUpdateArgs {
            readable_id: "ATL-1".to_owned(),
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
        };
        let err = build_update_body(&args).unwrap_err();
        assert!(matches!(err, CliError::Validation(_)));
    }

    #[test]
    fn build_update_body_with_only_title_omits_skip_serializing_fields() {
        let args = TasksUpdateArgs {
            readable_id: "ATL-1".to_owned(),
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
        };
        let body = build_update_body(&args).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(obj.get("title").and_then(|v| v.as_str()), Some("New title"));
        // title/description/labels lack skip_serializing_if so None → null is the correct
        // wire shape. The server treats null for these non-nullable fields as "leave unchanged".
        // The tri-state fields (priority, due_date, estimate) do use skip_serializing_if
        // and must be absent when neither a value nor a clear flag was provided.
        assert!(obj.get("priority").is_none(), "absent priority must be omitted");
        assert!(obj.get("due_date").is_none(), "absent due_date must be omitted");
        assert!(obj.get("estimate").is_none(), "absent estimate must be omitted");
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
            readable_id: "ATL-1".to_owned(),
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
        };
        let body = build_update_body(&args).unwrap();
        let json = serde_json::to_value(&body).unwrap();
        assert!(json["priority"].is_null(), "clear_priority must produce null");
    }

    // -----------------------------------------------------------------------
    // T33: Parse tasks move
    // -----------------------------------------------------------------------

    #[test]
    fn tasks_move_board_is_optional() {
        let cli = Cli::try_parse_from([
            "atlas", "tasks", "move", "ATL-1", "--column", "Done",
        ])
        .unwrap();
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
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "delete", "ATL-1"]).unwrap();
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
        let cli =
            Cli::try_parse_from(["atlas", "tasks", "delete", "ATL-1", "--confirm"]).unwrap();
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
}
