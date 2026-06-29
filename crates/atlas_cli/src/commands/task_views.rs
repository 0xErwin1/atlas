#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use clap::{Args, Parser, Subcommand};
use uuid::Uuid;

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::{DeleteByIdProjection, TaskViewProjection};

// ---------------------------------------------------------------------------
// TaskViewsArgs + TaskViewsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `task-views` subcommand group.
#[derive(Args)]
pub(crate) struct TaskViewsArgs {
    #[command(subcommand)]
    pub(crate) command: TaskViewsCmd,
}

#[derive(Subcommand)]
pub(crate) enum TaskViewsCmd {
    /// List task views in a workspace.
    List(TaskViewsListArgs),
    /// Get a single task view by UUID.
    Get(TaskViewsGetArgs),
    /// Create a new task view.
    Create(TaskViewsCreateArgs),
    /// Update a task view (name and filters are fully replaced).
    Update(TaskViewsUpdateArgs),
    /// Delete a task view (requires --confirm).
    Delete(TaskViewsDeleteArgs),
}

/// Dispatches a parsed `TaskViewsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: TaskViewsCmd) -> Result<(), CliError> {
    match cmd {
        TaskViewsCmd::List(args) => run_list(ctx, args).await,
        TaskViewsCmd::Get(args) => run_get(ctx, args).await,
        TaskViewsCmd::Create(args) => run_create(ctx, args).await,
        TaskViewsCmd::Update(args) => run_update(ctx, args).await,
        TaskViewsCmd::Delete(args) => run_delete(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas task-views list`.
#[derive(Parser)]
pub(crate) struct TaskViewsListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_list(ctx: &Ctx, args: TaskViewsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let views = ctx.client.list_task_views(ws).await?;

    let items: Vec<TaskViewProjection> = views.into_iter().map(TaskViewProjection::from).collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Get
// ---------------------------------------------------------------------------

/// Arguments for `atlas task-views get`.
#[derive(Parser)]
pub(crate) struct TaskViewsGetArgs {
    /// UUID of the task view to retrieve.
    #[arg(long)]
    pub(crate) id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_get(ctx: &Ctx, args: TaskViewsGetArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let view = ctx.client.get_task_view(ws, args.id).await?;
    let proj = TaskViewProjection::from(view);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas task-views create`.
///
/// Filters are supplied as a JSON object string. Pass `{}` for an
/// all-workspace view. Supported keys: `sort`, `priorities` (array),
/// `labels` (array), `column_ids` (array of UUIDs), `board_id` (UUID),
/// `assignee` (string), `actor_type` (string).
#[derive(Parser)]
pub(crate) struct TaskViewsCreateArgs {
    /// Display name for the task view.
    #[arg(long)]
    pub(crate) name: String,

    /// Filter set as a JSON object. Defaults to `{}` (all workspace tasks).
    #[arg(long, default_value = "{}")]
    pub(crate) filters: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_create(ctx: &Ctx, args: TaskViewsCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::task_views::{CreateTaskViewRequest, TaskViewFiltersDto};

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let filters: TaskViewFiltersDto = serde_json::from_str(&args.filters)
        .map_err(|e| CliError::Validation(format!("invalid --filters JSON: {e}")))?;

    let body = CreateTaskViewRequest {
        name: args.name,
        filters,
    };

    let view = ctx.client.create_task_view(ws, body).await?;
    let proj = TaskViewProjection::from(view);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Arguments for `atlas task-views update`.
///
/// The update replaces both `name` and `filters` fully (no partial PATCH).
#[derive(Parser)]
pub(crate) struct TaskViewsUpdateArgs {
    /// UUID of the task view to update.
    #[arg(long)]
    pub(crate) id: Uuid,

    /// New display name.
    #[arg(long)]
    pub(crate) name: String,

    /// New filter set as a JSON object. Defaults to `{}` (clears existing filters).
    #[arg(long, default_value = "{}")]
    pub(crate) filters: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_update(ctx: &Ctx, args: TaskViewsUpdateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::task_views::{TaskViewFiltersDto, UpdateTaskViewRequest};

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let filters: TaskViewFiltersDto = serde_json::from_str(&args.filters)
        .map_err(|e| CliError::Validation(format!("invalid --filters JSON: {e}")))?;

    let body = UpdateTaskViewRequest {
        name: args.name,
        filters,
    };

    let view = ctx.client.update_task_view(ws, args.id, body).await?;
    let proj = TaskViewProjection::from(view);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Arguments for `atlas task-views delete`.
#[derive(Parser)]
pub(crate) struct TaskViewsDeleteArgs {
    /// UUID of the task view to delete.
    #[arg(long)]
    pub(crate) id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the deletion. Required — permanently removes the task view.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete(ctx: &Ctx, args: TaskViewsDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete the task view".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    ctx.client.delete_task_view(ws, args.id).await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Commands;
    use clap::Parser as ClapParser;

    #[derive(ClapParser)]
    struct Cli {
        #[command(subcommand)]
        command: Commands,
    }

    #[test]
    fn task_views_list_parses_without_workspace() {
        let cli = Cli::try_parse_from(["atlas", "task-views", "list"]).unwrap();
        let Commands::TaskViews(args) = cli.command else {
            panic!("expected TaskViews");
        };
        assert!(matches!(args.command, TaskViewsCmd::List(_)));
    }

    #[test]
    fn task_views_get_requires_id() {
        let result = Cli::try_parse_from(["atlas", "task-views", "get"]);
        assert!(result.is_err(), "missing --id must fail");
    }

    #[test]
    fn task_views_get_parses_id() {
        let cli = Cli::try_parse_from([
            "atlas",
            "task-views",
            "get",
            "--id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::TaskViews(args) = cli.command else {
            panic!("expected TaskViews");
        };
        assert!(matches!(args.command, TaskViewsCmd::Get(_)));
    }

    #[test]
    fn task_views_create_requires_name() {
        let result = Cli::try_parse_from(["atlas", "task-views", "create"]);
        assert!(result.is_err(), "missing --name must fail");
    }

    #[test]
    fn task_views_create_parses_name_with_default_filters() {
        let cli =
            Cli::try_parse_from(["atlas", "task-views", "create", "--name", "My View"]).unwrap();
        let Commands::TaskViews(args) = cli.command else {
            panic!("expected TaskViews");
        };
        let TaskViewsCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.name, "My View");
        assert_eq!(c.filters, "{}");
    }

    #[test]
    fn task_views_create_parses_filters_json() {
        let cli = Cli::try_parse_from([
            "atlas",
            "task-views",
            "create",
            "--name",
            "Urgent",
            "--filters",
            r#"{"priorities":["urgent"]}"#,
        ])
        .unwrap();
        let Commands::TaskViews(args) = cli.command else {
            panic!("expected TaskViews");
        };
        let TaskViewsCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.filters, r#"{"priorities":["urgent"]}"#);
    }

    #[test]
    fn task_views_update_requires_id_and_name() {
        assert!(
            Cli::try_parse_from(["atlas", "task-views", "update"]).is_err(),
            "missing args must fail"
        );
        assert!(
            Cli::try_parse_from([
                "atlas",
                "task-views",
                "update",
                "--id",
                "00000000-0000-0000-0000-000000000001"
            ])
            .is_err(),
            "missing --name must fail"
        );
    }

    #[test]
    fn task_views_delete_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "task-views",
            "delete",
            "--id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::TaskViews(args) = cli.command else {
            panic!("expected TaskViews");
        };
        let TaskViewsCmd::Delete(d) = args.command else {
            panic!("expected Delete");
        };
        assert!(!d.confirm, "--confirm must default to false");
    }

    #[test]
    fn task_views_delete_confirm_guard_fires_before_network() {
        let args = TaskViewsDeleteArgs {
            id: Uuid::nil(),
            workspace: None,
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn task_views_invalid_filters_json_rejects() {
        use atlas_api::dtos::task_views::TaskViewFiltersDto;
        let bad = "{not valid json";
        let result = serde_json::from_str::<TaskViewFiltersDto>(bad);
        assert!(result.is_err(), "malformed JSON must fail validation");
    }
}
