//! Pure projection and pagination helpers for MCP tool responses.
//!
//! All functions in this module are synchronous and have no I/O dependency,
//! making them fully unit-testable without a live server. Tool bodies in
//! `lib.rs` delegate all data-shaping work here so the testable surface is
//! maximised.

use atlas_api::{
    dtos::{
        boards_tasks::{
            BoardSummaryDto, ColumnDto, ReferenceDto, TaskBacklinkDto, TaskDto, TaskSummaryDto,
        },
        documents::{BacklinkDto, DocumentDto, DocumentSummaryDto},
        folders::FolderDto,
        saved_searches::SavedSearchDto,
        search::SearchHitDto,
        tags::TagDto,
        task_views::TaskViewDto,
        {PrincipalDto, ProjectDto, WorkspaceDto},
    },
    pagination::Page,
};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------
// Detail level
// ---------------------------------------------------------------------------

/// Whether to emit the compact (metadata-only) or full (adds heavy fields)
/// projection of a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detail {
    Compact,
    Full,
}

/// Parses the `detail` tool parameter.
///
/// Accepts `"full"` (case-insensitive) to opt in; anything else (including
/// absent) yields `Compact`. Lenient: unknown strings default to compact so
/// agents cannot accidentally break a call with a typo.
pub(crate) fn parse_detail(value: Option<&str>) -> Detail {
    match value {
        Some(s) if s.eq_ignore_ascii_case("full") => Detail::Full,
        _ => Detail::Compact,
    }
}

// ---------------------------------------------------------------------------
// CSV / list param parsing
// ---------------------------------------------------------------------------

/// Splits a comma-separated string into a trimmed, non-empty `Vec<String>`.
///
/// Empty input or whitespace-only input yields an empty vec.
/// Individual items that are blank after trimming are skipped.
pub(crate) fn parse_csv(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(String::from)
        .collect()
}

// ---------------------------------------------------------------------------
// Pagination envelope
// ---------------------------------------------------------------------------

/// Wraps a `Page<Value>` into the uniform agent-facing envelope:
/// `{"items":[...], "next_cursor": <string|null>, "has_more": <bool>}`.
///
/// `Page<T>` already carries `next_cursor` and `has_more`; this function just
/// re-serialises its already-projected `items` into the envelope shape.
pub(crate) fn paginated_envelope(
    items: Vec<Value>,
    next_cursor: Option<String>,
    has_more: bool,
) -> Value {
    json!({
        "items": items,
        "next_cursor": next_cursor,
        "has_more": has_more,
    })
}

/// Convenience: build a `Page<T>` envelope helper for endpoints that return a
/// `Page<T>`. Projects each item with `project_fn`, then wraps in the envelope.
pub(crate) fn envelope_page<T, F>(page: Page<T>, project_fn: F) -> Value
where
    F: Fn(T) -> Value,
{
    let items: Vec<Value> = page.items.into_iter().map(project_fn).collect();
    paginated_envelope(items, page.next_cursor, page.has_more)
}

/// Convenience: build the envelope for `Vec`-returning endpoints (no cursor).
///
/// These are always returned with `next_cursor: null` and `has_more: false` so
/// the agent never needs to special-case them.
pub(crate) fn wrap_vec<T, F>(items: Vec<T>, project_fn: F) -> Value
where
    F: Fn(T) -> Value,
{
    let projected: Vec<Value> = items.into_iter().map(project_fn).collect();
    paginated_envelope(projected, None, false)
}

// ---------------------------------------------------------------------------
// Column-name resolver
// ---------------------------------------------------------------------------

