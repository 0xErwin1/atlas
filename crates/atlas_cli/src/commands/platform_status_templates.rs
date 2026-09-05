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
use crate::projections::{DeleteByIdProjection, PlatformStatusTemplateProjection};

// ---------------------------------------------------------------------------
// PlatformStatusTemplatesArgs + PlatformStatusTemplatesCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `platform-status-templates` subcommand group.
#[derive(Args)]
pub(crate) struct PlatformStatusTemplatesArgs {
    #[command(subcommand)]
    pub(crate) command: PlatformStatusTemplatesCmd,
}

#[derive(Subcommand)]
pub(crate) enum PlatformStatusTemplatesCmd {
    /// List the Atlas-wide default statuses.
    List,
    /// Create a new Atlas-wide default status (appended last).
    Create(PlatformStatusTemplatesCreateArgs),
    /// Update an Atlas-wide default status (rename, recolor, or reorder).
    Update(PlatformStatusTemplatesUpdateArgs),
    /// Delete an Atlas-wide default status (requires --confirm).
    Delete(PlatformStatusTemplatesDeleteArgs),
}

/// Dispatches a parsed `PlatformStatusTemplatesCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: PlatformStatusTemplatesCmd) -> Result<(), CliError> {
    match cmd {
        PlatformStatusTemplatesCmd::List => run_list(ctx).await,
        PlatformStatusTemplatesCmd::Create(args) => run_create(ctx, args).await,
        PlatformStatusTemplatesCmd::Update(args) => run_update(ctx, args).await,
        PlatformStatusTemplatesCmd::Delete(args) => run_delete(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

async fn run_list(ctx: &Ctx) -> Result<(), CliError> {
    let templates = ctx.client.acta().list_platform_status_templates().await?;

    let items: Vec<PlatformStatusTemplateProjection> = templates
        .into_iter()
        .map(PlatformStatusTemplateProjection::from)
        .collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas platform-status-templates create`.
#[derive(Parser)]
pub(crate) struct PlatformStatusTemplatesCreateArgs {
    /// Name for the new default status.
    #[arg(long)]
    pub(crate) name: String,

    /// Optional color swatch identifier.
    #[arg(long)]
    pub(crate) color: Option<String>,
}

async fn run_create(ctx: &Ctx, args: PlatformStatusTemplatesCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::status_templates::CreateStatusTemplateRequest;

    let body = CreateStatusTemplateRequest {
        name: args.name,
        color: args.color,
        before: None,
        after: None,
    };

    let template = ctx
        .client
        .acta()
        .create_platform_status_template(body)
        .await?;
    let proj = PlatformStatusTemplateProjection::from(template);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Arguments for `atlas platform-status-templates update`.
#[derive(Parser)]
pub(crate) struct PlatformStatusTemplatesUpdateArgs {
    /// UUID of the default status to update.
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
}

async fn run_update(ctx: &Ctx, args: PlatformStatusTemplatesUpdateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::status_templates::UpdateStatusTemplateRequest;

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
        .acta()
        .update_platform_status_template(args.template_id, body)
        .await?;
    let proj = PlatformStatusTemplateProjection::from(template);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Arguments for `atlas platform-status-templates delete`.
#[derive(Parser)]
pub(crate) struct PlatformStatusTemplatesDeleteArgs {
    /// UUID of the default status to delete.
    #[arg(long)]
    pub(crate) template_id: Uuid,

    /// Confirm the deletion. Required — removes the default status.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete(ctx: &Ctx, args: PlatformStatusTemplatesDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete the default status".to_owned(),
        ));
    }

    ctx.client
        .acta()
        .delete_platform_status_template(args.template_id)
        .await?;

    let proj = DeleteByIdProjection {
        deleted: true,
        id: args.template_id,
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
    fn platform_status_templates_list_parses_without_workspace() {
        let cli = Cli::try_parse_from(["atlas", "platform-status-templates", "list"]).unwrap();
        let Commands::PlatformStatusTemplates(args) = cli.command else {
            panic!("expected PlatformStatusTemplates");
        };
        assert!(matches!(args.command, PlatformStatusTemplatesCmd::List));
    }

    #[test]
    fn platform_status_templates_list_rejects_workspace_flag() {
        let result = Cli::try_parse_from([
            "atlas",
            "platform-status-templates",
            "list",
            "--workspace",
            "ws",
        ]);
        assert!(
            result.is_err(),
            "platform defaults are instance-wide: --workspace must not be accepted"
        );
    }

    #[test]
    fn platform_status_templates_create_requires_name() {
        let result = Cli::try_parse_from(["atlas", "platform-status-templates", "create"]);
        assert!(result.is_err(), "missing --name must fail");
    }

    #[test]
    fn platform_status_templates_create_parses_name_and_color() {
        let cli = Cli::try_parse_from([
            "atlas",
            "platform-status-templates",
            "create",
            "--name",
            "Done",
            "--color",
            "green",
        ])
        .unwrap();
        let Commands::PlatformStatusTemplates(args) = cli.command else {
            panic!("expected PlatformStatusTemplates");
        };
        let PlatformStatusTemplatesCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.name, "Done");
        assert_eq!(c.color.as_deref(), Some("green"));
    }

    #[test]
    fn platform_status_templates_update_color_and_clear_color_conflict() {
        let result = Cli::try_parse_from([
            "atlas",
            "platform-status-templates",
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
    fn platform_status_templates_delete_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "platform-status-templates",
            "delete",
            "--template-id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::PlatformStatusTemplates(args) = cli.command else {
            panic!("expected PlatformStatusTemplates");
        };
        let PlatformStatusTemplatesCmd::Delete(d) = args.command else {
            panic!("expected Delete");
        };
        assert!(!d.confirm, "--confirm must default to false");
    }
}
