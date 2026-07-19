#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use crate::cli::SearchArgs;
use crate::commands::common::{LIMIT_DEFAULT, LIMIT_MAX, LIMIT_MIN};
use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::SearchHitProjection;

/// Executes the `search` command.
///
/// Resolves the workspace from the per-command flag or the global context,
/// clamps the limit to `1..=200`, then emits results through the output layer.
pub(crate) async fn run(ctx: &Ctx, args: SearchArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let type_filter = match args.r#type.as_str() {
        "all" => None,
        other => Some(other.to_string()),
    };

    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let page = ctx
        .client
        .search(
            ws,
            &args.query,
            type_filter.as_deref(),
            Some(&args.sort),
            args.cursor.as_deref(),
            Some(limit),
        )
        .await?;

    let projections: Vec<SearchHitProjection> = page
        .items
        .into_iter()
        .map(SearchHitProjection::from)
        .collect();

    output::emit_list(
        ctx.output,
        &projections,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_clamp_zero_becomes_one() {
        let clamped = 0u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 1);
    }

    #[test]
    fn limit_clamp_over_max_becomes_200() {
        let clamped = 9999u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 200);
    }

    #[test]
    fn limit_clamp_within_range_unchanged() {
        let clamped = 50u32.clamp(LIMIT_MIN, LIMIT_MAX);
        assert_eq!(clamped, 50);
    }
}
