#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

//! Bulk stdin JSON-lines helper shared by create/update commands.
//!
//! Commands that accept `--stdin` read one JSON object per line from stdin.
//! `parse_batch_lines` encapsulates the read loop, per-line parse-error
//! reporting, and the "any-failed" tracking. The calling command owns the
//! async API dispatch loop.

use std::io::BufRead;

use serde::de::DeserializeOwned;

use atlas_api::dtos::boards_tasks::{CreateTaskRequest, UpdateTaskRequest};
use atlas_api::dtos::documents::{CreateDocumentRequest, UpdateDocumentRequest};

use crate::error::CliError;

// ---------------------------------------------------------------------------
// Per-command stdin line shapes
// ---------------------------------------------------------------------------

/// JSON-lines shape for `atlas tasks create --stdin`.
///
/// Required fields per line: `board_id` (UUID of the target board), `column_id`
/// (UUID of the target column), `title` (string). Optional: `description`,
/// `properties` (`priority`, `estimate`, `labels`, `due_date`). Example:
///
/// ```json
/// {"board_id":"<uuid>","column_id":"<uuid>","title":"Fix login bug"}
/// ```
#[derive(serde::Deserialize)]
pub(crate) struct BulkTaskCreateLine {
    pub board_id: uuid::Uuid,
    #[serde(flatten)]
    pub body: CreateTaskRequest,
}

/// Captures field presence for nullable patch fields on bulk update lines.
///
/// Without this, serde maps JSON `null` and a missing key both to `None`,
/// making it impossible to distinguish "leave unchanged" from "clear the field".
/// With this deserializer on `#[serde(default)]` fields, an absent key stays
/// `None` (default), while an explicit `null` or a value becomes `Some(...)`.
fn maybe_value<'de, D>(de: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    serde_json::Value::deserialize(de).map(Some)
}

/// JSON-lines shape for `atlas tasks update --stdin`.
///
/// Required field per line: `readable_id` (e.g. `"ATL-42"`). All other fields
/// are optional and follow PATCH semantics: an absent key leaves the field
/// unchanged, an explicit `null` clears it (for `priority`, `due_date`,
/// `estimate`), and a value sets it. Example:
///
/// ```json
/// {"readable_id":"ATL-42","title":"New title","priority":null}
/// ```
#[derive(serde::Deserialize)]
pub(crate) struct BulkTaskUpdateLine {
    pub readable_id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(default, deserialize_with = "maybe_value")]
    pub priority: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "maybe_value")]
    pub due_date: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "maybe_value")]
    pub estimate: Option<serde_json::Value>,
    pub labels: Option<Vec<String>>,
}

impl BulkTaskUpdateLine {
    /// Converts the parsed line into an `UpdateTaskRequest` body.
    pub(crate) fn into_request(self) -> UpdateTaskRequest {
        UpdateTaskRequest {
            title: self.title,
            description: self.description,
            priority: self.priority,
            due_date: self.due_date,
            estimate: self.estimate,
            labels: self.labels,
            properties: None,
        }
    }
}

/// JSON-lines shape for `atlas docs create --stdin`.
///
/// Required fields per line: `project` (project slug), `title` (string).
/// Optional: `folder_id` (UUID), `content` (markdown string). Example:
///
/// ```json
/// {"project":"my-project","title":"Meeting notes","content":"# Hello"}
/// ```
#[derive(serde::Deserialize)]
pub(crate) struct BulkDocCreateLine {
    pub project: String,
    #[serde(flatten)]
    pub body: CreateDocumentRequest,
}

/// JSON-lines shape for `atlas docs update-metadata --stdin`.
///
/// Required field per line: `slug` (document slug). Optional: `title`,
/// `folder_id` (UUID). Absent fields are left unchanged. Example:
///
/// ```json
/// {"slug":"my-doc","title":"New title"}
/// ```
#[derive(serde::Deserialize)]
pub(crate) struct BulkDocUpdateMetadataLine {
    pub slug: String,
    #[serde(flatten)]
    pub body: UpdateDocumentRequest,
}

// ---------------------------------------------------------------------------
// Core parsing helper
// ---------------------------------------------------------------------------

/// Reads non-empty lines from `reader`, deserializes each as `R`, and returns
/// successfully-parsed items plus `any_failed`.
///
/// Parse failures are written to stderr with a 1-based line number; the batch
/// continues after each failure so all lines are attempted. I/O errors on
/// `reader` cause an immediate `Err` return.
pub(crate) fn parse_batch_lines<R, B>(reader: B) -> Result<(Vec<R>, bool), CliError>
where
    R: DeserializeOwned,
    B: BufRead,
{
    let mut items: Vec<R> = Vec::new();
    let mut any_failed = false;

    for (idx, line_result) in reader.lines().enumerate() {
        let line = line_result?;
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<R>(trimmed) {
            Ok(item) => items.push(item),
            Err(e) => {
                eprintln!("line {}: parse error: {e}", idx + 1);
                any_failed = true;
            }
        }
    }

    Ok((items, any_failed))
}

