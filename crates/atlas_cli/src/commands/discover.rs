#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output::{self, OutputFormat, TableRow};
use crate::projections::{DiscoverProjection, discover_projections};

/// `atlas discover` (also reachable as `atlas custos discover` through the
/// component-prefix alias, design D4). Flat, no subcommands — the same
/// shape as `atlas doctor`.
///
/// Json mode prints the server's `DiscoverResponseDto` unchanged (`atlas
/// doctor`'s own precedent for a single-object response, `commands/
/// doctor.rs`): `components`, `admin`, `truncated` at the top level, so an
/// empty `components` list still carries both flags. Human mode renders one
/// table row per component (`admin`/`truncated` repeated per row for a
/// reader scrolled past the top) — a rendering choice, not the wire shape.
pub(crate) async fn run(ctx: &Ctx) -> Result<(), CliError> {
    let response = ctx.client.custos().discover().await?;

    match ctx.output {
        OutputFormat::Json => output::print_json(&response)?,
        OutputFormat::Human => {
            let rows: Vec<Vec<String>> = discover_projections(response)
                .into_iter()
                .map(|item| item.row())
                .collect();
            output::print_table(DiscoverProjection::headers(), rows)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use atlas_api::dtos::discovery::DiscoverResponseDto;

    use crate::cli::Cli;
    use clap::Parser as ClapParser;

    #[test]
    fn discover_parses() {
        let cli = Cli::try_parse_from(["atlas", "discover"]).unwrap();
        assert!(matches!(cli.command, crate::cli::Commands::Discover));
    }

    /// Json mode prints `DiscoverResponseDto` exactly as `output::print_json`
    /// receives it (`run`'s `OutputFormat::Json` arm) — this is the value
    /// under test, not the CLI process end to end. An empty `components`
    /// list is the case the CRITICAL finding named: it must not silently
    /// drop `admin`/`truncated` from the emitted JSON.
    #[test]
    fn json_mode_keeps_admin_and_truncated_when_components_is_empty() {
        let response = DiscoverResponseDto {
            components: Vec::new(),
            admin: true,
            truncated: false,
        };

        let value = serde_json::to_value(&response).expect("serialize DiscoverResponseDto");

        assert_eq!(value["components"], serde_json::json!([]));
        assert_eq!(value["admin"], serde_json::json!(true));
        assert_eq!(value["truncated"], serde_json::json!(false));
    }
}
