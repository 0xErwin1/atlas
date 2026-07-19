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
use crate::projections::ProjectProjection;

// ---------------------------------------------------------------------------
// ProjectsArgs + ProjectsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `projects` subcommand group.
#[derive(Args)]
pub(crate) struct ProjectsArgs {
    #[command(subcommand)]
    pub(crate) command: ProjectsCmd,
}

#[derive(Subcommand)]
pub(crate) enum ProjectsCmd {
    /// List projects in a workspace.
    List(ProjectsListArgs),
    /// Get a single project by its slug.
    Get(ProjectsGetArgs),
}

/// Dispatches a parsed `ProjectsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: ProjectsCmd) -> Result<(), CliError> {
    match cmd {
        ProjectsCmd::List(args) => run_list(ctx, args).await,
        ProjectsCmd::Get(args) => run_get(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas projects list`.
#[derive(Parser)]
pub(crate) struct ProjectsListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Maximum number of results (clamped to 1..=200; default 20).
    #[arg(long)]
    pub(crate) limit: Option<u32>,

    /// Pagination cursor returned by a previous list.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
}

async fn run_list(ctx: &Ctx, args: ProjectsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let page = ctx
        .client
        .list_projects(ws, args.cursor.as_deref(), Some(limit))
        .await?;

    let items: Vec<ProjectProjection> = page
        .items
        .into_iter()
        .map(ProjectProjection::from)
        .collect();

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

/// Arguments for `atlas projects get`.
#[derive(Parser)]
pub(crate) struct ProjectsGetArgs {
    /// Project slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_get(ctx: &Ctx, args: ProjectsGetArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let dto = ctx.client.get_project(ws, &args.slug).await?;
    let proj = ProjectProjection::from(dto);
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
    fn projects_list_parses_without_required_args() {
        let cli = Cli::try_parse_from(["atlas", "projects", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Projects(_)));
    }

    #[test]
    fn projects_list_parses_with_workspace() {
        let cli =
            Cli::try_parse_from(["atlas", "projects", "list", "--workspace", "my-ws"]).unwrap();
        if let Commands::Projects(args) = cli.command {
            if let ProjectsCmd::List(list) = args.command {
                assert_eq!(list.workspace.as_deref(), Some("my-ws"));
            } else {
                panic!("expected List");
            }
        } else {
            panic!("expected Projects");
        }
    }

    #[test]
    fn projects_list_cursor_parses() {
        let cli = Cli::try_parse_from(["atlas", "projects", "list", "--cursor", "abc"]).unwrap();
        if let Commands::Projects(args) = cli.command {
            if let ProjectsCmd::List(list) = args.command {
                assert_eq!(list.cursor.as_deref(), Some("abc"));
            } else {
                panic!("expected List");
            }
        } else {
            panic!("expected Projects");
        }
    }

    #[test]
    fn projects_list_limit_clamp_zero_becomes_one() {
        let clamped = 0u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 1);
    }

    #[test]
    fn projects_list_limit_clamp_over_max_becomes_200() {
        let clamped = 9999u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 200);
    }

    #[test]
    fn projects_get_parses_slug() {
        let cli = Cli::try_parse_from(["atlas", "projects", "get", "atlas"]).unwrap();
        if let Commands::Projects(args) = cli.command {
            if let ProjectsCmd::Get(get) = args.command {
                assert_eq!(get.slug, "atlas");
            } else {
                panic!("expected Get");
            }
        } else {
            panic!("expected Projects");
        }
    }

    #[test]
    fn projects_get_requires_slug() {
        let result = Cli::try_parse_from(["atlas", "projects", "get"]);
        assert!(result.is_err(), "get without slug must fail");
    }
}
