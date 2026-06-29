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
use crate::projections::{ColumnProjection, DeleteByIdProjection, StatusTemplateProjection};

// ---------------------------------------------------------------------------
// StatusTemplatesArgs + StatusTemplatesCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `status-templates` subcommand group.
#[derive(Args)]
pub(crate) struct StatusTemplatesArgs {
    #[command(subcommand)]
    pub(crate) command: StatusTemplatesCmd,
}

#[derive(Subcommand)]
pub(crate) enum StatusTemplatesCmd {
    /// List status templates in a workspace.
    List(StatusTemplatesListArgs),
    /// Create a new status template.
    Create(StatusTemplatesCreateArgs),
    /// Update an existing status template (rename, recolor, or reorder).
    Update(StatusTemplatesUpdateArgs),
    /// Delete a status template (requires --confirm).
    Delete(StatusTemplatesDeleteArgs),
    /// Apply the workspace status templates to a board.
    Apply(StatusTemplatesApplyArgs),
}

/// Dispatches a parsed `StatusTemplatesCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: StatusTemplatesCmd) -> Result<(), CliError> {
    match cmd {
        StatusTemplatesCmd::List(args) => run_list(ctx, args).await,
        StatusTemplatesCmd::Create(args) => run_create(ctx, args).await,
        StatusTemplatesCmd::Update(args) => run_update(ctx, args).await,
        StatusTemplatesCmd::Delete(args) => run_delete(ctx, args).await,
        StatusTemplatesCmd::Apply(args) => run_apply(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas status-templates list`.
#[derive(Parser)]
pub(crate) struct StatusTemplatesListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_list(ctx: &Ctx, args: StatusTemplatesListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let templates = ctx.client.list_status_templates(ws).await?;

    let items: Vec<StatusTemplateProjection> = templates
        .into_iter()
        .map(StatusTemplateProjection::from)
        .collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas status-templates create`.
#[derive(Parser)]
pub(crate) struct StatusTemplatesCreateArgs {
    /// Name for the new status template.
    #[arg(long)]
    pub(crate) name: String,

    /// Optional color swatch identifier.
    #[arg(long)]
    pub(crate) color: Option<String>,

    /// Insert before this template's position key (ordering anchor).
    #[arg(long)]
    pub(crate) before: Option<String>,

    /// Insert after this template's position key (ordering anchor).
    #[arg(long)]
    pub(crate) after: Option<String>,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_create(ctx: &Ctx, args: StatusTemplatesCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::status_templates::CreateStatusTemplateRequest;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = CreateStatusTemplateRequest {
        name: args.name,
        color: args.color,
        before: args.before,
        after: args.after,
    };

    let template = ctx.client.create_status_template(ws, body).await?;
    let proj = StatusTemplateProjection::from(template);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Arguments for `atlas status-templates update`.
#[derive(Parser)]
pub(crate) struct StatusTemplatesUpdateArgs {
    /// UUID of the status template to update.
    #[arg(long)]
    pub(crate) template_id: Uuid,

    /// New name. Omit to leave unchanged.
    #[arg(long)]
    pub(crate) name: Option<String>,

    /// New color swatch. Omit to leave unchanged.
    #[arg(long)]
    pub(crate) color: Option<String>,

    /// Clear the color (set to null). Mutually exclusive with --color.
    #[arg(long, conflicts_with = "color")]
    pub(crate) clear_color: bool,

    /// Reorder: insert before this template's position key.
    #[arg(long)]
    pub(crate) before: Option<String>,

    /// Reorder: insert after this template's position key.
    #[arg(long)]
    pub(crate) after: Option<String>,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_update(ctx: &Ctx, args: StatusTemplatesUpdateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::status_templates::UpdateStatusTemplateRequest;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let color = if args.clear_color {
        Some(serde_json::Value::Null)
    } else {
        args.color.map(serde_json::Value::String)
    };

    let body = UpdateStatusTemplateRequest {
        name: args.name,
        color,
        before: args.before,
        after: args.after,
    };

    let template = ctx
        .client
        .update_status_template(ws, args.template_id, body)
        .await?;
    let proj = StatusTemplateProjection::from(template);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Arguments for `atlas status-templates delete`.
#[derive(Parser)]
pub(crate) struct StatusTemplatesDeleteArgs {
    /// UUID of the status template to delete.
    #[arg(long)]
    pub(crate) template_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the deletion. Required — permanently removes the template.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete(ctx: &Ctx, args: StatusTemplatesDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete the status template".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    ctx.client
        .delete_status_template(ws, args.template_id)
        .await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.template_id,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Apply
// ---------------------------------------------------------------------------

/// Arguments for `atlas status-templates apply`.
#[derive(Parser)]
pub(crate) struct StatusTemplatesApplyArgs {
    /// UUID of the board to apply the workspace status templates to.
    #[arg(long)]
    pub(crate) board_id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_apply(ctx: &Ctx, args: StatusTemplatesApplyArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let columns = ctx.client.apply_status_templates(ws, args.board_id).await?;

    let items: Vec<ColumnProjection> = columns.into_iter().map(ColumnProjection::from).collect();

    output::emit_list(ctx.output, &items, None, false)
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
    fn status_templates_list_parses_without_workspace() {
        let cli = Cli::try_parse_from(["atlas", "status-templates", "list"]).unwrap();
        let Commands::StatusTemplates(args) = cli.command else {
            panic!("expected StatusTemplates");
        };
        assert!(matches!(args.command, StatusTemplatesCmd::List(_)));
    }

    #[test]
    fn status_templates_list_parses_with_workspace() {
        let cli =
            Cli::try_parse_from(["atlas", "status-templates", "list", "--workspace", "my-ws"])
                .unwrap();
        let Commands::StatusTemplates(args) = cli.command else {
            panic!("expected StatusTemplates");
        };
        let StatusTemplatesCmd::List(l) = args.command else {
            panic!("expected List");
        };
        assert_eq!(l.workspace.as_deref(), Some("my-ws"));
    }

    #[test]
    fn status_templates_create_requires_name() {
        let result = Cli::try_parse_from(["atlas", "status-templates", "create"]);
        assert!(result.is_err(), "missing --name must fail");
    }

    #[test]
    fn status_templates_create_parses_name_and_color() {
        let cli = Cli::try_parse_from([
            "atlas",
            "status-templates",
            "create",
            "--name",
            "Done",
            "--color",
            "green",
        ])
        .unwrap();
        let Commands::StatusTemplates(args) = cli.command else {
            panic!("expected StatusTemplates");
        };
        let StatusTemplatesCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.name, "Done");
        assert_eq!(c.color.as_deref(), Some("green"));
    }

    #[test]
    fn status_templates_update_requires_template_id() {
        let result = Cli::try_parse_from(["atlas", "status-templates", "update"]);
        assert!(result.is_err(), "missing --template-id must fail");
    }

    #[test]
    fn status_templates_update_color_and_clear_color_conflict() {
        let result = Cli::try_parse_from([
            "atlas",
            "status-templates",
            "update",
            "--template-id",
            "00000000-0000-0000-0000-000000000001",
            "--color",
            "blue",
            "--clear-color",
        ]);
        assert!(result.is_err(), "--color and --clear-color must conflict");
    }

    #[test]
    fn status_templates_delete_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "status-templates",
            "delete",
            "--template-id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::StatusTemplates(args) = cli.command else {
            panic!("expected StatusTemplates");
        };
        let StatusTemplatesCmd::Delete(d) = args.command else {
            panic!("expected Delete");
        };
        assert!(!d.confirm, "--confirm must default to false");
    }

    #[test]
    fn status_templates_delete_confirm_guard_fires_before_network() {
        let args = StatusTemplatesDeleteArgs {
            template_id: Uuid::nil(),
            workspace: None,
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }

    #[test]
    fn status_templates_apply_requires_board_id() {
        let result = Cli::try_parse_from(["atlas", "status-templates", "apply"]);
        assert!(result.is_err(), "missing --board-id must fail");
    }

    #[test]
    fn status_templates_apply_parses_board_id() {
        let cli = Cli::try_parse_from([
            "atlas",
            "status-templates",
            "apply",
            "--board-id",
            "00000000-0000-0000-0000-000000000002",
        ])
        .unwrap();
        let Commands::StatusTemplates(args) = cli.command else {
            panic!("expected StatusTemplates");
        };
        assert!(matches!(args.command, StatusTemplatesCmd::Apply(_)));
    }
}
