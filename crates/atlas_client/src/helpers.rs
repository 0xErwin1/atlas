//! Shared board/column resolution helpers and field validators.
//!
//! Free functions that can be called from both `atlas_mcp` and `atlas_cli`
//! without either crate depending on the other. Async resolvers accept a bare
//! `&AtlasClient`, keeping the contract narrow and free of MCP or CLI state.

use std::fmt;

use atlas_api::dtos::boards_tasks::ColumnDto;

use crate::{AtlasClient, ClientError};

// ---------------------------------------------------------------------------
// ResolverError
// ---------------------------------------------------------------------------

/// Typed error for board/column resolution failures.
///
/// `Display` output is byte-identical to the strings `atlas_mcp` previously
/// returned as plain `String` errors, so `.to_string()` adapters preserve
/// existing behavior at every call site.
#[derive(Debug)]
pub enum ResolverError {
    /// No board matching the supplied name or UUID was found.
    BoardNotFound {
        board_ref: String,
        workspace: String,
    },
    /// A resolved board_id string did not parse as a valid UUID.
    InvalidBoardUuid { board_id: String },
    /// An underlying Atlas client call failed; `context` names the operation.
    Client {
        context: String,
        source: ClientError,
    },
}

impl fmt::Display for ResolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BoardNotFound {
                board_ref,
                workspace,
            } => write!(
                f,
                "no board matching '{board_ref}' found in workspace '{workspace}'"
            ),
            Self::InvalidBoardUuid { board_id } => {
                write!(f, "resolved board_id '{board_id}' is not a valid UUID")
            }
            Self::Client { context, source } => write!(f, "{context}: {source}"),
        }
    }
}

impl std::error::Error for ResolverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client { source, .. } => Some(source),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// CSV / list param parsing
// ---------------------------------------------------------------------------

/// Splits a comma-separated string into a trimmed, non-empty `Vec<String>`.
///
/// Empty input or whitespace-only input yields an empty vec.
/// Individual items that are blank after trimming are skipped.
pub fn parse_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(String::from)
        .collect()
}

// ---------------------------------------------------------------------------
// Column-name helpers
// ---------------------------------------------------------------------------

/// Returns the UUIDs of all columns whose name contains `name_fragment` as a
/// case-insensitive substring.
///
/// Returns an empty vec when no column matches — callers should propagate this
/// as an empty result rather than an error. Multi-match is intentional: a
/// workspace may have multiple boards each with a column called "To Do"; all
/// matching UUIDs are returned so the filter covers all of them.
pub fn match_columns_by_name(name_fragment: &str, cols: &[ColumnDto]) -> Vec<String> {
    let needle = name_fragment.to_ascii_lowercase();
    cols.iter()
        .filter(|col| col.name.to_ascii_lowercase().contains(&needle))
        .map(|col| col.id.to_string())
        .collect()
}

