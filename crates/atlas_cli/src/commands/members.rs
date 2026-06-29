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
use crate::projections::MemberProjection;

// ---------------------------------------------------------------------------
// MembersArgs + MembersCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `members` subcommand group.
#[derive(Args)]
pub(crate) struct MembersArgs {
    #[command(subcommand)]
    pub(crate) command: MembersCmd,
}

#[derive(Subcommand)]
pub(crate) enum MembersCmd {
    /// List all members (users and API-key principals) in a workspace.
    List(MembersListArgs),
}

/// Dispatches a parsed `MembersCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: MembersCmd) -> Result<(), CliError> {
    match cmd {
        MembersCmd::List(args) => run_list(ctx, args).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas members list`.
#[derive(Parser)]
pub(crate) struct MembersListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_list(ctx: &Ctx, args: MembersListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let members = ctx.client.list_workspace_members(ws).await?;

    let items: Vec<MemberProjection> = members.into_iter().map(MemberProjection::from).collect();

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
    fn members_list_parses_without_required_args() {
        let cli = Cli::try_parse_from(["atlas", "members", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Members(_)));
    }

    #[test]
    fn members_list_parses_with_workspace() {
        let cli =
            Cli::try_parse_from(["atlas", "members", "list", "--workspace", "my-ws"]).unwrap();
        let Commands::Members(args) = cli.command else {
            panic!("expected Members");
        };
        let MembersCmd::List(list) = args.command;
        assert_eq!(list.workspace.as_deref(), Some("my-ws"));
    }
}
