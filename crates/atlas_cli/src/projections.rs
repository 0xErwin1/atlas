#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use atlas_api::dtos::boards_tasks::{TaskDto, TaskSummaryDto};
use atlas_api::dtos::documents::{ActorDto, DocumentDto, DocumentSummaryDto};
use atlas_api::dtos::search::{SearchHitDto, SearchKindDto};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::output::TableRow;

/// Standard pagination envelope used for all list responses.
#[derive(Debug, Serialize)]
pub(crate) struct Envelope<T: Serialize> {
    pub(crate) items: Vec<T>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) has_more: bool,
}

/// CLI projection for a search hit, mirroring the MCP search-hit shape exactly.
///
/// Optional fields use `skip_serializing_if` so they are omitted (not `null`)
/// when absent, matching the MCP wire format.
#[derive(Debug, Serialize)]
pub(crate) struct SearchHitProjection {
    pub(crate) id: Uuid,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) score: f32,
    pub(crate) updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) readable_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) project_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) column_name: Option<String>,
}

impl From<SearchHitDto> for SearchHitProjection {
    fn from(dto: SearchHitDto) -> Self {
        let kind = match dto.kind {
            SearchKindDto::Document => "document".to_owned(),
            SearchKindDto::Task => "task".to_owned(),
        };

        Self {
            id: dto.id,
            kind,
            title: dto.title,
            score: dto.score,
            updated_at: dto.updated_at,
            readable_id: dto.readable_id,
            snippet: dto.snippet,
            project_slug: dto.project_slug,
            column_name: dto.column_name,
        }
    }
}

impl TableRow for SearchHitProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Kind", "Title", "Score", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        let id_display = self
            .readable_id
            .as_deref()
            .map(|r| format!("{} [{}]", self.id, r))
            .unwrap_or_else(|| self.id.to_string());

        vec![
            id_display,
            self.kind.clone(),
            self.title.clone(),
            format!("{:.4}", self.score),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Task projections
// ---------------------------------------------------------------------------

/// Compact assignee shape used in task summary rows.
///
/// Mirrors the `{type, display_name}` pair emitted by the MCP `list_tasks`
/// response. `type` is serialized as the JSON key; stored as `type_` to avoid
/// conflict with the Rust keyword.
#[derive(Debug, Serialize)]
pub(crate) struct AssigneeProjection {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) display_name: Option<String>,
}

impl From<ActorDto> for AssigneeProjection {
    fn from(a: ActorDto) -> Self {
        Self {
            type_: a.r#type,
            display_name: a.display_name,
        }
    }
}

impl TableRow for AssigneeProjection {
    fn headers() -> &'static [&'static str] {
        &["Type", "Display Name"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.type_.clone(),
            self.display_name.clone().unwrap_or_default(),
        ]
    }
}

/// Task list row projection (from `TaskSummaryDto`).
#[derive(Debug, Serialize)]
pub(crate) struct TaskSummaryProjection {
    pub(crate) readable_id: String,
    pub(crate) title: String,
    pub(crate) board_name: String,
    pub(crate) column_name: String,
    pub(crate) priority: Option<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) estimate: Option<i32>,
    pub(crate) assignees: Vec<AssigneeProjection>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<TaskSummaryDto> for TaskSummaryProjection {
    fn from(dto: TaskSummaryDto) -> Self {
        let assignees = dto.assignees.into_iter().map(AssigneeProjection::from).collect();

        Self {
            readable_id: dto.readable_id,
            title: dto.title,
            board_name: dto.board_name,
            column_name: dto.column_name,
            priority: dto.priority,
            labels: dto.labels,
            estimate: dto.estimate,
            assignees,
            updated_at: dto.updated_at,
        }
    }
}

