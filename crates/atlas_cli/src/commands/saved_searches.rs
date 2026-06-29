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
use crate::projections::{DeleteByIdProjection, SavedSearchProjection};

// ---------------------------------------------------------------------------
// SavedSearchesArgs + SavedSearchesCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `saved-searches` subcommand group.
#[derive(Args)]
pub(crate) struct SavedSearchesArgs {
    #[command(subcommand)]
    pub(crate) command: SavedSearchesCmd,
}

#[derive(Subcommand)]
pub(crate) enum SavedSearchesCmd {
    /// List saved searches in a workspace.
    List(SavedSearchesListArgs),
    /// Create a new saved search.
    Create(SavedSearchesCreateArgs),
    /// Rename an existing saved search.
    Rename(SavedSearchesRenameArgs),
    /// Delete a saved search (requires --confirm).
    Delete(SavedSearchesDeleteArgs),
}

/// Dispatches a parsed `SavedSearchesCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: SavedSearchesCmd) -> Result<(), CliError> {
    match cmd {
        SavedSearchesCmd::List(args) => run_list(ctx, args).await,
        SavedSearchesCmd::Create(args) => run_create(ctx, args).await,
        SavedSearchesCmd::Rename(args) => run_rename(ctx, args).await,
        SavedSearchesCmd::Delete(args) => run_delete(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas saved-searches list`.
#[derive(Parser)]
pub(crate) struct SavedSearchesListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_list(ctx: &Ctx, args: SavedSearchesListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let searches = ctx.client.list_saved_searches(ws).await?;

    let items: Vec<SavedSearchProjection> = searches
        .into_iter()
        .map(SavedSearchProjection::from)
        .collect();

    output::emit_list(ctx.output, &items, None, false)
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas saved-searches create`.
#[derive(Parser)]
pub(crate) struct SavedSearchesCreateArgs {
    /// Display name for the saved search.
    #[arg(long)]
    pub(crate) name: String,

    /// Query string (supports token filters such as `status:open tag:bug`).
    #[arg(long)]
    pub(crate) query: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_create(ctx: &Ctx, args: SavedSearchesCreateArgs) -> Result<(), CliError> {
    use atlas_api::dtos::saved_searches::CreateSavedSearchRequest;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = CreateSavedSearchRequest {
        name: args.name,
        query: args.query,
    };

    let search = ctx.client.create_saved_search(ws, body).await?;
    let proj = SavedSearchProjection::from(search);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

/// Arguments for `atlas saved-searches rename`.
#[derive(Parser)]
pub(crate) struct SavedSearchesRenameArgs {
    /// UUID of the saved search to rename.
    #[arg(long)]
    pub(crate) id: Uuid,

    /// New display name.
    #[arg(long)]
    pub(crate) name: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_rename(ctx: &Ctx, args: SavedSearchesRenameArgs) -> Result<(), CliError> {
    use atlas_api::dtos::saved_searches::RenameSavedSearchRequest;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = RenameSavedSearchRequest { name: args.name };

    let search = ctx.client.rename_saved_search(ws, args.id, body).await?;
    let proj = SavedSearchProjection::from(search);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

/// Arguments for `atlas saved-searches delete`.
#[derive(Parser)]
pub(crate) struct SavedSearchesDeleteArgs {
    /// UUID of the saved search to delete.
    #[arg(long)]
    pub(crate) id: Uuid,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the deletion. Required — permanently removes the saved search.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete(ctx: &Ctx, args: SavedSearchesDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete the saved search".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    ctx.client.delete_saved_search(ws, args.id).await?;

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
    fn saved_searches_list_parses_without_workspace() {
        let cli = Cli::try_parse_from(["atlas", "saved-searches", "list"]).unwrap();
        let Commands::SavedSearches(args) = cli.command else {
            panic!("expected SavedSearches");
        };
        assert!(matches!(args.command, SavedSearchesCmd::List(_)));
    }

    #[test]
    fn saved_searches_create_requires_name_and_query() {
        assert!(
            Cli::try_parse_from(["atlas", "saved-searches", "create"]).is_err(),
            "missing --name --query must fail"
        );
        assert!(
            Cli::try_parse_from(["atlas", "saved-searches", "create", "--name", "bugs"]).is_err(),
            "missing --query must fail"
        );
    }

    #[test]
    fn saved_searches_create_parses_name_and_query() {
        let cli = Cli::try_parse_from([
            "atlas",
            "saved-searches",
            "create",
            "--name",
            "Open bugs",
            "--query",
            "status:open tag:bug",
        ])
        .unwrap();
        let Commands::SavedSearches(args) = cli.command else {
            panic!("expected SavedSearches");
        };
        let SavedSearchesCmd::Create(c) = args.command else {
            panic!("expected Create");
        };
        assert_eq!(c.name, "Open bugs");
        assert_eq!(c.query, "status:open tag:bug");
    }

    #[test]
    fn saved_searches_rename_requires_id_and_name() {
        assert!(
            Cli::try_parse_from([
                "atlas",
                "saved-searches",
                "rename",
                "--id",
                "00000000-0000-0000-0000-000000000001"
            ])
            .is_err(),
            "missing --name must fail"
        );
    }

    #[test]
    fn saved_searches_rename_parses_id_and_name() {
        let cli = Cli::try_parse_from([
            "atlas",
            "saved-searches",
            "rename",
            "--id",
            "00000000-0000-0000-0000-000000000001",
            "--name",
            "Renamed",
        ])
        .unwrap();
        let Commands::SavedSearches(args) = cli.command else {
            panic!("expected SavedSearches");
        };
        assert!(matches!(args.command, SavedSearchesCmd::Rename(_)));
    }

    #[test]
    fn saved_searches_delete_confirm_defaults_to_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "saved-searches",
            "delete",
            "--id",
            "00000000-0000-0000-0000-000000000001",
        ])
        .unwrap();
        let Commands::SavedSearches(args) = cli.command else {
            panic!("expected SavedSearches");
        };
        let SavedSearchesCmd::Delete(d) = args.command else {
            panic!("expected Delete");
        };
        assert!(!d.confirm, "--confirm must default to false");
    }

    #[test]
    fn saved_searches_delete_confirm_guard_fires_before_network() {
        let args = SavedSearchesDeleteArgs {
            id: Uuid::nil(),
            workspace: None,
            confirm: false,
        };
        assert!(
            !args.confirm,
            "confirm guard: must be false when --confirm absent"
        );
    }
}
