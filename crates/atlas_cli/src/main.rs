#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod cli;
mod commands;
mod component;
mod config;
mod ctx;
mod error;
mod help_group;
mod output;
mod projections;

use std::io::IsTerminal;
use std::process::ExitCode;

use atlas_client::AtlasClient;
use clap::FromArgMatches;

use cli::Cli;
use ctx::Ctx;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match help_group::build_command()
        .try_get_matches_from(std::env::args_os())
        .and_then(|matches| Cli::from_arg_matches(&matches))
    {
        Ok(c) => c,
        Err(e) => {
            e.print().ok();
            return ExitCode::from(e.exit_code() as u8);
        }
    };

    let file = match config::load() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1u8);
        }
    };

    let r = config::resolve(cli.base_url.as_deref(), cli.token.as_deref(), &file);

    let mut client = AtlasClient::new(&r.base_url);
    if let Some(t) = r.token {
        client.set_token(t);
    }

    let out = output::resolve(cli.json, std::io::stdout().is_terminal());
    let ctx = Ctx {
        client,
        output: out,
        workspace: cli.workspace,
    };

    match commands::dispatch(&ctx, cli.command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            output::report_error(out, &e);
            ExitCode::from(e.exit_code())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use crate::cli::Cli;
    use crate::help_group;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn grouped_command_is_also_a_valid_cli_definition() {
        help_group::build_command().debug_assert();
    }

    /// Proves D1's central claim: the grouped `--help` template changes only
    /// the rendered help text, never the parsed subcommand tree. Both
    /// `bash` and `zsh` completions are generated from the grouped command
    /// and compared byte-for-byte against fixtures captured from the
    /// pre-grouping tree.
    #[test]
    fn subcommand_names_and_order_are_unchanged_by_the_grouped_command() {
        let plain: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|sub| sub.get_name().to_string())
            .collect();
        let grouped: Vec<String> = help_group::build_command()
            .get_subcommands()
            .map(|sub| sub.get_name().to_string())
            .collect();
        assert_eq!(
            plain, grouped,
            "the grouped command must expose the same subcommands, in the same order"
        );
    }

    #[test]
    fn bash_completions_are_byte_identical_to_the_pre_grouping_snapshot() {
        let mut buf: Vec<u8> = Vec::new();
        clap_complete::generate(
            clap_complete::Shell::Bash,
            &mut help_group::build_command(),
            "atlas",
            &mut buf,
        );
        let fixture: &[u8] = include_bytes!("../tests/fixtures/completions_bash.snapshot");
        assert_eq!(
            buf, fixture,
            "bash completions must stay byte-identical: help_template changes rendered help \
             text only, never the clap subcommand tree"
        );
    }

    #[test]
    fn zsh_completions_are_byte_identical_to_the_pre_grouping_snapshot() {
        let mut buf: Vec<u8> = Vec::new();
        clap_complete::generate(
            clap_complete::Shell::Zsh,
            &mut help_group::build_command(),
            "atlas",
            &mut buf,
        );
        let fixture: &[u8] = include_bytes!("../tests/fixtures/completions_zsh.snapshot");
        assert_eq!(
            buf, fixture,
            "zsh completions must stay byte-identical: help_template changes rendered help \
             text only, never the clap subcommand tree"
        );
    }

    #[test]
    fn no_hide_attribute_exists_anywhere_in_cli_rs() {
        let source = include_str!("cli.rs");
        assert!(
            !source.contains("hide = true") && !source.contains("hide(true)"),
            "INV-NO-CLI-RENAME: no subcommand may be hidden"
        );
    }
}
