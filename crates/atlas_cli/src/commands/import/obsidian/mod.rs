#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub(crate) mod frontmatter;
pub(crate) mod parser;

use std::path::PathBuf;

use clap::Parser;

use crate::ctx::Ctx;
use crate::error::CliError;

/// Arguments for `atlas import obsidian`.
#[derive(Parser)]
pub(crate) struct ObsidianImportArgs {
    /// Workspace slug (uses the configured default when omitted).
    #[arg(long)]
    pub workspace: Option<String>,

    /// Target project slug (must already exist).
    #[arg(long)]
    pub project: String,

    /// Path to the Obsidian vault root directory.
    #[arg(index = 1)]
    pub path: PathBuf,

    /// Preview what would be imported without making any changes.
    #[arg(long)]
    pub dry_run: bool,

    /// Skip the confirmation prompt before mutating.
    #[arg(long)]
    pub yes: bool,
}

/// Entry point for `atlas import obsidian`.
///
/// Stub — full implementation lands in Batch B0b once the pure scan/plan layer
/// (B0a) is complete.
pub(crate) async fn run_obsidian(_ctx: &Ctx, _args: ObsidianImportArgs) -> Result<(), CliError> {
    Ok(())
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
    fn obsidian_import_parses_required_args() {
        let cli = Cli::try_parse_from([
            "atlas",
            "import",
            "obsidian",
            "--project",
            "my-project",
            "/tmp/vault",
        ])
        .unwrap();
        let Commands::Import(args) = cli.command else {
            panic!("expected Import command");
        };
        let super::super::ImportCmd::Obsidian(obs) = args.command;
        assert_eq!(obs.project, "my-project");
        assert_eq!(obs.path, PathBuf::from("/tmp/vault"));
        assert!(!obs.dry_run);
        assert!(!obs.yes);
    }

    #[test]
    fn obsidian_import_parses_optional_flags() {
        let cli = Cli::try_parse_from([
            "atlas",
            "import",
            "obsidian",
            "--workspace",
            "ws",
            "--project",
            "p",
            "--dry-run",
            "--yes",
            "/vault",
        ])
        .unwrap();
        let Commands::Import(args) = cli.command else {
            panic!("expected Import command");
        };
        let super::super::ImportCmd::Obsidian(obs) = args.command;
        assert_eq!(obs.workspace.as_deref(), Some("ws"));
        assert!(obs.dry_run);
        assert!(obs.yes);
    }

    #[test]
    fn obsidian_import_requires_project() {
        let result = Cli::try_parse_from(["atlas", "import", "obsidian", "/tmp/vault"]);
        assert!(
            result.is_err(),
            "import obsidian without --project must fail"
        );
    }
}
