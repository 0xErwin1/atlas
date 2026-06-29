#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub(crate) mod obsidian;

use clap::{Args, Subcommand};

use crate::ctx::Ctx;
use crate::error::CliError;

use obsidian::ObsidianImportArgs;

/// Arguments holder for the `import` subcommand group.
#[derive(Args)]
pub(crate) struct ImportArgs {
    #[command(subcommand)]
    pub command: ImportCmd,
}

#[derive(Subcommand)]
pub(crate) enum ImportCmd {
    /// Import an Obsidian vault into an Atlas project.
    Obsidian(ObsidianImportArgs),
}

/// Dispatches a parsed `ImportCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: ImportCmd) -> Result<(), CliError> {
    match cmd {
        ImportCmd::Obsidian(args) => obsidian::run_obsidian(ctx, args).await,
    }
}
