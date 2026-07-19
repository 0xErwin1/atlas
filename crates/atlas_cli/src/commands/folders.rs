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

use crate::commands::common::{LIMIT_DEFAULT, LIMIT_MAX, LIMIT_MIN};
use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::FolderProjection;

// ---------------------------------------------------------------------------
// FoldersArgs + FoldersCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `folders` subcommand group.
#[derive(Args)]
pub(crate) struct FoldersArgs {
    #[command(subcommand)]
    pub(crate) command: FoldersCmd,
}

#[derive(Subcommand)]
pub(crate) enum FoldersCmd {
    /// List folders in a project.
    List(FoldersListArgs),
    /// Get a single folder by its UUID.
    Get(FoldersGetArgs),
}

/// Dispatches a parsed `FoldersCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: FoldersCmd) -> Result<(), CliError> {
    match cmd {
        FoldersCmd::List(args) => run_list(ctx, args).await,
        FoldersCmd::Get(args) => run_get(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas folders list`.
#[derive(Parser)]
pub(crate) struct FoldersListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Project slug (required; folders are scoped to a project).
    #[arg(long)]
    pub(crate) project: String,

    /// Maximum number of results (clamped to 1..=200; default 20).
    #[arg(long)]
    pub(crate) limit: Option<u32>,

    /// Pagination cursor returned by a previous list.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
}

async fn run_list(ctx: &Ctx, args: FoldersListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let page = ctx
        .client
        .list_folders(ws, &args.project, args.cursor.as_deref(), Some(limit))
        .await?;

    let items: Vec<FolderProjection> = page.items.into_iter().map(FolderProjection::from).collect();

    output::emit_list(
        ctx.output,
        &items,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// Get
// ---------------------------------------------------------------------------

/// Arguments for `atlas folders get`.
#[derive(Parser)]
pub(crate) struct FoldersGetArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Folder UUID (required).
    #[arg(long)]
    pub(crate) folder_id: Uuid,
}

async fn run_get(ctx: &Ctx, args: FoldersGetArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let folder = ctx.client.get_folder(ws, args.folder_id).await?;
    let proj = FolderProjection::from(folder);
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
    fn folders_list_requires_project() {
        let result = Cli::try_parse_from(["atlas", "folders", "list"]);
        assert!(result.is_err(), "folders list without --project must fail");
    }

    #[test]
    fn folders_list_parses_with_project() {
        let cli = Cli::try_parse_from(["atlas", "folders", "list", "--project", "atlas"]).unwrap();
        if let Commands::Folders(args) = cli.command {
            if let FoldersCmd::List(list) = args.command {
                assert_eq!(list.project, "atlas");
            } else {
                panic!("expected List");
            }
        } else {
            panic!("expected Folders");
        }
    }

    #[test]
    fn folders_get_requires_folder_id() {
        let result = Cli::try_parse_from(["atlas", "folders", "get"]);
        assert!(result.is_err(), "folders get without --folder-id must fail");
    }

    #[test]
    fn folders_get_parses_folder_id() {
        let fid = "550e8400-e29b-41d4-a716-446655440000";
        let cli = Cli::try_parse_from(["atlas", "folders", "get", "--folder-id", fid]).unwrap();
        if let Commands::Folders(args) = cli.command {
            if let FoldersCmd::Get(get) = args.command {
                assert_eq!(get.folder_id.to_string(), fid);
            } else {
                panic!("expected Get");
            }
        } else {
            panic!("expected Folders");
        }
    }

    #[test]
    fn folders_list_limit_clamp_zero_becomes_one() {
        let clamped = 0u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 1);
    }

    #[test]
    fn folders_list_cursor_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "folders",
            "list",
            "--project",
            "atlas",
            "--cursor",
            "abc",
        ])
        .unwrap();
        if let Commands::Folders(args) = cli.command {
            if let FoldersCmd::List(list) = args.command {
                assert_eq!(list.cursor.as_deref(), Some("abc"));
            } else {
                panic!("expected List");
            }
        } else {
            panic!("expected Folders");
        }
    }
}
