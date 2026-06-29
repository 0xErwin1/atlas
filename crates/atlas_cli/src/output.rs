#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use comfy_table::{Table, presets::UTF8_BORDERS_ONLY};
use serde::Serialize;

use crate::error::CliError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
}

/// Selects the output format from the `--json` flag and TTY state.
///
/// Json is chosen when `json_flag` is set OR when stdout is not an interactive
/// TTY, so piped output is always machine-parseable by default.
pub(crate) fn resolve(json_flag: bool, stdout_is_tty: bool) -> OutputFormat {
    if json_flag || !stdout_is_tty {
        OutputFormat::Json
    } else {
        OutputFormat::Human
    }
}

/// Serializes `v` as pretty JSON to stdout.
pub(crate) fn print_json<T: Serialize>(v: &T) -> Result<(), CliError> {
    serde_json::to_writer_pretty(std::io::stdout(), v)
        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    println!();
    Ok(())
}

/// Renders a table with column `headers` and dynamic `rows` to stdout.
pub(crate) fn print_table(headers: &[&str], rows: Vec<Vec<String>>) -> Result<(), CliError> {
    let mut table = Table::new();
    table
        .load_preset(UTF8_BORDERS_ONLY)
        .set_header(headers.iter().map(|h| h.to_string()))
        .add_rows(rows);

    println!("{table}");
    Ok(())
}

/// A type that knows how to render itself as a table row.
pub(crate) trait TableRow: Serialize {
    fn headers() -> &'static [&'static str];
    fn row(&self) -> Vec<String>;
}

/// Emits a single item in the active output format.
// Used from B2 onwards by per-domain get commands; forward-declared here.
#[allow(dead_code)]
pub(crate) fn emit<T: TableRow>(out: OutputFormat, v: &T) -> Result<(), CliError> {
    match out {
        OutputFormat::Json => print_json(v),
        OutputFormat::Human => print_table(T::headers(), vec![v.row()]),
    }
}

/// Emits a list of items plus pagination metadata in the active output format.
///
/// In Human mode, renders a table followed by a cursor hint when more pages
/// are available. In Json mode, wraps items in the standard envelope.
pub(crate) fn emit_list<T: TableRow>(
    out: OutputFormat,
    items: &[T],
    next_cursor: Option<&str>,
    has_more: bool,
) -> Result<(), CliError> {
    match out {
        OutputFormat::Json => {
            let envelope = crate::projections::Envelope {
                items: items.iter().collect::<Vec<_>>(),
                next_cursor: next_cursor.map(str::to_owned),
                has_more,
            };
            print_json(&envelope)
        }

        OutputFormat::Human => {
            if items.is_empty() {
                println!("No results.");
                return Ok(());
            }

            let rows = items.iter().map(|item| item.row()).collect();
            print_table(T::headers(), rows)?;

            if has_more {
                let cursor = next_cursor.unwrap_or("<cursor missing>");
                println!("\n(More results available — next cursor: {cursor})");
            }

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // 4-case truth table for OutputFormat::resolve
    #[test]
    fn json_flag_true_tty_true_yields_json() {
        assert_eq!(resolve(true, true), OutputFormat::Json);
    }

    #[test]
    fn json_flag_true_tty_false_yields_json() {
        assert_eq!(resolve(true, false), OutputFormat::Json);
    }

    #[test]
    fn json_flag_false_tty_false_yields_json() {
        assert_eq!(resolve(false, false), OutputFormat::Json);
    }

    #[test]
    fn json_flag_false_tty_true_yields_human() {
        assert_eq!(resolve(false, true), OutputFormat::Human);
    }
}