impl TableRow for TaskSummaryProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Title", "Board", "Column", "Priority", "Labels", "Est.", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.readable_id.clone(),
            self.title.clone(),
            self.board_name.clone(),
            self.column_name.clone(),
            self.priority.clone().unwrap_or_default(),
            self.labels.join(", "),
            self.estimate.map(|e| e.to_string()).unwrap_or_default(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Compact task projection (from `TaskDto`).
///
/// `board_name` and `column_name` are omitted when the server returns an empty
/// string — mutation responses do not populate them, so an empty value would be
/// misleading rather than informative.
#[derive(Debug, Serialize)]
pub(crate) struct TaskCompactProjection {
    pub(crate) readable_id: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) board_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) column_name: Option<String>,
    pub(crate) priority: Option<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) estimate: Option<i32>,
    pub(crate) due_date: Option<DateTime<Utc>>,
    pub(crate) parent_task_id: Option<Uuid>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<TaskDto> for TaskCompactProjection {
    fn from(task: TaskDto) -> Self {
        let board_name = if task.board_name.is_empty() {
            None
        } else {
            Some(task.board_name)
        };

        let column_name = if task.column_name.is_empty() {
            None
        } else {
            Some(task.column_name)
        };

        Self {
            readable_id: task.readable_id,
            title: task.title,
            board_name,
            column_name,
            priority: task.priority,
            labels: task.labels,
            estimate: task.estimate,
            due_date: task.due_date,
            parent_task_id: task.parent_task_id,
            updated_at: task.updated_at,
        }
    }
}

impl TableRow for TaskCompactProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Title", "Board", "Column", "Priority", "Labels", "Est.", "Due", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.readable_id.clone(),
            self.title.clone(),
            self.board_name.as_deref().unwrap_or("").to_owned(),
            self.column_name.as_deref().unwrap_or("").to_owned(),
            self.priority.clone().unwrap_or_default(),
            self.labels.join(", "),
            self.estimate.map(|e| e.to_string()).unwrap_or_default(),
            self.due_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Full task projection (`TaskDto` plus all sub-resources).
///
/// Sub-resources (references, subtasks, assignees) each have an error fallback:
/// when a backing client call fails, `*_error` is set and the value field is
/// absent. Exactly one of each pair is `Some` at runtime; `skip_serializing_if`
/// ensures the absent member is omitted from the JSON output.
#[derive(Debug, Serialize)]
pub(crate) struct TaskFullProjection {
    pub(crate) readable_id: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) board_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) column_name: Option<String>,
    pub(crate) priority: Option<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) estimate: Option<i32>,
    pub(crate) due_date: Option<DateTime<Utc>>,
    pub(crate) parent_task_id: Option<Uuid>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) references: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) references_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subtasks: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subtasks_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) assignees: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) assignees_error: Option<String>,
}

impl TaskFullProjection {
    /// Assembles the full task projection from a task and its sub-resource results.
    ///
    /// Each sub-resource is provided as a `Result`: `Ok(items)` populates the
    /// value field; `Err(msg)` populates the corresponding `*_error` field instead.
    pub(crate) fn new(
        task: TaskDto,
        refs: Result<Vec<serde_json::Value>, String>,
        subtasks: Result<Vec<serde_json::Value>, String>,
        assignees: Result<Vec<serde_json::Value>, String>,
    ) -> Self {
        let board_name = if task.board_name.is_empty() {
            None
        } else {
            Some(task.board_name)
        };

        let column_name = if task.column_name.is_empty() {
            None
        } else {
            Some(task.column_name)
        };

        let (references, references_error) = match refs {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };

        let (subtasks, subtasks_error) = match subtasks {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };

        let (assignees, assignees_error) = match assignees {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };

        Self {
            readable_id: task.readable_id,
            title: task.title,
            board_name,
            column_name,
            priority: task.priority,
            labels: task.labels,
            estimate: task.estimate,
            due_date: task.due_date,
            parent_task_id: task.parent_task_id,
            updated_at: task.updated_at,
            description: task.description,
            references,
            references_error,
            subtasks,
            subtasks_error,
            assignees,
            assignees_error,
        }
    }
}

impl TableRow for TaskFullProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Title", "Board", "Column", "Priority", "Labels", "Est.", "Due", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.readable_id.clone(),
            self.title.clone(),
            self.board_name.as_deref().unwrap_or("").to_owned(),
            self.column_name.as_deref().unwrap_or("").to_owned(),
            self.priority.clone().unwrap_or_default(),
            self.labels.join(", "),
            self.estimate.map(|e| e.to_string()).unwrap_or_default(),
            self.due_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Projection for a completed task deletion.
