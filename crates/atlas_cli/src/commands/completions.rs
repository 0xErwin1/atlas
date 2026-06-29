#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use clap::{Args, CommandFactory, ValueEnum};

use crate::cli::Cli;
use crate::ctx::Ctx;
use crate::error::CliError;

/// Supported shell targets for completion generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum Shell {
    Bash,
    Elvish,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
    Zsh,
}

impl From<Shell> for clap_complete::Shell {
    fn from(s: Shell) -> Self {
        match s {
            Shell::Bash => clap_complete::Shell::Bash,
            Shell::Elvish => clap_complete::Shell::Elvish,
            Shell::Fish => clap_complete::Shell::Fish,
            Shell::PowerShell => clap_complete::Shell::PowerShell,
            Shell::Zsh => clap_complete::Shell::Zsh,
        }
    }
}

/// Arguments for `atlas completions`.
#[derive(Args)]
pub(crate) struct CompletionsArgs {
    /// Shell to generate completions for: `bash`, `elvish`, `fish`,
    /// `powershell`, or `zsh`.
    #[arg(value_enum, index = 1)]
    pub(crate) shell: Shell,
}

/// Generates shell completions for `shell` into `writer`.
///
/// Factored out so tests can capture output into a `Vec<u8>` without a server.
pub(crate) fn generate<W: std::io::Write>(shell: Shell, writer: &mut W) {
    clap_complete::generate(
        clap_complete::Shell::from(shell),
        &mut Cli::command(),
        "atlas",
        writer,
    );
}

/// Writes completions for the requested shell to stdout and returns `Ok(())`.
pub(crate) async fn run(_ctx: &Ctx, args: CompletionsArgs) -> Result<(), CliError> {
    generate(args.shell, &mut std::io::stdout());
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    // T64: Parse tests

    #[test]
    fn completions_bash_parses() {
        let cli = Cli::try_parse_from(["atlas", "completions", "bash"]).unwrap();
        assert!(
            matches!(cli.command, Commands::Completions(_)),
            "expected Completions command"
        );
    }

    #[test]
    fn completions_zsh_parses_shell_variant() {
        let cli = Cli::try_parse_from(["atlas", "completions", "zsh"]).unwrap();
        if let Commands::Completions(args) = cli.command {
            assert_eq!(args.shell, Shell::Zsh);
        } else {
            panic!("expected Completions");
        }
    }

    #[test]
    fn completions_powershell_parses() {
        let cli = Cli::try_parse_from(["atlas", "completions", "powershell"]).unwrap();
        if let Commands::Completions(args) = cli.command {
            assert_eq!(args.shell, Shell::PowerShell);
        } else {
            panic!("expected Completions");
        }
    }

    #[test]
    fn completions_requires_shell_argument() {
        let result = Cli::try_parse_from(["atlas", "completions"]);
        assert!(result.is_err(), "shell argument is required");
    }

    #[test]
    fn completions_invalid_shell_fails_at_parse() {
        let result = Cli::try_parse_from(["atlas", "completions", "invalid-shell"]);
        assert!(
            result.is_err(),
            "invalid shell must fail at parse time (exit 2)"
        );
    }

    // T65: Output is non-empty (no server required)

    #[test]
    fn bash_completions_produce_non_empty_output() {
        let mut buf: Vec<u8> = Vec::new();
        generate(Shell::Bash, &mut buf);
        assert!(
            !buf.is_empty(),
            "bash completions must produce non-empty output"
        );
    }

    #[test]
    fn zsh_completions_produce_non_empty_output() {
        let mut buf: Vec<u8> = Vec::new();
        generate(Shell::Zsh, &mut buf);
        assert!(
            !buf.is_empty(),
            "zsh completions must produce non-empty output"
        );
    }

    #[test]
    fn fish_completions_produce_non_empty_output() {
        let mut buf: Vec<u8> = Vec::new();
        generate(Shell::Fish, &mut buf);
        assert!(
            !buf.is_empty(),
            "fish completions must produce non-empty output"
        );
    }
}