/// Resolves a column name to exactly one UUID on a given board.
///
/// Unlike `match_columns_by_name` (which returns all fuzzy matches for read
/// filters), this function enforces single-match semantics required by write
/// operations: 0 or >1 matches are both errors that include the board's full
/// column list so the caller can correct the name immediately.
pub fn resolve_column_id_on_board(name: &str, cols: &[ColumnDto]) -> Result<uuid::Uuid, String> {
    let needle = name.to_ascii_lowercase();
    let matches: Vec<&ColumnDto> = cols
        .iter()
        .filter(|c| c.name.to_ascii_lowercase().contains(&needle))
        .collect();

    let available: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
    let available_list = available.join(", ");

    match matches.as_slice() {
        [] => Err(format!(
            "column '{name}' not found on this board; available columns: [{available_list}]"
        )),
        [single] => Ok(single.id),
        many => {
            let matched_names: Vec<&str> = many.iter().map(|c| c.name.as_str()).collect();
            Err(format!(
                "column '{name}' is ambiguous; matches: [{}]; pass a more specific name",
                matched_names.join(", ")
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Write-side: enum validators
// ---------------------------------------------------------------------------

/// Validates a task priority string.
///
/// Returns `Ok(())` for accepted values, or an `Err` listing the valid set.
pub fn validate_priority(v: &str) -> Result<(), String> {
    match v {
        "low" | "medium" | "high" | "urgent" => Ok(()),
        _ => Err(format!(
            "invalid priority '{v}'; valid values: low, medium, high, urgent"
        )),
    }
}

/// Validates a task assignee type.
///
/// Returns `Ok(())` for accepted values, or an `Err` listing the valid set.
pub fn validate_assignee_type(v: &str) -> Result<(), String> {
    match v {
        "user" | "api_key" => Ok(()),
        _ => Err(format!(
            "invalid assignee_type '{v}'; valid values: user, api_key"
        )),
    }
}

/// Validates a task reference kind.
///
/// Returns `Ok(())` for accepted values, or an `Err` listing the valid set.
pub fn validate_reference_kind(v: &str) -> Result<(), String> {
    match v {
        "relates" | "blocks" | "parent" | "spec" | "docs" => Ok(()),
        _ => Err(format!(
            "invalid kind '{v}'; valid values: relates, blocks, parent, spec, docs"
        )),
    }
}

/// Validates that an estimate value is non-negative.
///
/// Accepts any `i32 >= 0`. Returns an actionable error string for negative values.
pub fn validate_estimate(v: i32) -> Result<(), String> {
    if v < 0 {
        return Err(format!(
            "invalid estimate '{v}': must be a non-negative integer"
        ));
    }
    Ok(())
}

/// Validates an estimate carried as a `serde_json::Value` (used in PATCH paths).
///
/// Null (clear) and absent pass through unchecked. Only a negative numeric
/// value is rejected.
pub fn validate_estimate_value(v: &serde_json::Value) -> Result<(), String> {
    if let serde_json::Value::Number(n) = v
        && n.as_i64().is_some_and(|i| i < 0)
    {
        return Err(format!(
            "invalid estimate '{n}': must be a non-negative integer"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Async board/column resolvers
// ---------------------------------------------------------------------------

/// Resolves a board reference (name fragment or UUID string) to a UUID string.
///
/// When the input parses as a UUID it is returned directly without any network
/// call. Otherwise walks all projects' boards to find the first
/// case-insensitive partial name match.
pub async fn resolve_board_id(
    client: &AtlasClient,
    ws: &str,
    board_ref: &str,
) -> Result<String, ResolverError> {
    if uuid::Uuid::parse_str(board_ref).is_ok() {
        return Ok(board_ref.to_string());
    }

    let needle = board_ref.to_ascii_lowercase();
    let mut project_cursor: Option<String> = None;

    loop {
        let projects = client
            .list_projects(ws, project_cursor.as_deref(), Some(200))
            .await
            .map_err(|e| ResolverError::Client {
                context: "list_projects failed".into(),
                source: e,
            })?;

        for project in &projects.items {
            let mut board_cursor: Option<String> = None;
            loop {
                let boards = client
                    .list_boards(ws, &project.slug, board_cursor.as_deref(), Some(200))
                    .await
                    .map_err(|e| ResolverError::Client {
                        context: "list_boards failed".into(),
                        source: e,
                    })?;

                for board in &boards.items {
                    if board.name.to_ascii_lowercase().contains(&needle) {
                        return Ok(board.id.to_string());
                    }
                }

                match boards.next_cursor {
                    Some(next) if boards.has_more => board_cursor = Some(next),
                    _ => break,
                }
            }
        }

        match projects.next_cursor {
            Some(next) if projects.has_more => project_cursor = Some(next),
            _ => break,
        }
    }

    Err(ResolverError::BoardNotFound {
        board_ref: board_ref.to_string(),
        workspace: ws.to_string(),
    })
}

/// Resolves a status/column name to matching column UUIDs.
///
/// When `board` is provided (name or UUID), resolves within that one board
/// using a single `list_columns` call. Otherwise walks all projects and all
/// their boards to collect matching columns across the workspace.
pub async fn resolve_column_ids(
    client: &AtlasClient,
    ws: &str,
    board: Option<&str>,
    status_name: &str,
) -> Result<Vec<String>, ResolverError> {
    if let Some(board_ref) = board {
        let board_id = resolve_board_id(client, ws, board_ref).await?;
        let board_uuid: uuid::Uuid =
            board_id
                .parse()
                .map_err(|_| ResolverError::InvalidBoardUuid {
                    board_id: board_id.clone(),
                })?;
        let cols =
            client
                .list_columns(ws, board_uuid)
                .await
                .map_err(|e| ResolverError::Client {
                    context: "list_columns failed".into(),
                    source: e,
                })?;
        return Ok(match_columns_by_name(status_name, &cols));
    }

    let mut all_cols = Vec::new();
    let mut project_cursor: Option<String> = None;

    loop {
        let projects = client
            .list_projects(ws, project_cursor.as_deref(), Some(200))
            .await
            .map_err(|e| ResolverError::Client {
                context: "list_projects failed".into(),
                source: e,
            })?;

        for project in &projects.items {
            let mut board_cursor: Option<String> = None;
            loop {
                let boards = client
                    .list_boards(ws, &project.slug, board_cursor.as_deref(), Some(200))
                    .await
                    .map_err(|e| ResolverError::Client {
                        context: format!("list_boards for project '{}' failed", project.slug),
                        source: e,
                    })?;

                for board in &boards.items {
                    let cols = client.list_columns(ws, board.id).await.map_err(|e| {
                        ResolverError::Client {
                            context: format!("list_columns for board '{}' failed", board.name),
                            source: e,
                        }
                    })?;
                    all_cols.extend(cols);
                }

                match boards.next_cursor {
                    Some(next) if boards.has_more => board_cursor = Some(next),
                    _ => break,
                }
            }
        }

        match projects.next_cursor {
            Some(next) if projects.has_more => project_cursor = Some(next),
            _ => break,
        }
    }

    Ok(match_columns_by_name(status_name, &all_cols))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn transport_error() -> ClientError {
        ClientError::Transport(reqwest::Client::new().get("not-a-url").build().unwrap_err())
    }

    // -----------------------------------------------------------------------
    // ResolverError Display (T18)
    // -----------------------------------------------------------------------

    #[test]
    fn resolver_error_board_not_found_display() {
        let e = ResolverError::BoardNotFound {
            board_ref: "x".into(),
            workspace: "ws".into(),
        };
        assert_eq!(
            e.to_string(),
            "no board matching 'x' found in workspace 'ws'"
        );
    }

    #[test]
    fn resolver_error_invalid_board_uuid_display() {
        let e = ResolverError::InvalidBoardUuid {
            board_id: "bad".into(),
        };
        assert_eq!(e.to_string(), "resolved board_id 'bad' is not a valid UUID");
    }

    #[test]
    fn resolver_error_client_display_includes_context_and_source() {
        let source = transport_error();
        let source_str = source.to_string();
        let e = ResolverError::Client {
            context: "list_projects failed".into(),
            source,
        };
        assert_eq!(e.to_string(), format!("list_projects failed: {source_str}"));
    }

    #[test]
    fn resolver_error_client_source_is_the_inner_error() {
        use std::error::Error;
        let e = ResolverError::Client {
            context: "ctx".into(),
            source: transport_error(),
        };
        assert!(e.source().is_some());
    }

    #[test]
    fn resolver_error_board_not_found_source_is_none() {
        use std::error::Error;
        let e = ResolverError::BoardNotFound {
            board_ref: "x".into(),
            workspace: "ws".into(),
        };
        assert!(e.source().is_none());
    }

    // -----------------------------------------------------------------------
    // parse_csv (T20 — moved from atlas_mcp response.rs)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_csv_empty_input() {
        assert!(parse_csv("").is_empty());
    }

    #[test]
    fn parse_csv_whitespace_only() {
        assert!(parse_csv("  ").is_empty());
    }

    #[test]
    fn parse_csv_single_item() {
        assert_eq!(parse_csv("low"), vec!["low"]);
    }

    #[test]
    fn parse_csv_multiple_items() {
        assert_eq!(parse_csv("low,medium,high"), vec!["low", "medium", "high"]);
    }

    #[test]
    fn parse_csv_trims_whitespace() {
        assert_eq!(
            parse_csv("low , medium , high"),
            vec!["low", "medium", "high"]
        );
    }

    #[test]
    fn parse_csv_skips_blank_segments() {
        assert_eq!(parse_csv("low,,high"), vec!["low", "high"]);
    }

    // -----------------------------------------------------------------------
    // validate_priority (T20 — moved from atlas_mcp response.rs)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_priority_valid_values_pass() {
        for v in &["low", "medium", "high", "urgent"] {
            assert!(validate_priority(v).is_ok(), "'{v}' should be valid");
        }
    }

    #[test]
    fn validate_priority_invalid_value_lists_options() {
        let err = validate_priority("critical").unwrap_err();
        assert!(err.contains("critical"), "error must echo the bad value");
        assert!(err.contains("low"), "error must list valid values");
        assert!(err.contains("urgent"), "error must list valid values");
    }

    // -----------------------------------------------------------------------
    // validate_assignee_type (T20 — moved from atlas_mcp response.rs)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_assignee_type_valid_values_pass() {
        assert!(validate_assignee_type("user").is_ok());
        assert!(validate_assignee_type("api_key").is_ok());
    }

    #[test]
    fn validate_assignee_type_invalid_lists_options() {
        let err = validate_assignee_type("group").unwrap_err();
        assert!(err.contains("group"), "error must echo the bad value");
        assert!(err.contains("user"), "error must list valid values");
        assert!(err.contains("api_key"), "error must list valid values");
    }

    // -----------------------------------------------------------------------
    // validate_reference_kind (T20 — moved from atlas_mcp response.rs)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_reference_kind_valid_values_pass() {
        for v in &["relates", "blocks", "parent", "spec", "docs"] {
            assert!(validate_reference_kind(v).is_ok(), "'{v}' should be valid");
        }
    }

    #[test]
    fn validate_reference_kind_docs_accepted() {
        assert!(validate_reference_kind("docs").is_ok());
    }

    #[test]
    fn validate_reference_kind_existing_kinds_preserved() {
        for v in &["relates", "blocks", "parent", "spec"] {
            assert!(validate_reference_kind(v).is_ok(), "'{v}' should still be valid");
        }
    }

    #[test]
    fn validate_reference_kind_unknown_error_includes_docs() {
        let err = validate_reference_kind("mentions").unwrap_err();
        assert!(err.contains("docs"), "error must list docs as a valid value");
    }

    #[test]
    fn validate_reference_kind_invalid_lists_options() {
        let err = validate_reference_kind("linked").unwrap_err();
        assert!(err.contains("linked"), "error must echo the bad value");
        assert!(err.contains("relates"), "error must list valid values");
        assert!(err.contains("blocks"), "error must list valid values");
        assert!(err.contains("parent"), "error must list valid values");
        assert!(err.contains("spec"), "error must list valid values");
        assert!(err.contains("docs"), "error must list valid values");
    }

    // -----------------------------------------------------------------------
    // validate_estimate / validate_estimate_value (T20 — moved)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_estimate_rejects_negative() {
        let err = validate_estimate(-1).unwrap_err();
        assert!(err.contains("-1"), "error must echo the bad value");
        assert!(
            err.contains("non-negative"),
            "error must state the constraint"
        );
    }

    #[test]
    fn validate_estimate_accepts_zero() {
        assert!(validate_estimate(0).is_ok());
    }

    #[test]
    fn validate_estimate_accepts_positive() {
        assert!(validate_estimate(5).is_ok());
        assert!(validate_estimate(100).is_ok());
    }

    #[test]
    fn validate_estimate_value_rejects_negative_number() {
        let v = serde_json::json!(-3);
        let err = validate_estimate_value(&v).unwrap_err();
        assert!(
            err.contains("non-negative"),
            "error must state the constraint"
        );
    }

    #[test]
    fn validate_estimate_value_accepts_zero_and_positive() {
        assert!(validate_estimate_value(&serde_json::json!(0)).is_ok());
        assert!(validate_estimate_value(&serde_json::json!(8)).is_ok());
    }

    #[test]
    fn validate_estimate_value_passes_null() {
        assert!(validate_estimate_value(&serde_json::Value::Null).is_ok());
    }

    #[test]
    fn validate_estimate_value_passes_non_number() {
        assert!(validate_estimate_value(&serde_json::json!("five")).is_ok());
    }

    // -----------------------------------------------------------------------
    // resolve_board_id: UUID passthrough path (T23a)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn resolve_board_id_returns_uuid_string_without_client_call() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        // AtlasClient pointed at a non-routable address — any network call would fail.
        // UUID passthrough must return before issuing any request.
        let client = AtlasClient::new("http://0.0.0.0:0");
        let result = resolve_board_id(&client, "my-ws", uuid_str).await;
        assert_eq!(result.unwrap(), uuid_str);
    }
}