///
/// Uses `readable_id` (not a generic `id`) to match the resource's public
/// identifier, consistent with how tasks are addressed across the CLI.
#[derive(Debug, Serialize)]
pub(crate) struct DeleteTaskProjection {
    pub(crate) deleted: bool,
    pub(crate) readable_id: String,
}

impl TableRow for DeleteTaskProjection {
    fn headers() -> &'static [&'static str] {
        &["Deleted", "ID"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.deleted.to_string(), self.readable_id.clone()]
    }
}

// ---------------------------------------------------------------------------
// Document projections
// ---------------------------------------------------------------------------

/// Summary document projection used for `docs list` responses.
///
/// Matches the `project_document_summary` shape from the MCP: optional fields
/// (`slug`, `folder_id`) are omitted when absent rather than serialized as null.
#[derive(Debug, Serialize)]
pub(crate) struct DocSummaryProjection {
    pub(crate) id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) slug: Option<String>,
    pub(crate) title: String,
    pub(crate) head_seq: i64,
    pub(crate) updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) folder_id: Option<Uuid>,
}

impl From<DocumentSummaryDto> for DocSummaryProjection {
    fn from(doc: DocumentSummaryDto) -> Self {
        Self {
            id: doc.id,
            slug: doc.slug,
            title: doc.title,
            head_seq: doc.head_seq,
            updated_at: doc.updated_at,
            folder_id: doc.folder_id,
        }
    }
}

impl TableRow for DocSummaryProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Slug", "Title", "Seq", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.slug.clone().unwrap_or_default(),
            self.title.clone(),
            self.head_seq.to_string(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Compact document projection mirroring `project_document_compact` exactly.
///
/// All eight fields are always present (optional values serialize as `null`
/// when absent), matching the MCP wire format.
#[derive(Debug, Serialize)]
pub(crate) struct DocCompactProjection {
    pub(crate) id: Uuid,
    pub(crate) slug: Option<String>,
    pub(crate) title: String,
    pub(crate) head_revision_id: Uuid,
    pub(crate) head_seq: i64,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) folder_id: Option<Uuid>,
    pub(crate) project_id: Option<Uuid>,
}

impl From<DocumentDto> for DocCompactProjection {
    fn from(doc: DocumentDto) -> Self {
        Self {
            id: doc.id,
            slug: doc.slug,
            title: doc.title,
            head_revision_id: doc.head_revision_id,
            head_seq: doc.head_seq,
            updated_at: doc.updated_at,
            folder_id: doc.folder_id,
            project_id: doc.project_id,
        }
    }
}

impl TableRow for DocCompactProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Slug", "Title", "Rev", "Seq", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.slug.clone().unwrap_or_default(),
            self.title.clone(),
            self.head_revision_id.to_string(),
            self.head_seq.to_string(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Full document projection mirroring `project_document_full`: compact fields
/// plus markdown content and frontmatter.
#[derive(Debug, Serialize)]
pub(crate) struct DocFullProjection {
    pub(crate) id: Uuid,
    pub(crate) slug: Option<String>,
    pub(crate) title: String,
    pub(crate) head_revision_id: Uuid,
    pub(crate) head_seq: i64,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) folder_id: Option<Uuid>,
    pub(crate) project_id: Option<Uuid>,
    pub(crate) content: String,
    pub(crate) frontmatter: serde_json::Value,
}

impl From<DocumentDto> for DocFullProjection {
    fn from(doc: DocumentDto) -> Self {
        Self {
            id: doc.id,
            slug: doc.slug,
            title: doc.title,
            head_revision_id: doc.head_revision_id,
            head_seq: doc.head_seq,
            updated_at: doc.updated_at,
            folder_id: doc.folder_id,
            project_id: doc.project_id,
            content: doc.content,
            frontmatter: doc.frontmatter,
        }
    }
}

