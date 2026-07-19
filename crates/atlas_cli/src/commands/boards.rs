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

use crate::commands::common::{LIMIT_DEFAULT, LIMIT_MAX, LIMIT_MIN};
use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::BoardProjection;

// ---------------------------------------------------------------------------
// BoardsArgs + BoardsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `boards` subcommand group.
#[derive(Args)]
pub(crate) struct BoardsArgs {
    #[command(subcommand)]
    pub(crate) command: BoardsCmd,
}

#[derive(Subcommand)]
pub(crate) enum BoardsCmd {
    /// List boards in a project.
    List(BoardsListArgs),
}

/// Dispatches a parsed `BoardsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: BoardsCmd) -> Result<(), CliError> {
    match cmd {
        BoardsCmd::List(args) => run_list(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas boards list`.
#[derive(Parser)]
pub(crate) struct BoardsListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Project slug (required; boards are scoped to a project).
    #[arg(long)]
    pub(crate) project: String,

    /// Maximum number of results (clamped to 1..=200; default 20).
    #[arg(long)]
    pub(crate) limit: Option<u32>,

    /// Pagination cursor returned by a previous list.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
}

async fn run_list(ctx: &Ctx, args: BoardsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let page = ctx
        .client
        .list_boards(ws, &args.project, args.cursor.as_deref(), Some(limit))
        .await?;

    let items: Vec<BoardProjection> = page.items.into_iter().map(BoardProjection::from).collect();

    output::emit_list(
        ctx.output,
        &items,
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
    use crate::cli::Commands;
    use clap::Parser as ClapParser;

    #[derive(ClapParser)]
    struct Cli {
        #[command(subcommand)]
        command: Commands,
    }

    #[test]
    fn boards_list_requires_project() {
        let result = Cli::try_parse_from(["atlas", "boards", "list"]);
        assert!(result.is_err(), "boards list without --project must fail");
    }

    #[test]
    fn boards_list_parses_with_project() {
        let cli = Cli::try_parse_from(["atlas", "boards", "list", "--project", "atlas"]).unwrap();
        let Commands::Boards(args) = cli.command else {
            panic!("expected Boards");
        };
        let BoardsCmd::List(list) = args.command;
        assert_eq!(list.project, "atlas");
    }

    #[test]
    fn boards_list_parses_with_workspace_and_project() {
        let cli = Cli::try_parse_from([
            "atlas",
            "boards",
            "list",
            "--workspace",
            "my-ws",
            "--project",
            "atlas",
        ])
        .unwrap();
        let Commands::Boards(args) = cli.command else {
            panic!("expected Boards");
        };
        let BoardsCmd::List(list) = args.command;
        assert_eq!(list.workspace.as_deref(), Some("my-ws"));
        assert_eq!(list.project, "atlas");
    }

    #[test]
    fn boards_list_limit_clamp_zero_becomes_one() {
        let clamped = 0u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 1);
    }

    #[test]
    fn boards_list_cursor_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "boards",
            "list",
            "--project",
            "atlas",
            "--cursor",
            "abc",
        ])
        .unwrap();
        let Commands::Boards(args) = cli.command else {
            panic!("expected Boards");
        };
        let BoardsCmd::List(list) = args.command;
        assert_eq!(list.cursor.as_deref(), Some("abc"));
    }
}
