#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use anyhow::{Context, Result};
use atlas_client::AtlasClient;
use clap::{Parser, Subcommand};

#[cfg(test)]
use clap::CommandFactory;

/// Top-level CLI.
///
/// `--base-url` applies globally; `ATLAS_TOKEN` is picked up from the
/// environment and used as a bearer token for every authenticated request.
#[derive(Parser)]
#[command(name = "atlas", about = "Atlas CLI")]
struct Cli {
    #[arg(long, global = true, default_value = "http://localhost:8080")]
    base_url: String,

    /// Bearer token for authentication. Falls back to the `ATLAS_TOKEN`
    /// environment variable when the flag is not provided.
    #[arg(long, global = true)]
    token: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print the version of the CLI binary.
    Version,
    /// Search across documents and tasks in a workspace.
    Search(SearchArgs),
}

/// Arguments for the `search` subcommand.
#[derive(Parser)]
struct SearchArgs {
    /// Workspace slug to search in.
    #[arg(long)]
    workspace: String,

    /// Query string (required). Supports token filters such as
    /// `status:open`, `tag:rust`, `project:atlas`.
    #[arg(index = 1)]
    query: String,

    /// Filter results by kind: `all` (default), `note`, or `task`.
    #[arg(long, default_value = "all")]
    r#type: String,

    /// Sort order: `relevance` (default) or `updated`.
    #[arg(long, default_value = "relevance")]
    sort: String,

    /// Maximum number of results to return (default 50, clamped to [1, 200]).
    #[arg(long)]
    limit: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut client = AtlasClient::new(&cli.base_url);
    let token = cli
        .token
        .or_else(|| std::env::var("ATLAS_TOKEN").ok());
    if let Some(t) = token {
        client.set_token(t);
    }

    match cli.command {
        Commands::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
        }

        Commands::Search(args) => {
            let type_filter = match args.r#type.as_str() {
                "all" => None,
                other => Some(other.to_string()),
            };

            let page = client
                .search(
                    &args.workspace,
                    &args.query,
                    type_filter.as_deref(),
                    Some(&args.sort),
                    None,
                    args.limit,
                )
                .await
                .context("search request failed")?;

            if page.items.is_empty() {
                println!("No results.");
                return Ok(());
            }

            for hit in &page.items {
                let kind = format!("{:?}", hit.kind).to_lowercase();
                let readable = hit
                    .readable_id
                    .as_deref()
                    .map(|r| format!(" [{r}]"))
                    .unwrap_or_default();

                println!("{} ({kind}{readable}) — {}", hit.id, hit.title);

                if let Some(snippet) = &hit.snippet {
                    let plain = snippet.replace("<mark>", "").replace("</mark>", "");
                    println!("  {plain}");
                }
                println!("  score={:.4}  updated={}", hit.score, hit.updated_at.format("%Y-%m-%d"));
            }

            if page.has_more {
                let next = page
                    .next_cursor
                    .as_deref()
                    .unwrap_or("<cursor missing>");
                println!("\n(More results available; next cursor: {next})");
            }
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

    #[test]
    fn version_subcommand_parses() {
        let cli = Cli::try_parse_from(["atlas", "version"]).unwrap();
        assert!(matches!(cli.command, Commands::Version));
    }

    #[test]
    fn search_subcommand_parses_required_args() {
        let cli =
            Cli::try_parse_from(["atlas", "search", "--workspace", "my-ws", "hello world"])
                .expect("search subcommand with required args must parse");
        if let Commands::Search(args) = cli.command {
            assert_eq!(args.workspace, "my-ws");
            assert_eq!(args.query, "hello world");
            assert_eq!(args.r#type, "all");
            assert_eq!(args.sort, "relevance");
        } else {
            panic!("expected Search command");
        }
    }

    #[test]
    fn search_subcommand_parses_optional_flags() {
        let cli = Cli::try_parse_from([
            "atlas", "search",
            "--workspace", "ws1",
            "--type", "task",
            "--sort", "updated",
            "--limit", "25",
            "my query",
        ])
        .expect("parse");
        if let Commands::Search(args) = cli.command {
            assert_eq!(args.r#type, "task");
            assert_eq!(args.sort, "updated");
            assert_eq!(args.limit, Some(25));
        } else {
            panic!("expected Search command");
        }
    }

    #[test]
    fn token_can_be_set_via_flag() {
        let cli = Cli::try_parse_from([
            "atlas",
            "--token", "my-bearer-token",
            "version",
        ])
        .expect("parse");
        assert_eq!(cli.token.as_deref(), Some("my-bearer-token"));
    }
}