impl TableRow for DocFullProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Slug", "Title", "Rev", "Seq", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.slug.clone().unwrap_or_default(),
            self.title.clone(),
            self.head_revision_id.to_string(),
            self.head_seq.to_string(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Projection for a completed document deletion.
///
/// Uses `slug` (not a generic `id`) to match the document's public identifier,
/// consistent with how documents are addressed across the CLI and the MCP.
#[derive(Debug, Serialize)]
pub(crate) struct DeleteDocProjection {
    pub(crate) deleted: bool,
    pub(crate) slug: String,
}

impl TableRow for DeleteDocProjection {
    fn headers() -> &'static [&'static str] {
        &["Deleted", "Slug"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.deleted.to_string(), self.slug.clone()]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Asserts that a serialized `serde_json::Value` contains all `required`
    /// keys and only keys from `required ∪ optional`. Unknown keys cause a panic.
    pub(crate) fn assert_projection_fields(
        v: &serde_json::Value,
        required: &[&str],
        optional: &[&str],
    ) {
        let obj = v
            .as_object()
            .expect("projection must serialize to a JSON object");

        for key in required {
            assert!(
                obj.contains_key(*key),
                "required field '{key}' missing from projection: {obj:?}"
            );
        }

        let allowed: std::collections::HashSet<&str> =
            required.iter().chain(optional.iter()).copied().collect();

        for key in obj.keys() {
            assert!(
                allowed.contains(key.as_str()),
                "unexpected field '{key}' in projection (not in required or optional)"
            );
        }
    }

    fn make_search_hit(kind: SearchKindDto, readable_id: Option<&str>) -> SearchHitDto {
        SearchHitDto {
            id: Uuid::now_v7(),
            kind,
            readable_id: readable_id.map(str::to_owned),
            title: "Test hit".to_owned(),
            snippet: None,
            score: 0.9,
            updated_at: Utc::now(),
            project_slug: None,
            column_name: None,
        }
    }

    #[test]
    fn search_hit_projection_contract_required_and_optional_fields() {
        let dto = make_search_hit(SearchKindDto::Task, Some("ATL-1"));
        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();

        assert_projection_fields(
            &value,
            &["id", "kind", "title", "score", "updated_at"],
            &["readable_id", "snippet", "project_slug", "column_name"],
        );
    }

    #[test]
    fn search_hit_task_kind_serializes_as_lowercase_string() {
        let dto = make_search_hit(SearchKindDto::Task, None);
        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["kind"], "task");
    }

    #[test]
    fn search_hit_document_kind_serializes_as_lowercase_string() {
        let dto = make_search_hit(SearchKindDto::Document, None);
        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["kind"], "document");
    }

    #[test]
    fn search_hit_optional_fields_absent_when_none() {
        let dto = make_search_hit(SearchKindDto::Document, None);
        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();

        assert!(
            value.get("readable_id").is_none(),
            "readable_id must be absent when None"
        );
        assert!(
            value.get("snippet").is_none(),
            "snippet must be absent when None"
        );
        assert!(
            value.get("project_slug").is_none(),
            "project_slug must be absent when None"
        );
        assert!(
            value.get("column_name").is_none(),
            "column_name must be absent when None"
        );
    }

    #[test]
    fn search_hit_optional_fields_present_when_some() {
        let mut dto = make_search_hit(SearchKindDto::Task, Some("ATL-42"));
        dto.snippet = Some("highlighted text".to_owned());
        dto.project_slug = Some("my-project".to_owned());
        dto.column_name = Some("In Progress".to_owned());

        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();

        assert_eq!(value["readable_id"], "ATL-42");
        assert_eq!(value["snippet"], "highlighted text");
        assert_eq!(value["project_slug"], "my-project");
        assert_eq!(value["column_name"], "In Progress");
    }

    #[test]
    fn envelope_serializes_with_items_cursor_and_has_more() {
        let items: Vec<serde_json::Value> = vec![serde_json::json!({"x": 1})];
        let env = Envelope {
            items,
            next_cursor: Some("cursor123".to_owned()),
            has_more: true,
        };
        let value = serde_json::to_value(&env).unwrap();
        assert_eq!(value["has_more"], true);
        assert_eq!(value["next_cursor"], "cursor123");
        assert!(value["items"].is_array());
    }

    #[test]
    fn envelope_next_cursor_is_null_when_none() {
        let env: Envelope<serde_json::Value> = Envelope {
            items: vec![],
            next_cursor: None,
            has_more: false,
        };
        let value = serde_json::to_value(&env).unwrap();
        assert!(value["next_cursor"].is_null());
    }

    // -----------------------------------------------------------------------
    // Task projections (T27-T28)
    // -----------------------------------------------------------------------

    fn make_actor_dto() -> atlas_api::dtos::documents::ActorDto {
        atlas_api::dtos::documents::ActorDto {
            r#type: "user".to_owned(),
            id: Uuid::now_v7(),
            display_name: Some("Alice".to_owned()),
            key_type: None,
            account_status: None,
        }
    }

    fn make_task_dto(board_name: &str, column_name: &str) -> TaskDto {
        TaskDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Uuid::now_v7(),
            board_id: Uuid::now_v7(),
            column_id: Uuid::now_v7(),
            parent_task_id: None,
            readable_id: "ATL-1".to_owned(),
            title: "Test task".to_owned(),
            description: String::new(),
            priority: Some("high".to_owned()),
            due_date: None,
            estimate: Some(3),
            labels: vec!["rust".to_owned()],
            properties: None,
            created_by: make_actor_dto(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            board_name: board_name.to_owned(),
            column_name: column_name.to_owned(),
        }
    }

    fn make_task_summary_dto() -> TaskSummaryDto {
        TaskSummaryDto {
            id: Uuid::now_v7(),
            readable_id: "ATL-2".to_owned(),
            board_id: Uuid::now_v7(),
            column_id: Uuid::now_v7(),
            title: "Summary task".to_owned(),
            priority: Some("low".to_owned()),
            estimate: None,
            labels: vec![],
            assignees: vec![make_actor_dto()],
            board_name: "Dev Board".to_owned(),
            column_name: "In Progress".to_owned(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn assignee_projection_serializes_type_as_json_key() {
        let dto = make_actor_dto();
        let proj = AssigneeProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["type"], "user");
        assert_eq!(value["display_name"], "Alice");
    }

    #[test]
    fn assignee_projection_contract_fields() {
        let dto = make_actor_dto();
        let proj = AssigneeProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["type", "display_name"], &[]);
    }

    #[test]
    fn task_summary_projection_contract_fields() {
        let dto = make_task_summary_dto();
        let proj = TaskSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["readable_id", "title", "board_name", "column_name", "labels", "assignees", "updated_at"],
            &["priority", "estimate"],
        );
    }

    #[test]
    fn task_summary_assignees_contain_type_and_display_name() {
        let dto = make_task_summary_dto();
        let proj = TaskSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        let assignees = value["assignees"].as_array().unwrap();
        assert_eq!(assignees.len(), 1);
        assert_eq!(assignees[0]["type"], "user");
        assert_eq!(assignees[0]["display_name"], "Alice");
    }

    #[test]
    fn task_compact_projection_contract_fields() {
        let dto = make_task_dto("Dev Board", "To Do");
        let proj = TaskCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["readable_id", "title", "priority", "labels", "updated_at"],
            &["board_name", "column_name", "estimate", "due_date", "parent_task_id"],
        );
    }

    #[test]
    fn task_compact_board_and_column_omitted_when_empty() {
        let dto = make_task_dto("", "");
        let proj = TaskCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("board_name").is_none(), "board_name must be absent when empty");
        assert!(value.get("column_name").is_none(), "column_name must be absent when empty");
    }

    #[test]
    fn task_compact_board_and_column_present_when_non_empty() {
        let dto = make_task_dto("Dev Board", "To Do");
        let proj = TaskCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["board_name"], "Dev Board");
        assert_eq!(value["column_name"], "To Do");
    }

    #[test]
    fn task_full_projection_contract_fields() {
        let dto = make_task_dto("Dev Board", "To Do");
        let proj = TaskFullProjection::new(dto, Ok(vec![]), Ok(vec![]), Ok(vec![]));
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["readable_id", "title", "description", "labels", "updated_at"],
            &[
                "board_name", "column_name", "priority", "estimate", "due_date", "parent_task_id",
                "references", "references_error", "subtasks", "subtasks_error",
                "assignees", "assignees_error",
            ],
        );
    }

    #[test]
    fn task_full_projection_sub_resource_error_replaces_value() {
        let dto = make_task_dto("", "");
        let proj = TaskFullProjection::new(
            dto,
            Err("list_references failed: timeout".to_owned()),
            Ok(vec![]),
            Ok(vec![]),
        );
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("references").is_none(), "references must be absent on error");
        assert!(value["references_error"].is_string(), "references_error must be set");
        assert!(value.get("subtasks_error").is_none(), "subtasks_error must be absent on success");
        assert!(value["subtasks"].is_array(), "subtasks must be present on success");
    }

    #[test]
    fn delete_task_projection_serializes_readable_id_not_id() {
        let proj = DeleteTaskProjection {
            deleted: true,
            readable_id: "ATL-1".to_owned(),
        };
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["deleted"], true);
        assert_eq!(value["readable_id"], "ATL-1");
        assert!(value.get("id").is_none(), "must not contain a generic 'id' key");
        assert_projection_fields(&value, &["deleted", "readable_id"], &[]);
    }

    // -----------------------------------------------------------------------
    // Document projections (T36)
    // -----------------------------------------------------------------------

    fn make_document_dto(slug: Option<&str>, folder_id: Option<Uuid>) -> DocumentDto {
        DocumentDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Some(Uuid::now_v7()),
            folder_id,
            slug: slug.map(str::to_owned),
            title: "Test Document".to_owned(),
            content: "# Hello\nWorld".to_owned(),
            head_revision_id: Uuid::now_v7(),
            head_seq: 5,
            frontmatter: serde_json::json!({"tags": ["rust"]}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_document_summary_dto(slug: Option<&str>) -> DocumentSummaryDto {
        DocumentSummaryDto {
            id: Uuid::now_v7(),
            slug: slug.map(str::to_owned),
            title: "Summary Doc".to_owned(),
            folder_id: None,
            head_seq: 3,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn doc_compact_projection_contract_fields() {
        let dto = make_document_dto(Some("my-doc"), None);
        let proj = DocCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "slug", "title", "head_revision_id", "head_seq", "updated_at", "folder_id", "project_id"],
            &[],
        );
    }

    #[test]
    fn doc_compact_projection_slug_null_when_none() {
        let dto = make_document_dto(None, None);
        let proj = DocCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value["slug"].is_null(), "slug must be null (not absent) when None");
    }

    #[test]
    fn doc_compact_projection_folder_id_null_when_none() {
        let dto = make_document_dto(Some("x"), None);
        let proj = DocCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value["folder_id"].is_null(), "folder_id must be null when None");
    }

    #[test]
    fn doc_full_projection_contract_fields() {
        let dto = make_document_dto(Some("full-doc"), None);
        let proj = DocFullProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "id", "slug", "title", "head_revision_id", "head_seq", "updated_at",
                "folder_id", "project_id", "content", "frontmatter",
            ],
            &[],
        );
    }

    #[test]
    fn doc_full_projection_content_and_frontmatter_present() {
        let dto = make_document_dto(Some("doc"), None);
        let proj = DocFullProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value["content"].is_string());
        assert!(value["frontmatter"].is_object());
    }

    #[test]
    fn delete_doc_projection_serializes_slug_not_id() {
        let proj = DeleteDocProjection {
            deleted: true,
            slug: "my-doc".to_owned(),
        };
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["deleted"], true);
        assert_eq!(value["slug"], "my-doc");
        assert!(value.get("id").is_none(), "must not contain a generic 'id' key");
        assert_projection_fields(&value, &["deleted", "slug"], &[]);
    }

    #[test]
    fn doc_summary_projection_contract_fields_required() {
        let dto = make_document_summary_dto(None);
        let proj = DocSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "title", "head_seq", "updated_at"],
            &["slug", "folder_id"],
        );
    }

    #[test]
    fn doc_summary_projection_slug_absent_when_none() {
        let dto = make_document_summary_dto(None);
        let proj = DocSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("slug").is_none(), "slug must be absent (not null) when None");
    }

    #[test]
    fn doc_summary_projection_slug_present_when_some() {
        let dto = make_document_summary_dto(Some("notes"));
        let proj = DocSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["slug"], "notes");
    }
}
