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

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::WorkspaceProjection;

// ---------------------------------------------------------------------------
// WorkspacesArgs + WorkspacesCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `workspaces` subcommand group.
#[derive(Args)]
pub(crate) struct WorkspacesArgs {
    #[command(subcommand)]
    pub(crate) command: WorkspacesCmd,
}

#[derive(Subcommand)]
pub(crate) enum WorkspacesCmd {
    /// List all workspaces the current principal can access.
    List,
    /// Get a single workspace by its slug.
    Get(WorkspacesGetArgs),
}

/// Dispatches a parsed `WorkspacesCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: WorkspacesCmd) -> Result<(), CliError> {
    match cmd {
        WorkspacesCmd::List => run_list(ctx).await,
        WorkspacesCmd::Get(args) => run_get(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

async fn run_list(ctx: &Ctx) -> Result<(), CliError> {
    let workspaces = ctx.client.list_workspaces().await?;

    let items: Vec<WorkspaceProjection> = workspaces
        .into_iter()
        .map(WorkspaceProjection::from)
        .collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Get
// ---------------------------------------------------------------------------

/// Arguments for `atlas workspaces get`.
#[derive(Parser)]
pub(crate) struct WorkspacesGetArgs {
    /// Workspace slug.
    #[arg(index = 1)]
    pub(crate) slug: String,
}

async fn run_get(ctx: &Ctx, args: WorkspacesGetArgs) -> Result<(), CliError> {
    let ws = ctx.client.get_workspace(&args.slug).await?;
    let proj = WorkspaceProjection::from(ws);
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
    fn workspaces_list_parses() {
        let cli = Cli::try_parse_from(["atlas", "workspaces", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Workspaces(_)));
    }

    #[test]
    fn workspaces_get_parses_slug() {
        let cli = Cli::try_parse_from(["atlas", "workspaces", "get", "my-ws"]).unwrap();
        if let Commands::Workspaces(args) = cli.command {
            if let WorkspacesCmd::Get(get) = args.command {
                assert_eq!(get.slug, "my-ws");
            } else {
                panic!("expected Get");
            }
        } else {
            panic!("expected Workspaces");
        }
    }

    #[test]
    fn workspaces_get_requires_slug() {
        let result = Cli::try_parse_from(["atlas", "workspaces", "get"]);
        assert!(result.is_err(), "get without slug must fail");
    }
}