/// Returns the UUIDs of all columns whose name contains `name_fragment` as a
/// case-insensitive substring.
///
/// Returns an empty vec when no column matches — callers should propagate this
/// as an empty result rather than an error (the agent supplied an unrecognised
/// status name). Multi-match is intentional: a workspace may have multiple
/// boards each with a column called "To Do"; all matching UUIDs are returned so
/// the filter covers all of them.
pub(crate) fn match_columns_by_name(name_fragment: &str, cols: &[ColumnDto]) -> Vec<String> {
    let needle = name_fragment.to_ascii_lowercase();
    cols.iter()
        .filter(|col| col.name.to_ascii_lowercase().contains(&needle))
        .map(|col| col.id.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Search hit projection
// ---------------------------------------------------------------------------

/// Compact projection of a search hit.
///
/// The `kind` enum is lowercased to a plain string (`"document"`, `"task"`).
/// `readable_id`, `snippet`, and `project_slug` are absent when `None`.
pub(crate) fn project_search_hit(hit: SearchHitDto) -> Value {
    let kind = format!("{:?}", hit.kind).to_lowercase();

    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(hit.id));
    map.insert("kind".into(), json!(kind));
    map.insert("title".into(), json!(hit.title));
    map.insert("score".into(), json!(hit.score));
    map.insert("updated_at".into(), json!(hit.updated_at));

    if let Some(rid) = hit.readable_id {
        map.insert("readable_id".into(), json!(rid));
    }
    if let Some(snip) = hit.snippet {
        map.insert("snippet".into(), json!(snip));
    }
    if let Some(slug) = hit.project_slug {
        map.insert("project_slug".into(), json!(slug));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Document projections
// ---------------------------------------------------------------------------

/// Compact projection: identifying metadata only; content and frontmatter omitted.
pub(crate) fn project_document_compact(doc: DocumentDto) -> Value {
    json!({
        "id": doc.id,
        "slug": doc.slug,
        "title": doc.title,
        "head_seq": doc.head_seq,
        "updated_at": doc.updated_at,
        "folder_id": doc.folder_id,
        "project_id": doc.project_id,
    })
}

/// Full projection: compact fields plus markdown content and frontmatter.
pub(crate) fn project_document_full(doc: DocumentDto) -> Value {
    json!({
        "id": doc.id,
        "slug": doc.slug,
        "title": doc.title,
        "head_seq": doc.head_seq,
        "updated_at": doc.updated_at,
        "folder_id": doc.folder_id,
        "project_id": doc.project_id,
        "content": doc.content,
        "frontmatter": doc.frontmatter,
    })
}

// ---------------------------------------------------------------------------
// Task row projection (from TaskSummaryDto — list_tasks rows)
// ---------------------------------------------------------------------------

/// Compact projection of a task list row.
///
/// `TaskSummaryDto` already carries human-readable `board_name` and
/// `column_name`, so the agent sees names rather than raw UUIDs in list output.
pub(crate) fn project_task_row(task: TaskSummaryDto) -> Value {
    let assignees: Vec<Value> = task
        .assignees
        .into_iter()
        .map(|a| {
            json!({
                "type": a.r#type,
                "display_name": a.display_name,
            })
        })
        .collect();

    json!({
        "readable_id": task.readable_id,
        "title": task.title,
        "board_name": task.board_name,
        "column_name": task.column_name,
        "priority": task.priority,
        "labels": task.labels,
        "estimate": task.estimate,
        "assignees": assignees,
        "updated_at": task.updated_at,
    })
}

// ---------------------------------------------------------------------------
// Task projections (from TaskDto — get_task)
// ---------------------------------------------------------------------------

/// Compact projection of a full task.
///
/// `TaskDto` lacks `column_name` and `board_name` (API debt D-TASKNAME); the
/// raw `column_id` UUID is included so the agent can resolve the name via
/// `list_columns` if needed.
pub(crate) fn project_task_compact(task: &TaskDto) -> Value {
    json!({
        "readable_id": task.readable_id,
        "title": task.title,
        "column_id": task.column_id,
        "priority": task.priority,
        "labels": task.labels,
        "estimate": task.estimate,
        "due_date": task.due_date,
        "parent_task_id": task.parent_task_id,
        "updated_at": task.updated_at,
    })
}

/// Full projection: compact fields plus description and derived sub-resources.
///
/// `references` and `subtasks` are optional: when the backing call fails a
/// step-attribution error field is included instead of failing the whole
/// response. Callers supply pre-projected values or error strings.
pub(crate) fn project_task_full(
    task: &TaskDto,
    references: Result<Vec<Value>, String>,
    subtasks: Result<Vec<Value>, String>,
) -> Value {
    let compact = project_task_compact(task);

    let mut map = match compact {
        Value::Object(m) => m,
        other => {
            let mut m = serde_json::Map::new();
            m.insert("_raw".into(), other);
            m
        }
    };

    map.insert("description".into(), json!(task.description));

    match references {
        Ok(refs) => {
            map.insert("references".into(), json!(refs));
        }
        Err(e) => {
            map.insert("references_error".into(), json!(e));
        }
    }

    match subtasks {
        Ok(subs) => {
            map.insert("subtasks".into(), json!(subs));
        }
        Err(e) => {
            map.insert("subtasks_error".into(), json!(e));
        }
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Document summary projection (list_documents rows)
// ---------------------------------------------------------------------------

/// Compact projection of a document list row.
///
/// All fields on `DocumentSummaryDto` are cheap; `folder_id` is omitted when
/// absent to keep the output uncluttered.
pub(crate) fn project_document_summary(doc: DocumentSummaryDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(doc.id));
    map.insert("title".into(), json!(doc.title));
    map.insert("head_seq".into(), json!(doc.head_seq));
    map.insert("updated_at".into(), json!(doc.updated_at));

    if let Some(slug) = doc.slug {
        map.insert("slug".into(), json!(slug));
    }
    if let Some(folder_id) = doc.folder_id {
        map.insert("folder_id".into(), json!(folder_id));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Folder projection (list_folders rows)
// ---------------------------------------------------------------------------

/// Compact projection of a folder list row.
///
/// `workspace_id`, `project_id`, and `created_at` are dropped — the agent
/// works within a scoped project context where these are implicit.
/// `parent_folder_id` is retained so the agent can reconstruct the tree.
pub(crate) fn project_folder(folder: FolderDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(folder.id));
    map.insert("name".into(), json!(folder.name));
    map.insert("updated_at".into(), json!(folder.updated_at));

    if let Some(parent) = folder.parent_folder_id {
        map.insert("parent_folder_id".into(), json!(parent));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Board summary projection (list_boards rows)
// ---------------------------------------------------------------------------

/// Compact projection of a board list row.
///
/// `created_at` is dropped; `id` and `name` are what the agent needs to
/// reference a board (by name for display, by id for `list_columns`).
pub(crate) fn project_board_summary(board: BoardSummaryDto) -> Value {
    json!({
        "id": board.id,
        "name": board.name,
        "updated_at": board.updated_at,
    })
}

// ---------------------------------------------------------------------------
// Column projection (list_columns rows)
// ---------------------------------------------------------------------------

/// Compact projection of a board column.
///
/// `board_id`, `position_key`, and timestamps are dropped. `color` is omitted
/// when absent. The `id` + `name` pair is the primary value: `id` is passed
/// to status filters and `name` is the human-readable label.
pub(crate) fn project_column(col: ColumnDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(col.id));
    map.insert("name".into(), json!(col.name));

    if let Some(color) = col.color {
        map.insert("color".into(), json!(color));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Reference projection
// ---------------------------------------------------------------------------

/// Projects an outbound task reference to the compact MCP shape.
pub(crate) fn project_reference(r: ReferenceDto) -> Value {
    json!({
        "kind": r.kind,
        "target_readable_id": r.target_readable_id,
        "target_document_id": r.target_document_id,
        "target_title": r.target_title,
        "target_resolved": r.target_resolved,
    })
}

// ---------------------------------------------------------------------------
// Tag projection (list_tags rows)
// ---------------------------------------------------------------------------

/// Compact projection of a workspace tag.
///
/// `workspace_id` and timestamps are dropped; `color` is omitted when absent.
pub(crate) fn project_tag(tag: TagDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(tag.id));
    map.insert("name".into(), json!(tag.name));

    if let Some(color) = tag.color {
        map.insert("color".into(), json!(color));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Principal projection (list_members rows)
// ---------------------------------------------------------------------------

/// Compact projection of a workspace member or API-key principal.
///
/// Exposes `principal_type`, `id`, and `display` — the minimum needed to
/// resolve a human name to the id format required by assignee filters.
pub(crate) fn project_principal(p: PrincipalDto) -> Value {
    json!({
        "principal_type": p.principal_type,
        "id": p.id,
        "display": p.display,
    })
}

// ---------------------------------------------------------------------------
// Workspace projection (list_workspaces rows)
// ---------------------------------------------------------------------------

/// Compact projection of a workspace.
///
/// `created_at` is dropped; the agent needs the slug to scope subsequent calls.
pub(crate) fn project_workspace(ws: WorkspaceDto) -> Value {
    json!({
        "id": ws.id,
        "name": ws.name,
        "slug": ws.slug,
        "updated_at": ws.updated_at,
    })
}

// ---------------------------------------------------------------------------
// Project projection (list_projects rows)
// ---------------------------------------------------------------------------

/// Compact projection of a project.
///
/// `workspace_id` and `created_at` are dropped. `visibility_role` is omitted
/// when absent (only present on non-public projects with an explicit grant role).
pub(crate) fn project_project(p: ProjectDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(p.id));
    map.insert("name".into(), json!(p.name));
    map.insert("slug".into(), json!(p.slug));
    map.insert("task_prefix".into(), json!(p.task_prefix));
    map.insert("visibility".into(), json!(p.visibility));
    map.insert("updated_at".into(), json!(p.updated_at));

    if let Some(role) = p.visibility_role {
        map.insert("visibility_role".into(), json!(role));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Saved search projection (list_saved_searches rows)
// ---------------------------------------------------------------------------

/// Compact projection of a saved search.
///
/// `workspace_id` and timestamps are dropped; `query` is retained so the agent
/// can inspect or reuse the filter string directly.
pub(crate) fn project_saved_search(s: SavedSearchDto) -> Value {
    json!({
        "id": s.id,
        "name": s.name,
        "query": s.query,
    })
}

// ---------------------------------------------------------------------------
// Task view projection (list_task_views rows)
// ---------------------------------------------------------------------------

/// Compact projection of a saved task view.
///
/// `workspace_id` and timestamps are dropped; `filters` is passed through
/// verbatim — it is already a small structured object with skip_serializing_if
/// guards that keep absent fields out.
pub(crate) fn project_task_view(v: TaskViewDto) -> Value {
    json!({
        "id": v.id,
        "name": v.name,
        "filters": v.filters,
    })
}

// ---------------------------------------------------------------------------
// Task backlink projection (get_task_backlinks rows)
// ---------------------------------------------------------------------------

/// Projects an inbound task backlink to the compact MCP shape.
///
/// `source_task_id` (UUID) is dropped; the agent uses `source_readable_id` to
/// navigate to the source task.
pub(crate) fn project_task_backlink(b: TaskBacklinkDto) -> Value {
    json!({
        "source_readable_id": b.source_readable_id,
        "source_title": b.source_title,
        "kind": b.kind,
    })
}

// ---------------------------------------------------------------------------
// Document backlink projection (get_document_backlinks rows)
// ---------------------------------------------------------------------------

/// Projects a document backlink to the compact MCP shape.
///
/// `source_document_id` (UUID) is dropped in favour of `source_slug` when
/// present. `display_title` is the rendered link text, which may differ from
/// the source document's title when the author used a custom alias.
pub(crate) fn project_backlink(b: BacklinkDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("source_title".into(), json!(b.source_title));
    map.insert("display_title".into(), json!(b.display_title));

    if let Some(slug) = b.source_slug {
        map.insert("source_slug".into(), json!(slug));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use atlas_api::dtos::{
        PrincipalDto, ProjectDto, WorkspaceDto,
        boards_tasks::{BoardSummaryDto, ColumnDto, ReferenceDto, TaskDto, TaskSummaryDto},
        documents::{ActorDto, DocumentDto, DocumentSummaryDto},
        folders::FolderDto,
        saved_searches::SavedSearchDto,
        search::{SearchHitDto, SearchKindDto},
        tags::TagDto,
        task_views::{TaskViewDto, TaskViewFiltersDto},
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    fn fixed_uuid() -> Uuid {
        Uuid::parse_str("018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234").unwrap()
    }

    fn actor() -> ActorDto {
        ActorDto {
            r#type: "user".into(),
            id: fixed_uuid(),
            display_name: Some("Alice".into()),
        }
    }

    fn make_doc() -> DocumentDto {
        DocumentDto {
            id: fixed_uuid(),
            workspace_id: fixed_uuid(),
            project_id: Some(fixed_uuid()),
            folder_id: Some(fixed_uuid()),
            slug: Some("my-doc".into()),
            title: "My Doc".into(),
            content: "# Hello".into(),
            head_revision_id: fixed_uuid(),
            head_seq: 3,
            frontmatter: serde_json::json!({"author": "alice"}),
            created_at: now(),
            updated_at: now(),
        }
    }

    fn make_task_dto() -> TaskDto {
        TaskDto {
            id: fixed_uuid(),
            workspace_id: fixed_uuid(),
            project_id: fixed_uuid(),
            board_id: fixed_uuid(),
            column_id: fixed_uuid(),
            parent_task_id: None,
            readable_id: "ATL-1".into(),
            title: "Fix bug".into(),
            description: "A long description".into(),
            priority: Some("high".into()),
            due_date: None,
            estimate: Some(3),
            labels: vec!["backend".into()],
            properties: None,
            created_by: actor(),
            created_at: now(),
            updated_at: now(),
        }
    }

    fn make_task_summary() -> TaskSummaryDto {
        TaskSummaryDto {
            id: fixed_uuid(),
            readable_id: "ATL-2".into(),
            board_id: fixed_uuid(),
            column_id: fixed_uuid(),
            title: "Task two".into(),
            priority: Some("low".into()),
            estimate: None,
            labels: vec![],
            assignees: vec![actor()],
            board_name: "Main Board".into(),
            column_name: "To Do".into(),
            updated_at: now(),
        }
    }

    // -----------------------------------------------------------------------
    // parse_detail
    // -----------------------------------------------------------------------

    #[test]
    fn parse_detail_absent_yields_compact() {
        assert_eq!(parse_detail(None), Detail::Compact);
    }

    #[test]
    fn parse_detail_empty_string_yields_compact() {
        assert_eq!(parse_detail(Some("")), Detail::Compact);
    }

    #[test]
    fn parse_detail_full_lowercase_yields_full() {
        assert_eq!(parse_detail(Some("full")), Detail::Full);
    }

    #[test]
    fn parse_detail_full_uppercase_yields_full() {
        assert_eq!(parse_detail(Some("FULL")), Detail::Full);
    }

    #[test]
    fn parse_detail_compact_explicit_yields_compact() {
        assert_eq!(parse_detail(Some("compact")), Detail::Compact);
    }

    #[test]
    fn parse_detail_unknown_yields_compact() {
        assert_eq!(parse_detail(Some("extended")), Detail::Compact);
    }

    // -----------------------------------------------------------------------
    // parse_csv
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
    // paginated_envelope / wrap_vec
    // -----------------------------------------------------------------------

    #[test]
    fn paginated_envelope_with_cursor() {
        let val = paginated_envelope(vec![json!({"id": 1})], Some("next-page-token".into()), true);
        assert_eq!(val["next_cursor"], "next-page-token");
        assert_eq!(val["has_more"], true);
        assert_eq!(val["items"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn paginated_envelope_without_cursor() {
        let val = paginated_envelope(vec![], None, false);
        assert!(val["next_cursor"].is_null());
        assert_eq!(val["has_more"], false);
        assert!(val["items"].as_array().unwrap().is_empty());
    }

    #[test]
    fn wrap_vec_always_has_null_cursor() {
        let cols = vec![json!({"name": "a"}), json!({"name": "b"})];
        let val = paginated_envelope(cols, None, false);
        assert!(val["next_cursor"].is_null());
        assert_eq!(val["has_more"], false);
        assert_eq!(val["items"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn envelope_page_threads_page_fields() {
        use atlas_api::pagination::Page;
        let page: Page<u32> = Page {
            items: vec![1, 2],
            next_cursor: Some("tok".into()),
            has_more: true,
        };
        let val = envelope_page(page, |n| json!(n));
        assert_eq!(val["next_cursor"], "tok");
        assert_eq!(val["has_more"], true);
        assert_eq!(val["items"], json!([1, 2]));
    }

    // -----------------------------------------------------------------------
    // match_columns_by_name
    // -----------------------------------------------------------------------

    fn make_column(name: &str) -> ColumnDto {
        ColumnDto {
            id: Uuid::now_v7(),
            board_id: fixed_uuid(),
            name: name.into(),
            position_key: "a".into(),
            color: None,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn match_columns_exact_match() {
        let cols = vec![make_column("To Do"), make_column("Done")];
        let ids = match_columns_by_name("To Do", &cols);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], cols[0].id.to_string());
    }

    #[test]
    fn match_columns_partial_match() {
        let cols = vec![
            make_column("Todo"),
            make_column("Todo Later"),
            make_column("Done"),
        ];
        let ids = match_columns_by_name("todo", &cols);
        assert_eq!(ids.len(), 2, "both 'Todo' and 'Todo Later' match 'todo'");
    }

    #[test]
    fn match_columns_case_insensitive() {
        let cols = vec![make_column("In Progress"), make_column("Blocked")];
        let ids = match_columns_by_name("IN PROGRESS", &cols);
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn match_columns_no_match_returns_empty() {
        let cols = vec![make_column("To Do"), make_column("Done")];
        let ids = match_columns_by_name("nonexistent", &cols);
        assert!(ids.is_empty());
    }

    #[test]
    fn match_columns_multi_match_across_boards() {
        // Two boards each have a "To Do" column with distinct UUIDs.
        let c1 = make_column("To Do");
        let c2 = make_column("To Do");
        assert_ne!(c1.id, c2.id, "test setup: UUIDs must differ");
        let ids = match_columns_by_name("to do", &[c1.clone(), c2.clone()]);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&c1.id.to_string()));
        assert!(ids.contains(&c2.id.to_string()));
    }

    // -----------------------------------------------------------------------
    // project_search_hit
    // -----------------------------------------------------------------------

    #[test]
    fn search_hit_compact_task_fields() {
        let hit = SearchHitDto {
            id: fixed_uuid(),
            kind: SearchKindDto::Task,
            readable_id: Some("ATL-5".into()),
            title: "Test task".into(),
            snippet: Some("<mark>test</mark>".into()),
            score: 0.8,
            updated_at: now(),
            project_slug: Some("my-project".into()),
        };
        let val = project_search_hit(hit);
        assert_eq!(val["kind"], "task");
        assert_eq!(val["readable_id"], "ATL-5");
        assert_eq!(val["snippet"], "<mark>test</mark>");
        assert_eq!(val["project_slug"], "my-project");
        // must NOT include workspace_id or other heavy fields
        assert!(val.get("workspace_id").is_none());
    }

    #[test]
    fn search_hit_document_omits_absent_optionals() {
        let hit = SearchHitDto {
            id: fixed_uuid(),
            kind: SearchKindDto::Document,
            readable_id: None,
            title: "Doc".into(),
            snippet: None,
            score: 0.5,
            updated_at: now(),
            project_slug: None,
        };
        let val = project_search_hit(hit);
        assert_eq!(val["kind"], "document");
        assert!(val.get("readable_id").is_none());
        assert!(val.get("snippet").is_none());
        assert!(val.get("project_slug").is_none());
    }

    // -----------------------------------------------------------------------
    // project_document_compact / full
    // -----------------------------------------------------------------------

    #[test]
    fn document_compact_omits_content_and_frontmatter() {
        let doc = make_doc();
        let val = project_document_compact(doc);
        assert!(
            val.get("content").is_none(),
            "compact must not include content"
        );
        assert!(
            val.get("frontmatter").is_none(),
            "compact must not include frontmatter"
        );
        assert_eq!(val["title"], "My Doc");
        assert_eq!(val["head_seq"], 3);
    }

    #[test]
    fn document_compact_includes_slug_folder_project() {
        let doc = make_doc();
        let val = project_document_compact(doc);
        assert_eq!(val["slug"], "my-doc");
        assert!(!val["folder_id"].is_null());
        assert!(!val["project_id"].is_null());
    }

    #[test]
    fn document_full_includes_content_and_frontmatter() {
        let doc = make_doc();
        let val = project_document_full(doc);
        assert_eq!(val["content"], "# Hello");
        assert_eq!(val["frontmatter"]["author"], "alice");
    }

    #[test]
    fn document_full_includes_all_compact_fields() {
        let doc = make_doc();
        let val = project_document_full(doc);
        assert_eq!(val["title"], "My Doc");
        assert_eq!(val["head_seq"], 3);
        assert!(!val["folder_id"].is_null());
    }

    // -----------------------------------------------------------------------
    // project_task_row
    // -----------------------------------------------------------------------

    #[test]
    fn task_row_includes_names_not_uuids() {
        let summary = make_task_summary();
        let val = project_task_row(summary);
        assert_eq!(val["board_name"], "Main Board");
        assert_eq!(val["column_name"], "To Do");
        // Raw UUIDs must NOT be present in the row output
        assert!(val.get("board_id").is_none());
        assert!(val.get("column_id").is_none());
        assert!(val.get("id").is_none());
    }

    #[test]
    fn task_row_projects_assignees() {
        let summary = make_task_summary();
        let val = project_task_row(summary);
        let assignees = val["assignees"].as_array().unwrap();
        assert_eq!(assignees.len(), 1);
        assert_eq!(assignees[0]["type"], "user");
        assert_eq!(assignees[0]["display_name"], "Alice");
    }

    // -----------------------------------------------------------------------
    // project_task_compact / full
    // -----------------------------------------------------------------------

    #[test]
    fn task_compact_omits_description() {
        let task = make_task_dto();
        let val = project_task_compact(&task);
        assert!(val.get("description").is_none());
    }

    #[test]
    fn task_compact_includes_column_id() {
        let task = make_task_dto();
        let val = project_task_compact(&task);
        assert!(!val["column_id"].is_null());
    }

    #[test]
    fn task_full_includes_description_and_references() {
        let task = make_task_dto();
        let refs = vec![json!({"kind": "relates", "target_resolved": true})];
        let val = project_task_full(&task, Ok(refs.clone()), Ok(vec![]));
        assert_eq!(val["description"], "A long description");
        assert_eq!(val["references"], json!(refs));
        assert!(val.get("references_error").is_none());
    }

    #[test]
    fn task_full_step_attribution_on_references_error() {
        let task = make_task_dto();
        let val = project_task_full(&task, Err("timeout".into()), Ok(vec![]));
        assert!(val.get("references").is_none());
        assert_eq!(val["references_error"], "timeout");
        // Task body is still present despite the sub-call failure
        assert_eq!(val["title"], "Fix bug");
    }

    #[test]
    fn task_full_step_attribution_on_subtasks_error() {
        let task = make_task_dto();
        let val = project_task_full(&task, Ok(vec![]), Err("not found".into()));
        assert!(val.get("subtasks").is_none());
        assert_eq!(val["subtasks_error"], "not found");
    }

    // -----------------------------------------------------------------------
    // project_reference
    // -----------------------------------------------------------------------

    #[test]
    fn reference_projection_includes_kind_and_resolved() {
        let r = ReferenceDto {
            id: fixed_uuid(),
            kind: "blocks".into(),
            target_task_id: None,
            target_readable_id: Some("ATL-3".into()),
            target_document_id: None,
            target_title: None,
            target_resolved: true,
            created_by: actor(),
            created_at: now(),
        };
        let val = project_reference(r);
        assert_eq!(val["kind"], "blocks");
        assert_eq!(val["target_readable_id"], "ATL-3");
        assert_eq!(val["target_resolved"], true);
        // Heavy fields dropped
        assert!(val.get("id").is_none());
        assert!(val.get("created_by").is_none());
    }

    // -----------------------------------------------------------------------
    // project_document_summary
    // -----------------------------------------------------------------------

    fn make_doc_summary(slug: Option<&str>, folder_id: Option<Uuid>) -> DocumentSummaryDto {
        DocumentSummaryDto {
            id: fixed_uuid(),
            slug: slug.map(String::from),
            title: "My Note".into(),
            folder_id,
            head_seq: 7,
            updated_at: now(),
        }
    }

    #[test]
    fn document_summary_includes_required_fields() {
        let val = project_document_summary(make_doc_summary(Some("my-note"), None));
        assert_eq!(val["title"], "My Note");
        assert_eq!(val["head_seq"], 7);
        assert!(!val["id"].is_null());
        assert!(!val["updated_at"].is_null());
    }

    #[test]
    fn document_summary_includes_slug_when_present() {
        let val = project_document_summary(make_doc_summary(Some("my-note"), None));
        assert_eq!(val["slug"], "my-note");
    }

    #[test]
    fn document_summary_omits_slug_when_absent() {
        let val = project_document_summary(make_doc_summary(None, None));
        assert!(val.get("slug").is_none());
    }

    #[test]
    fn document_summary_includes_folder_id_when_present() {
        let folder_id = fixed_uuid();
        let val = project_document_summary(make_doc_summary(None, Some(folder_id)));
        assert!(!val["folder_id"].is_null());
    }

    #[test]
    fn document_summary_omits_folder_id_when_absent() {
        let val = project_document_summary(make_doc_summary(None, None));
        assert!(val.get("folder_id").is_none());
    }

    // -----------------------------------------------------------------------
    // project_folder
    // -----------------------------------------------------------------------

    fn make_folder(parent: Option<Uuid>) -> FolderDto {
        FolderDto {
            id: fixed_uuid(),
            workspace_id: fixed_uuid(),
            project_id: Some(fixed_uuid()),
            parent_folder_id: parent,
            name: "Design".into(),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn folder_projection_includes_id_name_updated_at() {
        let val = project_folder(make_folder(None));
        assert_eq!(val["name"], "Design");
        assert!(!val["id"].is_null());
        assert!(!val["updated_at"].is_null());
    }

    #[test]
    fn folder_projection_drops_workspace_and_project_ids() {
        let val = project_folder(make_folder(None));
        assert!(val.get("workspace_id").is_none());
        assert!(val.get("project_id").is_none());
        assert!(val.get("created_at").is_none());
    }

    #[test]
    fn folder_projection_includes_parent_when_present() {
        let parent = Uuid::now_v7();
        let val = project_folder(make_folder(Some(parent)));
        assert_eq!(
            val["parent_folder_id"].as_str().unwrap(),
            parent.to_string()
        );
    }

    #[test]
    fn folder_projection_omits_parent_when_absent() {
        let val = project_folder(make_folder(None));
        assert!(val.get("parent_folder_id").is_none());
    }

    // -----------------------------------------------------------------------
    // project_board_summary
    // -----------------------------------------------------------------------

    fn make_board_summary() -> BoardSummaryDto {
        BoardSummaryDto {
            id: fixed_uuid(),
            name: "Sprint Board".into(),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn board_summary_includes_id_name_updated_at() {
        let val = project_board_summary(make_board_summary());
        assert_eq!(val["name"], "Sprint Board");
        assert!(!val["id"].is_null());
        assert!(!val["updated_at"].is_null());
    }

    #[test]
    fn board_summary_drops_created_at() {
        let val = project_board_summary(make_board_summary());
        assert!(val.get("created_at").is_none());
    }

    // -----------------------------------------------------------------------
    // project_column
    // -----------------------------------------------------------------------

    fn make_column_dto(color: Option<&str>) -> ColumnDto {
        ColumnDto {
            id: fixed_uuid(),
            board_id: fixed_uuid(),
            name: "In Progress".into(),
            position_key: "a0".into(),
            color: color.map(String::from),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn column_projection_includes_id_and_name() {
        let val = project_column(make_column_dto(None));
        assert_eq!(val["name"], "In Progress");
        assert!(!val["id"].is_null());
    }

    #[test]
    fn column_projection_drops_board_id_and_timestamps() {
        let val = project_column(make_column_dto(None));
        assert!(val.get("board_id").is_none());
        assert!(val.get("position_key").is_none());
        assert!(val.get("created_at").is_none());
        assert!(val.get("updated_at").is_none());
    }

    #[test]
    fn column_projection_includes_color_when_present() {
        let val = project_column(make_column_dto(Some("#FF5733")));
        assert_eq!(val["color"], "#FF5733");
    }

    #[test]
    fn column_projection_omits_color_when_absent() {
        let val = project_column(make_column_dto(None));
        assert!(val.get("color").is_none());
    }

    // -----------------------------------------------------------------------
    // project_tag
    // -----------------------------------------------------------------------

    fn make_tag(color: Option<&str>) -> TagDto {
        TagDto {
            id: fixed_uuid(),
            workspace_id: fixed_uuid(),
            name: "backend".into(),
            color: color.map(String::from),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn tag_projection_includes_id_and_name() {
        let val = project_tag(make_tag(None));
        assert_eq!(val["name"], "backend");
        assert!(!val["id"].is_null());
    }

    #[test]
    fn tag_projection_drops_workspace_id_and_timestamps() {
        let val = project_tag(make_tag(None));
        assert!(val.get("workspace_id").is_none());
        assert!(val.get("created_at").is_none());
        assert!(val.get("updated_at").is_none());
    }

    #[test]
    fn tag_projection_includes_color_when_present() {
        let val = project_tag(make_tag(Some("#3B82F6")));
        assert_eq!(val["color"], "#3B82F6");
    }

    #[test]
    fn tag_projection_omits_color_when_absent() {
        let val = project_tag(make_tag(None));
        assert!(val.get("color").is_none());
    }

    // -----------------------------------------------------------------------
    // project_principal
    // -----------------------------------------------------------------------

    fn make_principal(principal_type: &str) -> PrincipalDto {
        PrincipalDto {
            principal_type: principal_type.into(),
            id: fixed_uuid(),
            display: "Alice".into(),
        }
    }

    #[test]
    fn principal_projection_includes_all_three_fields() {
        let val = project_principal(make_principal("user"));
        assert_eq!(val["principal_type"], "user");
        assert!(!val["id"].is_null());
        assert_eq!(val["display"], "Alice");
    }

    #[test]
    fn principal_projection_works_for_api_key_type() {
        let val = project_principal(make_principal("api_key"));
        assert_eq!(val["principal_type"], "api_key");
    }

    // -----------------------------------------------------------------------
    // project_workspace
    // -----------------------------------------------------------------------

    fn make_workspace() -> WorkspaceDto {
        WorkspaceDto {
            id: fixed_uuid(),
            name: "My Workspace".into(),
            slug: "my-ws".into(),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn workspace_projection_includes_id_name_slug_updated_at() {
        let val = project_workspace(make_workspace());
        assert_eq!(val["name"], "My Workspace");
        assert_eq!(val["slug"], "my-ws");
        assert!(!val["id"].is_null());
        assert!(!val["updated_at"].is_null());
    }

    #[test]
    fn workspace_projection_drops_created_at() {
        let val = project_workspace(make_workspace());
        assert!(val.get("created_at").is_none());
    }

    // -----------------------------------------------------------------------
    // project_project
    // -----------------------------------------------------------------------

    fn make_project(visibility_role: Option<&str>) -> ProjectDto {
        ProjectDto {
            id: fixed_uuid(),
            workspace_id: fixed_uuid(),
            name: "Atlas".into(),
            slug: "atlas".into(),
            task_prefix: "ATL".into(),
            visibility: "workspace".into(),
            visibility_role: visibility_role.map(String::from),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn project_projection_includes_required_fields() {
        let val = project_project(make_project(None));
        assert_eq!(val["name"], "Atlas");
        assert_eq!(val["slug"], "atlas");
        assert_eq!(val["task_prefix"], "ATL");
        assert_eq!(val["visibility"], "workspace");
        assert!(!val["id"].is_null());
        assert!(!val["updated_at"].is_null());
    }

    #[test]
    fn project_projection_drops_workspace_id_and_created_at() {
        let val = project_project(make_project(None));
        assert!(val.get("workspace_id").is_none());
        assert!(val.get("created_at").is_none());
    }

    #[test]
    fn project_projection_includes_visibility_role_when_present() {
        let val = project_project(make_project(Some("editor")));
        assert_eq!(val["visibility_role"], "editor");
    }

    #[test]
    fn project_projection_omits_visibility_role_when_absent() {
        let val = project_project(make_project(None));
        assert!(val.get("visibility_role").is_none());
    }

    // -----------------------------------------------------------------------
    // project_saved_search
    // -----------------------------------------------------------------------

    fn make_saved_search() -> SavedSearchDto {
        SavedSearchDto {
            id: fixed_uuid(),
            workspace_id: fixed_uuid(),
            name: "Open bugs".into(),
            query: "status:open tag:bug".into(),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn saved_search_projection_includes_id_name_query() {
        let val = project_saved_search(make_saved_search());
        assert_eq!(val["name"], "Open bugs");
        assert_eq!(val["query"], "status:open tag:bug");
        assert!(!val["id"].is_null());
    }

    #[test]
    fn saved_search_projection_drops_workspace_id_and_timestamps() {
        let val = project_saved_search(make_saved_search());
        assert!(val.get("workspace_id").is_none());
        assert!(val.get("created_at").is_none());
        assert!(val.get("updated_at").is_none());
    }

    // -----------------------------------------------------------------------
    // project_task_view
    // -----------------------------------------------------------------------

    fn make_task_view(filters: TaskViewFiltersDto) -> TaskViewDto {
        TaskViewDto {
            id: fixed_uuid(),
            workspace_id: fixed_uuid(),
            name: "My open tasks".into(),
            filters,
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn task_view_projection_includes_id_name_and_filters() {
        let filters = TaskViewFiltersDto {
            sort: Some("updated_at_desc".into()),
            priorities: vec!["high".into()],
            ..Default::default()
        };
        let val = project_task_view(make_task_view(filters));
        assert_eq!(val["name"], "My open tasks");
        assert!(!val["id"].is_null());
        assert!(!val["filters"].is_null());
        assert_eq!(val["filters"]["sort"], "updated_at_desc");
    }

    #[test]
    fn task_view_projection_drops_workspace_id_and_timestamps() {
        let val = project_task_view(make_task_view(TaskViewFiltersDto::default()));
        assert!(val.get("workspace_id").is_none());
        assert!(val.get("created_at").is_none());
        assert!(val.get("updated_at").is_none());
    }

    #[test]
    fn task_view_filters_empty_vec_fields_omitted_in_output() {
        let val = project_task_view(make_task_view(TaskViewFiltersDto::default()));
        let filters = &val["filters"];
        assert!(
            filters.get("priorities").is_none(),
            "empty priorities Vec should be absent (skip_serializing_if)"
        );
        assert!(
            filters.get("labels").is_none(),
            "empty labels Vec should be absent"
        );
    }

    // -----------------------------------------------------------------------
    // project_task_backlink
    // -----------------------------------------------------------------------

    use atlas_api::dtos::boards_tasks::TaskBacklinkDto;

    fn make_task_backlink(kind: &str) -> TaskBacklinkDto {
        TaskBacklinkDto {
            source_task_id: fixed_uuid(),
            source_readable_id: "ATL-7".into(),
            source_title: "Blocker task".into(),
            kind: kind.into(),
        }
    }

    #[test]
    fn task_backlink_includes_readable_id_title_kind() {
        let val = project_task_backlink(make_task_backlink("blocks"));
        assert_eq!(val["source_readable_id"], "ATL-7");
        assert_eq!(val["source_title"], "Blocker task");
        assert_eq!(val["kind"], "blocks");
    }

    #[test]
    fn task_backlink_drops_source_task_id() {
        let val = project_task_backlink(make_task_backlink("relates"));
        assert!(val.get("source_task_id").is_none());
    }

    #[test]
    fn task_backlink_works_for_all_reference_kinds() {
        for kind in &["relates", "blocks", "parent", "spec"] {
            let val = project_task_backlink(make_task_backlink(kind));
            assert_eq!(val["kind"], *kind);
        }
    }

    // -----------------------------------------------------------------------
    // project_backlink
    // -----------------------------------------------------------------------

    use atlas_api::dtos::documents::BacklinkDto;

    fn make_backlink(slug: Option<&str>) -> BacklinkDto {
        BacklinkDto {
            source_document_id: fixed_uuid(),
            source_slug: slug.map(String::from),
            source_title: "Source Doc".into(),
            display_title: "Custom Link Text".into(),
        }
    }

    #[test]
    fn backlink_includes_source_title_and_display_title() {
        let val = project_backlink(make_backlink(Some("source-doc")));
        assert_eq!(val["source_title"], "Source Doc");
        assert_eq!(val["display_title"], "Custom Link Text");
    }

    #[test]
    fn backlink_includes_source_slug_when_present() {
        let val = project_backlink(make_backlink(Some("source-doc")));
        assert_eq!(val["source_slug"], "source-doc");
    }

    #[test]
    fn backlink_omits_source_slug_when_absent() {
        let val = project_backlink(make_backlink(None));
        assert!(val.get("source_slug").is_none());
    }

    #[test]
    fn backlink_drops_source_document_id() {
        let val = project_backlink(make_backlink(None));
        assert!(val.get("source_document_id").is_none());
    }
}
