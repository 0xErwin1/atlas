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

use atlas_client::helpers;

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::ColumnProjection;

// ---------------------------------------------------------------------------
// ColumnsArgs + ColumnsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `columns` subcommand group.
#[derive(Args)]
pub(crate) struct ColumnsArgs {
    #[command(subcommand)]
    pub(crate) command: ColumnsCmd,
}

#[derive(Subcommand)]
pub(crate) enum ColumnsCmd {
    /// List columns on a board.
    List(ColumnsListArgs),
}

/// Dispatches a parsed `ColumnsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: ColumnsCmd) -> Result<(), CliError> {
    match cmd {
        ColumnsCmd::List(args) => run_list(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas columns list`.
#[derive(Parser)]
pub(crate) struct ColumnsListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Board name or UUID (required). A UUID passes through unchanged; a name
    /// is resolved by case-insensitive substring match across all boards in the
    /// workspace.
    #[arg(long)]
    pub(crate) board: String,
}

async fn run_list(ctx: &Ctx, args: ColumnsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let board_id_str = helpers::resolve_board_id(&ctx.client, ws, &args.board).await?;

    let board_uuid = Uuid::parse_str(&board_id_str).map_err(|_| {
        CliError::Validation(format!(
            "resolved board ID '{board_id_str}' is not a valid UUID"
        ))
    })?;

    let cols = ctx.client.list_columns(ws, board_uuid).await?;

    let items: Vec<ColumnProjection> = cols.into_iter().map(ColumnProjection::from).collect();

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
    fn columns_list_without_board_fails() {
        let result = Cli::try_parse_from(["atlas", "columns", "list"]);
        assert!(result.is_err(), "columns list without --board must fail");
    }

    #[test]
    fn columns_list_with_board_uuid_parses() {
        let board_uuid = "550e8400-e29b-41d4-a716-446655440000";
        let cli = Cli::try_parse_from(["atlas", "columns", "list", "--board", board_uuid]).unwrap();
        let Commands::Columns(args) = cli.command else {
            panic!("expected Columns");
        };
        let ColumnsCmd::List(list) = args.command;
        assert_eq!(list.board, board_uuid);
    }

    #[test]
    fn columns_list_with_board_name_parses() {
        let cli =
            Cli::try_parse_from(["atlas", "columns", "list", "--board", "Dev Board"]).unwrap();
        let Commands::Columns(args) = cli.command else {
            panic!("expected Columns");
        };
        let ColumnsCmd::List(list) = args.command;
        assert_eq!(list.board, "Dev Board");
    }
}
