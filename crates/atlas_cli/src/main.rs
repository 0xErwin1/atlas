#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod cli;
mod commands;
mod config;
mod ctx;
mod error;
mod output;
mod projections;

use std::io::IsTerminal;
use std::process::ExitCode;

use atlas_client::AtlasClient;
use clap::Parser;

use cli::Cli;
use ctx::Ctx;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
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

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
