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

use obsidian::ObsidianExportArgs;

/// Arguments holder for the `export` subcommand group.
#[derive(Args)]
pub(crate) struct ExportArgs {
    #[command(subcommand)]
    pub command: ExportCmd,
}

#[derive(Subcommand)]
pub(crate) enum ExportCmd {
    /// Export an Atlas project as an Obsidian vault.
    Obsidian(ObsidianExportArgs),
}

/// Dispatches a parsed `ExportCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: ExportCmd) -> Result<(), CliError> {
    match cmd {
        ExportCmd::Obsidian(args) => obsidian::run_obsidian(ctx, args).await,
    }
}
