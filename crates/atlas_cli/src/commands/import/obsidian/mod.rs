#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub(crate) mod create;
pub(crate) mod frontmatter;
pub(crate) mod manifest;
pub(crate) mod mapping;
pub(crate) mod parser;
pub(crate) mod plan;

use std::io::{BufRead, Write};
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
/// Flow: (1) resolve workspace; (2) verify the project exists; (3) load the
/// manifest; (4) scan the vault; (5) build the import plan; (6) if `--dry-run`,
/// print the plan and return; (7) if interactive, prompt for confirmation;
/// (8) execute the plan (stub until B1).
pub(crate) async fn run_obsidian(ctx: &Ctx, args: ObsidianImportArgs) -> Result<(), CliError> {
    use atlas_client::ClientError;

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    match ctx.client.get_project(ws, &args.project).await {
        Ok(_) => {}
        Err(ClientError::Api(p)) if p.status == 404 => {
            return Err(CliError::Validation(format!(
                "project '{}' does not exist",
                args.project
            )));
        }
        Err(e) => return Err(CliError::from(e)),
    }

    let manifest_path = args.path.join(".atlas-import.json");
    let mut manifest = manifest::Manifest::load(&manifest_path)?;

    let docs = parser::scan_vault(&args.path)?;
    let import_plan = plan::build_plan(&docs, &manifest);

    if args.dry_run {
        return plan::print_plan(&import_plan, ctx.output);
    }

    if !args.yes {
        eprint!("Import into project '{}' — proceed? [y/N] ", args.project);
        std::io::stderr().flush().ok();
        if !read_yes(std::io::stdin().lock()) {
            eprintln!("Import aborted.");
            return Ok(());
        }
    }

    execute(
        ctx,
        &import_plan,
        &mut manifest,
        ws,
        &args.project,
        &manifest_path,
    )
    .await
}

/// Returns `true` when the user responds with `y` or `Y`.
///
/// Any other input, including a read error or EOF, is treated as a rejection.
/// Factored as a `BufRead`-generic function so the confirmation logic can be
/// tested without a live TTY.
pub(crate) fn read_yes(mut reader: impl BufRead) -> bool {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => false,
        Ok(_) => line.trim().eq_ignore_ascii_case("y"),
        Err(_) => false,
    }
}

/// Executes the import plan against the Atlas API.
///
/// Phase order: folders (depth-first) → documents (create/update/skip) →
/// boards and tasks (B3) → attachments (stub, B4). The manifest is saved
/// inside each phase after every successful operation so a crash at any point
/// leaves a valid, resumable state.
async fn execute(
    ctx: &Ctx,
    plan: &plan::ImportPlan,
    manifest: &mut manifest::Manifest,
    ws: &str,
    project: &str,
    manifest_path: &std::path::Path,
) -> Result<(), CliError> {
    create::execute_folders(
        &ctx.client,
        ws,
        project,
        &plan.folders,
        manifest,
        manifest_path,
    )
    .await?;

    create::execute_documents(
        &ctx.client,
        ws,
        project,
        &plan.documents,
        manifest,
        manifest_path,
        ctx.output,
    )
    .await?;

    create::execute_boards_and_tasks(
        &ctx.client,
        ws,
        project,
        plan,
        manifest,
        manifest_path,
        ctx.output,
    )
    .await?;

    create::execute_attachments().await?;

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

    // -- CLI parsing -----------------------------------------------------------

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

    // -- read_yes --------------------------------------------------------------

    #[test]
    fn read_yes_accepts_lowercase_y() {
        assert!(read_yes("y\n".as_bytes()));
    }

    #[test]
    fn read_yes_accepts_uppercase_y() {
        assert!(read_yes("Y\n".as_bytes()));
    }

    #[test]
    fn read_yes_rejects_n() {
        assert!(!read_yes("n\n".as_bytes()));
    }

    #[test]
    fn read_yes_rejects_empty_line() {
        assert!(!read_yes("\n".as_bytes()));
    }

    #[test]
    fn read_yes_rejects_eof() {
        assert!(!read_yes("".as_bytes()));
    }

    #[test]
    fn read_yes_rejects_yes_spelled_out() {
        assert!(!read_yes("yes\n".as_bytes()));
    }
}
