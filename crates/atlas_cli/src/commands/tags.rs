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
use crate::projections::TagProjection;

// ---------------------------------------------------------------------------
// TagsArgs + TagsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `tags` subcommand group.
#[derive(Args)]
pub(crate) struct TagsArgs {
    #[command(subcommand)]
    pub(crate) command: TagsCmd,
}

#[derive(Subcommand)]
pub(crate) enum TagsCmd {
    /// List all tags in a workspace.
    List(TagsListArgs),
}

/// Dispatches a parsed `TagsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: TagsCmd) -> Result<(), CliError> {
    match cmd {
        TagsCmd::List(args) => run_list(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas tags list`.
#[derive(Parser)]
pub(crate) struct TagsListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_list(ctx: &Ctx, args: TagsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let tags = ctx.client.list_tags(ws).await?;

    let items: Vec<TagProjection> = tags.into_iter().map(TagProjection::from).collect();

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
    fn tags_list_parses_without_required_args() {
        let cli = Cli::try_parse_from(["atlas", "tags", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Tags(_)));
    }

    #[test]
    fn tags_list_parses_with_workspace() {
        let cli = Cli::try_parse_from(["atlas", "tags", "list", "--workspace", "my-ws"]).unwrap();
        let Commands::Tags(args) = cli.command else {
            panic!("expected Tags");
        };
        let TagsCmd::List(list) = args.command;
        assert_eq!(list.workspace.as_deref(), Some("my-ws"));
    }
}
