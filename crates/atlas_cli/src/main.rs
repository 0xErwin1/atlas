#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use anyhow::Result;
use atlas_client::AtlasClient;
use clap::{Parser, Subcommand};

#[cfg(test)]
use clap::CommandFactory;

#[derive(Parser)]
#[command(name = "atlas", about = "Atlas CLI")]
struct Cli {
    #[arg(long, global = true, default_value = "http://localhost:8080")]
    base_url: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Version,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let _client = AtlasClient::new(&cli.base_url);

    match cli.command {
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
