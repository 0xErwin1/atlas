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
use crate::projections::WorkspaceActivityProjection;

const LIMIT_MIN: u32 = 1;
const LIMIT_MAX: u32 = 200;
const LIMIT_DEFAULT: u32 = 20;

// ---------------------------------------------------------------------------
// ActivityArgs (wrapper for nesting into Commands) + ActivityCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `activity` subcommand group.
#[derive(Args)]
pub(crate) struct ActivityArgs {
    #[command(subcommand)]
    pub(crate) command: ActivityCmd,
}

#[derive(Subcommand)]
pub(crate) enum ActivityCmd {
    /// List workspace-level activity entries.
    List(ActivityListArgs),
}

/// Dispatches a parsed `ActivityCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: ActivityCmd) -> Result<(), CliError> {
    match cmd {
        ActivityCmd::List(args) => run_list(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas activity list`.
#[derive(Parser)]
pub(crate) struct ActivityListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Maximum number of activity entries to return (clamped to 1..=200; default 20).
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

async fn run_list(ctx: &Ctx, args: ActivityListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let page = ctx
        .client
        .list_workspace_activity(ws, None, None, None, Some(limit))
        .await?;

    let projections: Vec<WorkspaceActivityProjection> = page
        .items
        .into_iter()
        .map(WorkspaceActivityProjection::from)
        .collect();

    output::emit_list(
        ctx.output,
        &projections,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;

    // -----------------------------------------------------------------------
    // T53 / WU-24: Parse tests — activity list
    // -----------------------------------------------------------------------

    #[test]
    fn activity_list_parses_with_workspace() {
        let cli =
            Cli::try_parse_from(["atlas", "activity", "list", "--workspace", "my-ws"]).unwrap();
        if let crate::cli::Commands::Activity(args) = cli.command {
            let ActivityCmd::List(list_args) = args.command;
            assert_eq!(list_args.workspace.as_deref(), Some("my-ws"));
        } else {
            panic!("expected Activity");
        }
    }

    #[test]
    fn activity_list_limit_is_optional() {
        let cli = Cli::try_parse_from(["atlas", "activity", "list", "--workspace", "ws"]).unwrap();
        if let crate::cli::Commands::Activity(args) = cli.command {
            let ActivityCmd::List(list_args) = args.command;
            assert!(list_args.limit.is_none());
        } else {
            panic!("expected Activity");
        }
    }

    #[test]
    fn activity_list_limit_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "activity",
            "list",
            "--workspace",
            "ws",
            "--limit",
            "50",
        ])
        .unwrap();
        if let crate::cli::Commands::Activity(args) = cli.command {
            let ActivityCmd::List(list_args) = args.command;
            assert_eq!(list_args.limit, Some(50));
        } else {
            panic!("expected Activity");
        }
    }

    #[test]
    fn activity_limit_clamp_zero_becomes_one() {
        let clamped = 0u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 1);
    }

    #[test]
    fn activity_limit_clamp_over_max_becomes_200() {
        let clamped = 201u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 200);
    }
}