/// Reads from stdin using `parse_batch_lines`.
pub(crate) fn parse_stdin_batch<R>() -> Result<(Vec<R>, bool), CliError>
where
    R: DeserializeOwned,
{
    parse_batch_lines(std::io::stdin().lock())
}

/// Emits a compact JSON result line to stdout.
pub(crate) fn emit_batch_line(value: &serde_json::Value) -> Result<(), CliError> {
    let s = serde_json::to_string(value)
        .map_err(|e| CliError::Io(std::io::Error::other(e.to_string())))?;
    println!("{s}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // T66: Malformed JSON line is an error, iteration continues

    #[test]
    fn malformed_json_line_is_reported_and_batch_continues() {
        let input = b"not-json\n{\"readable_id\":\"ATL-1\"}\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkTaskUpdateLine, _>(Cursor::new(input)).unwrap();
        assert!(
            any_failed,
            "any_failed must be true when a line fails to parse"
        );
        assert_eq!(items.len(), 1, "valid lines must still be collected");
        assert_eq!(items[0].readable_id, "ATL-1");
    }

    #[test]
    fn all_malformed_lines_set_any_failed_and_return_empty_items() {
        let input = b"bad1\nbad2\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkTaskUpdateLine, _>(Cursor::new(input)).unwrap();
        assert!(any_failed, "any_failed must be true when all lines fail");
        assert!(items.is_empty(), "no items when all lines fail to parse");
    }

    // T67: Two valid lines produce two parsed items

    #[test]
    fn two_valid_lines_produce_two_items() {
        let input = b"{\"readable_id\":\"ATL-1\"}\n{\"readable_id\":\"ATL-2\"}\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkTaskUpdateLine, _>(Cursor::new(input)).unwrap();
        assert!(
            !any_failed,
            "any_failed must be false when all lines succeed"
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].readable_id, "ATL-1");
        assert_eq!(items[1].readable_id, "ATL-2");
    }

    #[test]
    fn missing_required_field_is_a_parse_error() {
        // BulkTaskUpdateLine requires `readable_id`; omitting it is a parse error.
        let input = b"{\"title\":\"Hello\"}\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkTaskUpdateLine, _>(Cursor::new(input)).unwrap();
        assert!(
            any_failed,
            "missing required field must produce a parse error"
        );
        assert!(items.is_empty());
    }

    #[test]
    fn empty_and_whitespace_lines_are_skipped() {
        let input = b"\n  \n{\"readable_id\":\"ATL-3\"}\n\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkTaskUpdateLine, _>(Cursor::new(input)).unwrap();
        assert!(!any_failed);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].readable_id, "ATL-3");
    }

    #[test]
    fn bulk_task_update_line_with_title_parses() {
        let input = b"{\"readable_id\":\"ATL-5\",\"title\":\"New title\"}\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkTaskUpdateLine, _>(Cursor::new(input)).unwrap();
        assert!(!any_failed);
        assert_eq!(items[0].title.as_deref(), Some("New title"));
    }

    #[test]
    fn bulk_task_update_line_into_request_sets_title() {
        let line = BulkTaskUpdateLine {
            readable_id: "ATL-1".to_owned(),
            title: Some("My title".to_owned()),
            description: None,
            priority: None,
            due_date: None,
            estimate: None,
            labels: None,
        };
        let req = line.into_request();
        assert_eq!(req.title.as_deref(), Some("My title"));
        assert!(req.priority.is_none(), "absent priority must be None");
    }

    #[test]
    fn bulk_doc_create_line_parses_project_and_title() {
        let input = b"{\"project\":\"my-proj\",\"title\":\"Hello\"}\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkDocCreateLine, _>(Cursor::new(input)).unwrap();
        assert!(!any_failed);
        assert_eq!(items[0].project, "my-proj");
        assert_eq!(items[0].body.title, "Hello");
    }

    #[test]
    fn bulk_doc_update_metadata_line_parses_slug_and_title() {
        let input = b"{\"slug\":\"my-doc\",\"title\":\"New title\"}\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkDocUpdateMetadataLine, _>(Cursor::new(input)).unwrap();
        assert!(!any_failed);
        assert_eq!(items[0].slug, "my-doc");
        assert_eq!(items[0].body.title.as_deref(), Some("New title"));
    }

    #[test]
    fn bulk_task_create_missing_board_id_is_parse_error() {
        // board_id is required on BulkTaskCreateLine
        let input = b"{\"column_id\":\"550e8400-e29b-41d4-a716-446655440000\",\"title\":\"T\"}\n";
        let (items, any_failed) =
            parse_batch_lines::<BulkTaskCreateLine, _>(Cursor::new(input)).unwrap();
        assert!(any_failed, "missing board_id must be a parse error");
        assert!(items.is_empty());
    }
}
