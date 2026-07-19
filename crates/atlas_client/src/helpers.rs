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
    /// More than one board matched the supplied name fragment.
    ///
    /// `matches` lists the full names of every board that matched so the caller
    /// can disambiguate by passing a more specific name or the board's UUID.
    AmbiguousBoard {
        board_ref: String,
        matches: Vec<String>,
    },
    /// A status filter was supplied but matched no column in scope.
    ///
    /// `available` lists the full names of every column that was considered so
    /// the caller can correct the status name.
    NoMatchingColumns {
        status: String,
        available: Vec<String>,
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
            Self::AmbiguousBoard { board_ref, matches } => write!(
                f,
                "board '{board_ref}' is ambiguous; matches: [{}]; \
                 pass a more specific name or the board's UUID",
                matches.join(", ")
            ),
            Self::NoMatchingColumns { status, available } => write!(
                f,
                "status '{status}' matched no columns; available columns: [{}]",
                available.join(", ")
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

/// Selects exactly one board id from the boards that matched a name fragment.
///
/// Enforces the single-match semantics required by write/destructive callers:
/// zero matches is `BoardNotFound`, more than one is `AmbiguousBoard` listing
/// the candidate names. `matches` holds `(name, id)` for every board whose name
/// contained the fragment. Mirrors `resolve_column_id_on_board`.
// `ResolverError` is intentionally value-based (its async callers already return
// it by value); the large `Err` only exists on the cold error path here.
#[allow(clippy::result_large_err)]
fn select_board_match(
    board_ref: &str,
    ws: &str,
    matches: Vec<(String, String)>,
) -> Result<String, ResolverError> {
    match matches.len() {
        0 => Err(ResolverError::BoardNotFound {
            board_ref: board_ref.to_string(),
            workspace: ws.to_string(),
        }),
        1 => Ok(matches
            .into_iter()
            .next()
            .map(|(_, id)| id)
            .unwrap_or_default()),
        _ => Err(ResolverError::AmbiguousBoard {
            board_ref: board_ref.to_string(),
            matches: matches.into_iter().map(|(name, _)| name).collect(),
        }),
    }
}

/// Resolves a board reference (name fragment or UUID string) to a UUID string.
///
/// When the input parses as a UUID it is returned directly without any network
/// call. Otherwise walks all projects' boards and collects every case-insensitive
/// partial name match. A single match resolves; zero matches is `BoardNotFound`
/// and more than one is `AmbiguousBoard` (listing the candidate names) so an
/// ambiguous name can never silently feed a destructive operation.
pub async fn resolve_board_id(
    client: &AtlasClient,
    ws: &str,
    board_ref: &str,
) -> Result<String, ResolverError> {
    if uuid::Uuid::parse_str(board_ref).is_ok() {
        return Ok(board_ref.to_string());
    }

    let needle = board_ref.to_ascii_lowercase();
    let mut matches: Vec<(String, String)> = Vec::new();
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
                        matches.push((board.name.clone(), board.id.to_string()));
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

    select_board_match(board_ref, ws, matches)
}

/// Collects the columns that a status filter should be matched against.
///
/// When `board` is provided (name or UUID), returns that one board's columns via
/// a single `list_columns` call. Otherwise walks all projects and all their
/// boards, accumulating every column in the workspace.
async fn collect_columns(
    client: &AtlasClient,
    ws: &str,
    board: Option<&str>,
) -> Result<Vec<ColumnDto>, ResolverError> {
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
        return Ok(cols);
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

    Ok(all_cols)
}

/// Resolves a status/column name to matching column UUIDs.
///
/// Returns an empty vec when no column matches — callers that treat an empty set
/// as "no filter" get lenient behavior. Write/read callers that must not silently
/// drop a status filter should use `resolve_column_ids_required` instead.
pub async fn resolve_column_ids(
    client: &AtlasClient,
    ws: &str,
    board: Option<&str>,
    status_name: &str,
) -> Result<Vec<String>, ResolverError> {
    let cols = collect_columns(client, ws, board).await?;
    Ok(match_columns_by_name(status_name, &cols))
}

/// Resolves a status/column name to matching column UUIDs, erroring on no match.
///
/// Unlike `resolve_column_ids`, a status that matches zero columns is a
/// `NoMatchingColumns` error listing the available column names, rather than an
/// empty vec. This prevents a mistyped status from being interpreted downstream
/// as "no filter" and silently returning the full, unfiltered result set.
pub async fn resolve_column_ids_required(
    client: &AtlasClient,
    ws: &str,
    board: Option<&str>,
    status_name: &str,
) -> Result<Vec<String>, ResolverError> {
    let cols = collect_columns(client, ws, board).await?;
    columns_for_status(status_name, &cols)
}

/// Matches a status name against a column set, erroring when nothing matches.
///
/// Pure counterpart of `resolve_column_ids_required`: separated so the
/// no-match-lists-available-columns behavior is testable without a client.
// See `select_board_match`: the large `Err` only occurs on the cold error path.
#[allow(clippy::result_large_err)]
fn columns_for_status(status_name: &str, cols: &[ColumnDto]) -> Result<Vec<String>, ResolverError> {
    let ids = match_columns_by_name(status_name, cols);

    if ids.is_empty() {
        return Err(ResolverError::NoMatchingColumns {
            status: status_name.to_string(),
            available: cols.iter().map(|c| c.name.clone()).collect(),
        });
    }

    Ok(ids)
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

    fn column(name: &str) -> ColumnDto {
        serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4(),
            "board_id": uuid::Uuid::new_v4(),
            "name": name,
            "position_key": "a0",
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
        }))
        .expect("valid ColumnDto json")
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
    fn resolver_error_ambiguous_board_display_lists_candidates() {
        let e = ResolverError::AmbiguousBoard {
            board_ref: "sprint".into(),
            matches: vec!["Sprint Board".into(), "Sprint Backlog".into()],
        };
        let msg = e.to_string();
        assert!(msg.contains("'sprint' is ambiguous"), "got: {msg}");
        assert!(msg.contains("Sprint Board"), "must list candidate: {msg}");
        assert!(msg.contains("Sprint Backlog"), "must list candidate: {msg}");
    }

    #[test]
    fn resolver_error_no_matching_columns_display_lists_available() {
        let e = ResolverError::NoMatchingColumns {
            status: "dne".into(),
            available: vec!["To Do".into(), "Done".into()],
        };
        let msg = e.to_string();
        assert!(msg.contains("'dne' matched no columns"), "got: {msg}");
        assert!(msg.contains("To Do"), "must list available column: {msg}");
        assert!(msg.contains("Done"), "must list available column: {msg}");
    }

    // -----------------------------------------------------------------------
    // select_board_match: 0 / 1 / many matching boards
    // -----------------------------------------------------------------------

    #[test]
    fn select_board_match_no_matches_is_board_not_found() {
        let e = select_board_match("x", "ws", Vec::new()).unwrap_err();
        assert!(matches!(e, ResolverError::BoardNotFound { .. }));
    }

    #[test]
    fn select_board_match_single_match_resolves() {
        let matches = vec![("Sprint Board".to_string(), "board-id-1".to_string())];
        let id = select_board_match("sprint", "ws", matches).unwrap();
        assert_eq!(id, "board-id-1");
    }

    #[test]
    fn select_board_match_multiple_matches_is_ambiguous_with_candidates() {
        let matches = vec![
            ("Sprint Board".to_string(), "id-1".to_string()),
            ("Sprint Backlog".to_string(), "id-2".to_string()),
        ];
        let e = select_board_match("sprint", "ws", matches).unwrap_err();

        match e {
            ResolverError::AmbiguousBoard { board_ref, matches } => {
                assert_eq!(board_ref, "sprint");
                assert_eq!(matches, vec!["Sprint Board", "Sprint Backlog"]);
            }
            other => panic!("expected AmbiguousBoard, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // columns_for_status: strict status matching (finding 2)
    // -----------------------------------------------------------------------

    #[test]
    fn columns_for_status_returns_matching_ids() {
        let in_progress = column("In Progress");
        let expected = in_progress.id.to_string();
        let cols = vec![column("To Do"), in_progress, column("Done")];

        let ids = columns_for_status("progress", &cols).unwrap();

        assert_eq!(ids, vec![expected]);
    }

    #[test]
    fn columns_for_status_no_match_errors_with_available_columns() {
        let cols = vec![column("To Do"), column("Done")];
        let e = columns_for_status("archived", &cols).unwrap_err();

        match e {
            ResolverError::NoMatchingColumns { status, available } => {
                assert_eq!(status, "archived");
                assert_eq!(available, vec!["To Do", "Done"]);
            }
            other => panic!("expected NoMatchingColumns, got {other:?}"),
        }
    }

    #[test]
    fn columns_for_status_empty_board_errors_with_empty_available() {
        let e = columns_for_status("anything", &[]).unwrap_err();
        assert!(matches!(e, ResolverError::NoMatchingColumns { .. }));
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
            assert!(
                validate_reference_kind(v).is_ok(),
                "'{v}' should still be valid"
            );
        }
    }

    #[test]
    fn validate_reference_kind_unknown_error_includes_docs() {
        let err = validate_reference_kind("mentions").unwrap_err();
        assert!(
            err.contains("docs"),
            "error must list docs as a valid value"
        );
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
