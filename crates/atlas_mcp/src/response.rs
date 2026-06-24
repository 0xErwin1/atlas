//! Pure projection and pagination helpers for MCP tool responses.
//!
//! All functions in this module are synchronous and have no I/O dependency,
//! making them fully unit-testable without a live server. Tool bodies in
//! `lib.rs` delegate all data-shaping work here so the testable surface is
//! maximised.

use atlas_api::{
    dtos::{
        boards_tasks::{
            ActivityEntryDto, AssigneeDto, BoardSummaryDto, ChecklistItemDto, ColumnDto,
            ReferenceDto, TaskBacklinkDto, TaskDto, TaskSummaryDto,
        },
        documents::{
            ActorDto, AttachmentDto, BacklinkDto, DocumentDto, DocumentSummaryDto,
            RevisionContentDto, RevisionMetaDto,
        },
        folders::FolderDto,
        saved_searches::SavedSearchDto,
        search::SearchHitDto,
        status_templates::StatusTemplateDto,
        tags::TagDto,
        task_views::TaskViewDto,
        {PrincipalDto, ProjectDto, WorkspaceDto},
    },
    pagination::Page,
};
use atlas_client::ClientError;
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
    if let Some(col) = hit.column_name {
        map.insert("column_name".into(), json!(col));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Document projections
// ---------------------------------------------------------------------------

/// Compact projection: identifying metadata only; content and frontmatter omitted.
///
/// `head_revision_id` is the CAS token required by `update_document_content`.
/// It is included here so agents can read the token after any write without
/// needing a separate full-detail fetch.
pub(crate) fn project_document_compact(doc: DocumentDto) -> Value {
    json!({
        "id": doc.id,
        "slug": doc.slug,
        "title": doc.title,
        "head_revision_id": doc.head_revision_id,
        "head_seq": doc.head_seq,
        "updated_at": doc.updated_at,
        "folder_id": doc.folder_id,
        "project_id": doc.project_id,
    })
}

/// Full projection: compact fields plus markdown content and frontmatter.
///
/// `head_revision_id` enables the agent to proceed directly to
/// `update_document_content` after reading content, without an extra fetch.
pub(crate) fn project_document_full(doc: DocumentDto) -> Value {
    json!({
        "id": doc.id,
        "slug": doc.slug,
        "title": doc.title,
        "head_revision_id": doc.head_revision_id,
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
/// `board_name`/`column_name` are omitted when empty: the read path (`get_task`)
/// resolves them, but task mutation responses leave them blank, and an empty
/// name would misrepresent the task as having no board/column.
pub(crate) fn project_task_compact(task: &TaskDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("readable_id".into(), json!(task.readable_id));
    map.insert("title".into(), json!(task.title));

    if !task.board_name.is_empty() {
        map.insert("board_name".into(), json!(task.board_name));
    }
    if !task.column_name.is_empty() {
        map.insert("column_name".into(), json!(task.column_name));
    }

    map.insert("priority".into(), json!(task.priority));
    map.insert("labels".into(), json!(task.labels));
    map.insert("estimate".into(), json!(task.estimate));
    map.insert("due_date".into(), json!(task.due_date));
    map.insert("parent_task_id".into(), json!(task.parent_task_id));
    map.insert("updated_at".into(), json!(task.updated_at));

    Value::Object(map)
}

/// Full projection: compact fields plus description and derived sub-resources.
///
/// `references`, `subtasks`, and `assignees` are optional: when a backing call
/// fails a step-attribution error field is included instead of failing the
/// whole response. Callers supply pre-projected values or error strings.
pub(crate) fn project_task_full(
    task: &TaskDto,
    references: Result<Vec<Value>, String>,
    subtasks: Result<Vec<Value>, String>,
    assignees: Result<Vec<Value>, String>,
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

    match assignees {
        Ok(a) => {
            map.insert("assignees".into(), json!(a));
        }
        Err(e) => {
            map.insert("assignees_error".into(), json!(e));
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
// Status template projection
// ---------------------------------------------------------------------------

/// Compact projection of a workspace status template.
///
/// `workspace_id` and `created_at` are dropped; `color` is omitted when absent
/// since the DTO already marks it `skip_serializing_if = "Option::is_none"`.
/// `position_key` is retained so callers can use `before`/`after` anchors on
/// subsequent create/update calls.
pub(crate) fn project_status_template(t: StatusTemplateDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(t.id));
    map.insert("name".into(), json!(t.name));
    map.insert("position_key".into(), json!(t.position_key));
    map.insert("updated_at".into(), json!(t.updated_at));

    if let Some(color) = t.color {
        map.insert("color".into(), json!(color));
    }

    Value::Object(map)
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
// Actor helper (shared by revision, activity, attachment projections)
// ---------------------------------------------------------------------------

/// Projects an `ActorDto` to the compact MCP shape: `{type, display_name?}`.
///
/// `id` (UUID) is dropped; agents identify actors by type + display name.
/// `display_name` is omitted when the server sends `None` (e.g. deleted principals).
fn project_actor(a: ActorDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("type".into(), json!(a.r#type));

    if let Some(name) = a.display_name {
        map.insert("display_name".into(), json!(name));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Checklist item projection (list_checklist rows)
// ---------------------------------------------------------------------------

/// Projects a checklist item to the compact MCP shape.
///
/// `task_id` and `position_key` are internal ordering signals with no agent
/// value. `promoted_readable_id` is preserved so the agent can navigate to
/// the promoted task when the item has been converted.
pub(crate) fn project_checklist_item(item: ChecklistItemDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(item.id));
    map.insert("title".into(), json!(item.title));
    map.insert("checked".into(), json!(item.checked));

    if let Some(rid) = item.promoted_readable_id {
        map.insert("promoted_readable_id".into(), json!(rid));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Activity entry projection (list_activity rows)
// ---------------------------------------------------------------------------

/// Projects a task activity entry to the compact MCP shape.
///
/// `id` (UUID) is dropped; entries are identified by `kind` + `created_at`.
/// `payload` is passed through verbatim because its schema varies per `kind`
/// and the agent interprets it in context.
pub(crate) fn project_activity_entry(entry: ActivityEntryDto) -> Value {
    json!({
        "kind": entry.kind,
        "actor": project_actor(entry.actor),
        "payload": entry.payload,
        "created_at": entry.created_at,
    })
}

/// Projects a workspace activity entry to the compact MCP shape.
///
/// Includes `task_readable_id` so the agent can navigate to the task or
/// understand which task the event belongs to. `id` (UUID) is dropped.
pub(crate) fn project_workspace_activity_entry(entry: ActivityEntryDto) -> Value {
    json!({
        "task_readable_id": entry.task_readable_id,
        "kind": entry.kind,
        "actor": project_actor(entry.actor),
        "payload": entry.payload,
        "created_at": entry.created_at,
    })
}

// ---------------------------------------------------------------------------
// Document revision projections (list_document_history / get_document_revision)
// ---------------------------------------------------------------------------

/// Projects revision metadata (history list rows) to the compact MCP shape.
///
/// `id` (UUID) is dropped; the `seq` number is the stable handle for fetching
/// content via `get_document_revision`. `is_anchor` flags checkpoint revisions
/// that are always retained.
pub(crate) fn project_revision_meta(rev: RevisionMetaDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("seq".into(), json!(rev.seq));
    map.insert("is_anchor".into(), json!(rev.is_anchor));

    if let Some(actor) = rev.actor {
        map.insert("actor".into(), project_actor(actor));
    }

    map.insert("created_at".into(), json!(rev.created_at));

    Value::Object(map)
}

/// Projects a full revision's content to the MCP shape.
///
/// `id` (UUID) is dropped; `seq` + `content` are the load-bearing fields.
pub(crate) fn project_revision_content(rev: RevisionContentDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("seq".into(), json!(rev.seq));
    map.insert("content".into(), json!(rev.content));

    if let Some(actor) = rev.actor {
        map.insert("actor".into(), project_actor(actor));
    }

    map.insert("created_at".into(), json!(rev.created_at));

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Write-side: confirm guard
// ---------------------------------------------------------------------------

/// Enforces the caller's intent for destructive (non-auto-reversible) operations.
///
/// Returns `Ok(())` when `confirm` is `true`, or an actionable `Err` asking the
/// caller to re-invoke with `confirm: true`. Called before any client mutation so
/// the guard fires before any network round-trip.
pub(crate) fn require_confirm(confirm: bool, resource: &str, id: &str) -> Result<(), String> {
    if confirm {
        return Ok(());
    }
    Err(format!(
        "Refusing to delete {resource} '{id}'. \
         This is destructive and not auto-reversible. \
         Re-call with confirm: true to proceed."
    ))
}

// ---------------------------------------------------------------------------
// Write-side: column resolver (single-match, write-path semantics)
// ---------------------------------------------------------------------------

/// Resolves a column name to exactly one UUID on a given board.
///
/// Unlike `match_columns_by_name` (which returns all fuzzy matches for read
/// filters), this function enforces single-match semantics required by write
/// operations: 0 matches or >1 matches are both errors that include the board's
/// full column list so the caller can correct the name immediately.
pub(crate) fn resolve_column_id_on_board(
    name: &str,
    cols: &[ColumnDto],
) -> Result<uuid::Uuid, String> {
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
pub(crate) fn validate_priority(v: &str) -> Result<(), String> {
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
pub(crate) fn validate_assignee_type(v: &str) -> Result<(), String> {
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
pub(crate) fn validate_reference_kind(v: &str) -> Result<(), String> {
    match v {
        "relates" | "blocks" | "parent" | "spec" => Ok(()),
        _ => Err(format!(
            "invalid kind '{v}'; valid values: relates, blocks, parent, spec"
        )),
    }
}

/// Validates that exactly one of task or document target is supplied for a reference.
///
/// Returns `Ok(())` when exactly one target is present, or an `Err` explaining
/// that both/neither were supplied with instructions to correct the call.
pub(crate) fn validate_single_target(
    task: Option<&str>,
    doc: Option<&uuid::Uuid>,
) -> Result<(), String> {
    match (task, doc) {
        (Some(_), None) | (None, Some(_)) => Ok(()),
        (Some(_), Some(_)) => Err(
            "supply exactly one of target_task_readable_id or target_document_id, not both"
                .to_string(),
        ),
        (None, None) => Err(
            "supply exactly one of target_task_readable_id or target_document_id; neither was provided"
                .to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Promotion projection (promote_checklist_item)
// ---------------------------------------------------------------------------

/// Projects the `PromotionDto` returned by `promote_checklist_item`.
///
/// Includes the newly created task (compact), the checklist item that was
/// promoted (for confirmation), and the optional parent reference created.
pub(crate) fn project_promotion(p: atlas_api::dtos::boards_tasks::PromotionDto) -> Value {
    let mut map = serde_json::Map::new();

    map.insert("task".into(), project_task_compact(&p.task));
    map.insert(
        "checklist_item".into(),
        project_checklist_item(p.checklist_item),
    );

    if let Some(r) = p.parent_reference {
        map.insert("parent_reference".into(), project_reference(r));
    }

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Write-side: PATCH present_value mapping
// ---------------------------------------------------------------------------

/// Validator function signature for string field validators.
pub(crate) type FieldValidator = fn(&str) -> Result<(), String>;

/// Maps an MCP tri-state parameter to the present_value field in a PATCH request.
///
/// The tri-state contract: absent (`None`) = leave unchanged, `Some(Value::Null)` =
/// clear the field, `Some(string/number)` = set the field. When a `validator` is
/// supplied it runs only for non-null values; clearing always passes through.
///
/// Returns the ready-to-use `Option<Value>` to assign to the request DTO field.
pub(crate) fn map_present_value(
    field: Option<&serde_json::Value>,
    validator: Option<FieldValidator>,
) -> Result<Option<serde_json::Value>, String> {
    match field {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(Value::Null)),
        Some(v) => {
            if let (Some(validate), Some(s)) = (validator, v.as_str()) {
                validate(s)?;
            }
            Ok(Some(v.clone()))
        }
    }
}

/// Validates that an estimate value is non-negative.
///
/// Accepts any `i32 >= 0`. Returns an actionable error string for negative values.
/// Called before the client request so the failure is cheap and local.
pub(crate) fn validate_estimate(v: i32) -> Result<(), String> {
    if v < 0 {
        return Err(format!(
            "invalid estimate '{v}': must be a non-negative integer"
        ));
    }
    Ok(())
}

/// Validates an estimate carried as a `serde_json::Value` (used in PATCH paths).
///
/// Null (clear) and absent are allowed and pass through unchecked. Only a numeric
/// value that is negative is rejected.
pub(crate) fn validate_estimate_value(v: &serde_json::Value) -> Result<(), String> {
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
// MCP resource URI parsing
// ---------------------------------------------------------------------------

/// Parses an `atlas:///` document URI into `(workspace, slug)`.
///
/// Expected form: `atlas:///{workspace}/{slug}` where both segments are
/// non-empty. The `atlas:///` prefix uses three slashes (empty host, absolute
/// path), which is standard for custom opaque schemes. Extra path segments
/// beyond the first two are rejected.
///
/// Returns `(workspace, slug)` on success, or a descriptive error string on
/// any malformed input.
pub(crate) fn parse_atlas_doc_uri(uri: &str) -> Result<(String, String), String> {
    let path = uri
        .strip_prefix("atlas:///")
        .ok_or_else(|| format!("URI must start with 'atlas:///'; got: {uri}"))?;

    if path.is_empty() {
        return Err("URI path must not be empty after 'atlas:///'".to_string());
    }

    let mut parts = path.splitn(3, '/');

    let workspace = parts
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "URI missing workspace segment".to_string())?;

    let slug = parts.next().filter(|s| !s.is_empty()).ok_or_else(|| {
        "URI missing slug segment (expected 'atlas:///{workspace}/{slug}')".to_string()
    })?;

    if parts.next().is_some() {
        return Err(format!(
            "URI has too many path segments; expected exactly 2 (workspace/slug), got: {path}"
        ));
    }

    Ok((workspace.to_string(), slug.to_string()))
}

// ---------------------------------------------------------------------------
// Write-side: client error enrichment
// ---------------------------------------------------------------------------

/// Maps a `ClientError` to an agent-actionable string with context.
///
/// `ctx` identifies the operation (e.g. `"create_task"`) and is prefixed on
/// transport and decode errors where the server's response is unavailable. API
/// errors surface the server's `title` + `detail` + `hint` directly.
pub(crate) fn enrich_client_error(e: ClientError, ctx: &str) -> String {
    match e {
        ClientError::Api(p) => {
            let mut msg = format!("{}: {}", p.title, p.status);
            if let Some(detail) = p.detail {
                msg.push_str(&format!(" — {detail}"));
            }
            if let Some(hint) = p.hint {
                msg.push_str(&format!(" (hint: {hint})"));
            }
            msg
        }
        ClientError::Conflict(c) => {
            // Structured JSON so the agent can machine-read the patch and revision id.
            json!({
                "error": "revision_conflict",
                "message": "The document changed since you read it. Apply base_to_current_patch \
                            to your edit and retry update_document_content with \
                            base_revision_id = current_revision_id.",
                "current_revision_id": c.current_revision_id,
                "current_seq": c.current_seq,
                "base_to_current_patch": c.base_to_current_patch,
            })
            .to_string()
        }
        ClientError::Transport(e) => format!("{ctx}: transport error: {e}"),
        ClientError::Decode { context, source } => {
            format!("{ctx}: decode error in {context}: {source}")
        }
    }
}

// ---------------------------------------------------------------------------
// Write-side: assignee projection
// ---------------------------------------------------------------------------

/// Projects an `AssigneeDto` to the compact MCP shape.
pub(crate) fn project_assignee(a: AssigneeDto) -> Value {
    json!({
        "type": a.assignee.r#type,
        "display_name": a.assignee.display_name,
        "assigned_at": a.assigned_at,
    })
}

// ---------------------------------------------------------------------------
// Attachment projection (list_attachments rows)
// ---------------------------------------------------------------------------

/// Projects attachment metadata to the compact MCP shape.
///
/// `document_id` and `sha256` are server-internal identifiers with no agent
/// navigation value. `actor` is preserved so the agent knows who uploaded it.
pub(crate) fn project_attachment(att: AttachmentDto) -> Value {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(att.id));
    map.insert("file_name".into(), json!(att.file_name));
    map.insert("content_type".into(), json!(att.content_type));
    map.insert("size_bytes".into(), json!(att.size_bytes));

    if let Some(actor) = att.actor {
        map.insert("actor".into(), project_actor(actor));
    }

    map.insert("created_at".into(), json!(att.created_at));

    Value::Object(map)
}

// ---------------------------------------------------------------------------
// Audit entry projection
// ---------------------------------------------------------------------------

/// Projects a single `AuditEntryDto` to the compact MCP shape.
///
/// `id` (UUID) is dropped — internal database identifier with no agent value.
/// `workspace_id` is dropped — always implicit from the tool's workspace param.
/// The `actor` sub-object surfaces `type`, `display_name`, `key_type`, and
/// `account_status` so the agent can identify who acted and whether their
/// account is still active.
pub(crate) fn project_audit_entry(entry: atlas_api::dtos::audit::AuditEntryDto) -> Value {
    let mut actor_map = serde_json::Map::new();
    actor_map.insert("type".into(), json!(entry.actor.r#type));

    if let Some(name) = entry.actor.display_name {
        actor_map.insert("display_name".into(), json!(name));
    }
    if let Some(kt) = entry.actor.key_type {
        actor_map.insert("key_type".into(), json!(kt));
    }
    if let Some(status) = entry.actor.account_status {
        actor_map.insert("account_status".into(), json!(status));
    }

    let mut map = serde_json::Map::new();
    map.insert("action".into(), json!(entry.action));
    map.insert("actor".into(), Value::Object(actor_map));
    map.insert("target_type".into(), json!(entry.target_type));

    if let Some(tid) = entry.target_id {
        map.insert("target_id".into(), json!(tid));
    }
    if let Some(label) = entry.target_label {
        map.insert("target_label".into(), json!(label));
    }

    map.insert("metadata".into(), entry.metadata);
    map.insert("created_at".into(), json!(entry.created_at));

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
        boards_tasks::{
            ActivityEntryDto, BoardSummaryDto, ChecklistItemDto, ColumnDto, ReferenceDto, TaskDto,
            TaskSummaryDto,
        },
        documents::{
            ActorDto, AttachmentDto, DocumentDto, DocumentSummaryDto, RevisionContentDto,
            RevisionMetaDto,
        },
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
            key_type: None,
            account_status: None,
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
            board_name: "Sprint Board".into(),
            column_name: "In Review".into(),
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
            column_name: None,
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
            column_name: None,
        };
        let val = project_search_hit(hit);
        assert_eq!(val["kind"], "document");
        assert!(val.get("readable_id").is_none());
        assert!(val.get("snippet").is_none());
        assert!(val.get("project_slug").is_none());
    }

    #[test]
    fn search_hit_task_emits_column_name_when_some() {
        let hit = SearchHitDto {
            id: fixed_uuid(),
            kind: SearchKindDto::Task,
            readable_id: Some("ATL-7".into()),
            title: "Task in column".into(),
            snippet: None,
            score: 0.9,
            updated_at: now(),
            project_slug: None,
            column_name: Some("In Progress".into()),
        };
        let val = project_search_hit(hit);
        assert_eq!(val["column_name"], "In Progress");
    }

    #[test]
    fn search_hit_document_omits_column_name_when_none() {
        let hit = SearchHitDto {
            id: fixed_uuid(),
            kind: SearchKindDto::Document,
            readable_id: None,
            title: "Doc".into(),
            snippet: None,
            score: 0.5,
            updated_at: now(),
            project_slug: None,
            column_name: None,
        };
        let val = project_search_hit(hit);
        assert!(
            val.get("column_name").is_none(),
            "column_name must be absent when None"
        );
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
    fn document_compact_includes_head_revision_id() {
        let val = project_document_compact(make_doc());
        assert_eq!(
            val["head_revision_id"].as_str().unwrap(),
            fixed_uuid().to_string()
        );
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
    fn document_full_includes_head_revision_id() {
        let val = project_document_full(make_doc());
        assert_eq!(
            val["head_revision_id"].as_str().unwrap(),
            fixed_uuid().to_string()
        );
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
    fn task_compact_includes_board_name_and_column_name() {
        let task = make_task_dto();
        let val = project_task_compact(&task);
        assert_eq!(val["board_name"], "Sprint Board");
        assert_eq!(val["column_name"], "In Review");
        assert!(
            val.get("column_id").is_none(),
            "column_id UUID must not be emitted now that names are present"
        );
    }

    #[test]
    fn task_compact_omits_empty_board_and_column_names() {
        let mut task = make_task_dto();
        task.board_name = String::new();
        task.column_name = String::new();
        let val = project_task_compact(&task);
        assert!(val.get("board_name").is_none());
        assert!(val.get("column_name").is_none());
        assert!(val.get("readable_id").is_some());
    }

    #[test]
    fn task_full_includes_description_and_references() {
        let task = make_task_dto();
        let refs = vec![json!({"kind": "relates", "target_resolved": true})];
        let val = project_task_full(&task, Ok(refs.clone()), Ok(vec![]), Ok(vec![]));
        assert_eq!(val["description"], "A long description");
        assert_eq!(val["references"], json!(refs));
        assert!(val.get("references_error").is_none());
    }

    #[test]
    fn task_full_step_attribution_on_references_error() {
        let task = make_task_dto();
        let val = project_task_full(&task, Err("timeout".into()), Ok(vec![]), Ok(vec![]));
        assert!(val.get("references").is_none());
        assert_eq!(val["references_error"], "timeout");
        // Task body is still present despite the sub-call failure
        assert_eq!(val["title"], "Fix bug");
    }

    #[test]
    fn task_full_step_attribution_on_subtasks_error() {
        let task = make_task_dto();
        let val = project_task_full(&task, Ok(vec![]), Err("not found".into()), Ok(vec![]));
        assert!(val.get("subtasks").is_none());
        assert_eq!(val["subtasks_error"], "not found");
    }

    #[test]
    fn task_full_includes_assignees_when_ok() {
        let task = make_task_dto();
        let assignees = vec![json!({"id": "u1", "username": "alice"})];
        let val = project_task_full(&task, Ok(vec![]), Ok(vec![]), Ok(assignees.clone()));
        assert_eq!(val["assignees"], json!(assignees));
        assert!(val.get("assignees_error").is_none());
    }

    #[test]
    fn task_full_step_attribution_on_assignees_error() {
        let task = make_task_dto();
        let val = project_task_full(
            &task,
            Ok(vec![]),
            Ok(vec![]),
            Err("list_assignees failed: connection reset".into()),
        );
        assert!(val.get("assignees").is_none());
        assert_eq!(
            val["assignees_error"],
            "list_assignees failed: connection reset"
        );
        assert_eq!(val["title"], "Fix bug");
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
            key_type: None,
            role: None,
            account_status: None,
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

    // -----------------------------------------------------------------------
    // project_checklist_item
    // -----------------------------------------------------------------------

    fn make_actor(display: Option<&str>) -> ActorDto {
        ActorDto {
            r#type: "user".into(),
            id: fixed_uuid(),
            display_name: display.map(String::from),
            key_type: None,
            account_status: None,
        }
    }

    fn make_checklist_item(checked: bool, promoted: Option<&str>) -> ChecklistItemDto {
        ChecklistItemDto {
            id: fixed_uuid(),
            task_id: fixed_uuid(),
            title: "Do the thing".into(),
            checked,
            position_key: "aaa".into(),
            promoted_task_id: promoted.map(|_| fixed_uuid()),
            promoted_readable_id: promoted.map(String::from),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn checklist_item_includes_id_title_checked() {
        let val = project_checklist_item(make_checklist_item(false, None));
        assert!(val.get("id").is_some());
        assert_eq!(val["title"], "Do the thing");
        assert_eq!(val["checked"], false);
    }

    #[test]
    fn checklist_item_includes_promoted_readable_id_when_present() {
        let val = project_checklist_item(make_checklist_item(true, Some("ATL-99")));
        assert_eq!(val["promoted_readable_id"], "ATL-99");
    }

    #[test]
    fn checklist_item_omits_promoted_readable_id_when_absent() {
        let val = project_checklist_item(make_checklist_item(false, None));
        assert!(val.get("promoted_readable_id").is_none());
    }

    #[test]
    fn checklist_item_drops_task_id_and_position_key() {
        let val = project_checklist_item(make_checklist_item(false, None));
        assert!(val.get("task_id").is_none());
        assert!(val.get("position_key").is_none());
    }

    // -----------------------------------------------------------------------
    // project_activity_entry
    // -----------------------------------------------------------------------

    fn make_activity(kind: &str) -> ActivityEntryDto {
        ActivityEntryDto {
            id: fixed_uuid(),
            kind: kind.into(),
            actor: make_actor(Some("Alice")),
            payload: serde_json::json!({"column": "Done"}),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            task_id: fixed_uuid(),
            task_readable_id: "ATL-1".into(),
        }
    }

    #[test]
    fn activity_entry_includes_kind_actor_payload_created_at() {
        let val = project_activity_entry(make_activity("moved"));
        assert_eq!(val["kind"], "moved");
        assert_eq!(val["actor"]["type"], "user");
        assert_eq!(val["actor"]["display_name"], "Alice");
        assert!(val.get("payload").is_some());
        assert!(val.get("created_at").is_some());
    }

    #[test]
    fn activity_entry_drops_id() {
        let val = project_activity_entry(make_activity("created"));
        assert!(val.get("id").is_none());
    }

    // -----------------------------------------------------------------------
    // project_workspace_activity_entry
    // -----------------------------------------------------------------------

    #[test]
    fn workspace_activity_entry_includes_task_readable_id() {
        let val = project_workspace_activity_entry(make_activity("created"));
        assert_eq!(val["task_readable_id"], "ATL-1");
    }

    #[test]
    fn workspace_activity_entry_includes_kind_actor_payload_created_at() {
        let val = project_workspace_activity_entry(make_activity("moved"));
        assert_eq!(val["kind"], "moved");
        assert_eq!(val["actor"]["type"], "user");
        assert_eq!(val["actor"]["display_name"], "Alice");
        assert!(val.get("payload").is_some());
        assert!(val.get("created_at").is_some());
    }

    #[test]
    fn workspace_activity_entry_drops_id() {
        let val = project_workspace_activity_entry(make_activity("created"));
        assert!(val.get("id").is_none());
    }

    // -----------------------------------------------------------------------
    // project_revision_meta
    // -----------------------------------------------------------------------

    fn make_revision_meta(seq: i64, actor: Option<ActorDto>) -> RevisionMetaDto {
        RevisionMetaDto {
            id: fixed_uuid(),
            seq,
            is_anchor: seq == 1,
            actor,
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn revision_meta_includes_seq_is_anchor_created_at() {
        let val = project_revision_meta(make_revision_meta(5, None));
        assert_eq!(val["seq"], 5);
        assert_eq!(val["is_anchor"], false);
        assert!(val.get("created_at").is_some());
    }

    #[test]
    fn revision_meta_includes_actor_when_present() {
        let val = project_revision_meta(make_revision_meta(1, Some(make_actor(Some("Bob")))));
        assert_eq!(val["actor"]["display_name"], "Bob");
        assert_eq!(val["is_anchor"], true);
    }

    #[test]
    fn revision_meta_omits_actor_when_absent() {
        let val = project_revision_meta(make_revision_meta(2, None));
        assert!(val.get("actor").is_none());
    }

    #[test]
    fn revision_meta_drops_id() {
        let val = project_revision_meta(make_revision_meta(3, None));
        assert!(val.get("id").is_none());
    }

    // -----------------------------------------------------------------------
    // project_revision_content
    // -----------------------------------------------------------------------

    fn make_revision_content(seq: i64) -> RevisionContentDto {
        RevisionContentDto {
            id: fixed_uuid(),
            seq,
            content: "# Hello\nworld".into(),
            actor: Some(make_actor(Some("Carol"))),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn revision_content_includes_seq_content_actor_created_at() {
        let val = project_revision_content(make_revision_content(7));
        assert_eq!(val["seq"], 7);
        assert_eq!(val["content"], "# Hello\nworld");
        assert_eq!(val["actor"]["display_name"], "Carol");
        assert!(val.get("created_at").is_some());
    }

    #[test]
    fn revision_content_drops_id() {
        let val = project_revision_content(make_revision_content(7));
        assert!(val.get("id").is_none());
    }

    // -----------------------------------------------------------------------
    // project_attachment
    // -----------------------------------------------------------------------

    fn make_attachment(actor: Option<ActorDto>) -> AttachmentDto {
        AttachmentDto {
            id: fixed_uuid(),
            document_id: fixed_uuid(),
            file_name: "diagram.png".into(),
            content_type: "image/png".into(),
            size_bytes: 4096,
            sha256: "abc123".into(),
            actor,
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn attachment_includes_id_file_name_content_type_size_created_at() {
        let val = project_attachment(make_attachment(None));
        assert!(val.get("id").is_some());
        assert_eq!(val["file_name"], "diagram.png");
        assert_eq!(val["content_type"], "image/png");
        assert_eq!(val["size_bytes"], 4096);
        assert!(val.get("created_at").is_some());
    }

    #[test]
    fn attachment_includes_actor_when_present() {
        let val = project_attachment(make_attachment(Some(make_actor(Some("Dave")))));
        assert_eq!(val["actor"]["display_name"], "Dave");
    }

    #[test]
    fn attachment_omits_actor_when_absent() {
        let val = project_attachment(make_attachment(None));
        assert!(val.get("actor").is_none());
    }

    #[test]
    fn attachment_drops_document_id_and_sha256() {
        let val = project_attachment(make_attachment(None));
        assert!(val.get("document_id").is_none());
        assert!(val.get("sha256").is_none());
    }

    // -----------------------------------------------------------------------
    // require_confirm
    // -----------------------------------------------------------------------

    #[test]
    fn require_confirm_true_passes() {
        assert!(require_confirm(true, "task", "ATL-42").is_ok());
    }

    #[test]
    fn require_confirm_false_returns_actionable_error() {
        let err = require_confirm(false, "task", "ATL-42").unwrap_err();
        assert!(err.contains("ATL-42"), "error must name the resource id");
        assert!(
            err.contains("confirm: true"),
            "error must instruct re-call with confirm: true"
        );
        assert!(err.contains("task"), "error must name the resource type");
    }

    // -----------------------------------------------------------------------
    // resolve_column_id_on_board
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_column_exact_match_returns_id() {
        let cols = vec![make_column("To Do"), make_column("Done")];
        let id = resolve_column_id_on_board("To Do", &cols).unwrap();
        assert_eq!(id, cols[0].id);
    }

    #[test]
    fn resolve_column_partial_match_returns_id() {
        let cols = vec![make_column("In Progress"), make_column("Done")];
        let id = resolve_column_id_on_board("progress", &cols).unwrap();
        assert_eq!(id, cols[0].id);
    }

    #[test]
    fn resolve_column_case_insensitive() {
        let cols = vec![make_column("In Progress")];
        let id = resolve_column_id_on_board("IN PROGRESS", &cols).unwrap();
        assert_eq!(id, cols[0].id);
    }

    #[test]
    fn resolve_column_no_match_lists_available_columns() {
        let cols = vec![make_column("To Do"), make_column("Done")];
        let err = resolve_column_id_on_board("nonexistent", &cols).unwrap_err();
        assert!(
            err.contains("nonexistent"),
            "error must echo the name searched"
        );
        assert!(err.contains("To Do"), "error must list available columns");
        assert!(err.contains("Done"), "error must list available columns");
    }

    #[test]
    fn resolve_column_ambiguous_match_errors_with_matched_names() {
        let cols = vec![
            make_column("Todo"),
            make_column("Todo Later"),
            make_column("Done"),
        ];
        let err = resolve_column_id_on_board("todo", &cols).unwrap_err();
        assert!(err.contains("ambiguous"), "error must say ambiguous");
        assert!(err.contains("Todo"), "error must list matched names");
        assert!(err.contains("Todo Later"), "error must list matched names");
    }

    // -----------------------------------------------------------------------
    // validate_priority
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
    // validate_assignee_type
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
    // map_present_value
    // -----------------------------------------------------------------------

    #[test]
    fn map_present_value_absent_yields_none() {
        let result = map_present_value(None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn map_present_value_explicit_null_yields_some_null() {
        let result = map_present_value(Some(&Value::Null), None).unwrap();
        assert_eq!(result, Some(Value::Null));
    }

    #[test]
    fn map_present_value_string_value_yields_some_value() {
        let v = json!("high");
        let result = map_present_value(Some(&v), None).unwrap();
        assert_eq!(result, Some(json!("high")));
    }

    #[test]
    fn map_present_value_with_valid_priority_passes() {
        let v = json!("high");
        let result = map_present_value(Some(&v), Some(validate_priority)).unwrap();
        assert_eq!(result, Some(json!("high")));
    }

    #[test]
    fn map_present_value_with_invalid_priority_errors() {
        let v = json!("bogus");
        let err = map_present_value(Some(&v), Some(validate_priority)).unwrap_err();
        assert!(err.contains("bogus"));
        assert!(err.contains("low"));
    }

    #[test]
    fn map_present_value_null_with_validator_bypasses_validation() {
        // Clearing a field never validates — the validator only fires for non-null.
        let result = map_present_value(Some(&Value::Null), Some(validate_priority)).unwrap();
        assert_eq!(result, Some(Value::Null));
    }

    // -----------------------------------------------------------------------
    // validate_estimate / validate_estimate_value
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
        let v = json!(-3);
        let err = validate_estimate_value(&v).unwrap_err();
        assert!(
            err.contains("non-negative"),
            "error must state the constraint"
        );
    }

    #[test]
    fn validate_estimate_value_accepts_zero_and_positive() {
        assert!(validate_estimate_value(&json!(0)).is_ok());
        assert!(validate_estimate_value(&json!(8)).is_ok());
    }

    #[test]
    fn validate_estimate_value_passes_null() {
        assert!(validate_estimate_value(&Value::Null).is_ok());
    }

    #[test]
    fn validate_estimate_value_passes_absent_represented_as_non_number() {
        assert!(validate_estimate_value(&json!("five")).is_ok());
    }

    // -----------------------------------------------------------------------
    // project_assignee
    // -----------------------------------------------------------------------

    use atlas_api::dtos::boards_tasks::AssigneeDto;

    fn make_assignee() -> AssigneeDto {
        AssigneeDto {
            assignee: ActorDto {
                r#type: "user".into(),
                id: fixed_uuid(),
                display_name: Some("Bob".into()),
                key_type: None,
                account_status: None,
            },
            assigned_by: ActorDto {
                r#type: "api_key".into(),
                id: fixed_uuid(),
                display_name: None,
                key_type: None,
                account_status: None,
            },
            assigned_at: now(),
        }
    }

    #[test]
    fn assignee_projection_includes_type_display_name_assigned_at() {
        let val = project_assignee(make_assignee());
        assert_eq!(val["type"], "user");
        assert_eq!(val["display_name"], "Bob");
        assert!(val.get("assigned_at").is_some());
    }

    #[test]
    fn assignee_projection_drops_assigned_by() {
        let val = project_assignee(make_assignee());
        assert!(val.get("assigned_by").is_none());
    }

    // -----------------------------------------------------------------------
    // validate_reference_kind
    // -----------------------------------------------------------------------

    #[test]
    fn validate_reference_kind_valid_values_pass() {
        for v in &["relates", "blocks", "parent", "spec"] {
            assert!(validate_reference_kind(v).is_ok(), "'{v}' should be valid");
        }
    }

    #[test]
    fn validate_reference_kind_invalid_lists_options() {
        let err = validate_reference_kind("linked").unwrap_err();
        assert!(err.contains("linked"), "error must echo the bad value");
        assert!(err.contains("relates"), "error must list valid values");
        assert!(err.contains("blocks"), "error must list valid values");
        assert!(err.contains("parent"), "error must list valid values");
        assert!(err.contains("spec"), "error must list valid values");
    }

    // -----------------------------------------------------------------------
    // validate_single_target
    // -----------------------------------------------------------------------

    #[test]
    fn validate_single_target_task_only_passes() {
        let result = validate_single_target(Some("ATL-1"), None);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_single_target_doc_only_passes() {
        let id = fixed_uuid();
        let result = validate_single_target(None, Some(&id));
        assert!(result.is_ok());
    }

    #[test]
    fn validate_single_target_both_errors() {
        let id = fixed_uuid();
        let err = validate_single_target(Some("ATL-1"), Some(&id)).unwrap_err();
        assert!(
            err.contains("exactly one"),
            "error must mention exactly one target"
        );
        assert!(err.contains("not both"), "error must say not both");
    }

    #[test]
    fn validate_single_target_neither_errors() {
        let err = validate_single_target(None, None).unwrap_err();
        assert!(
            err.contains("exactly one"),
            "error must mention exactly one target"
        );
        assert!(
            err.contains("neither"),
            "error must say neither was provided"
        );
    }

    // -----------------------------------------------------------------------
    // project_promotion
    // -----------------------------------------------------------------------

    use atlas_api::dtos::boards_tasks::PromotionDto;

    fn make_promotion(with_reference: bool) -> PromotionDto {
        let parent_reference = if with_reference {
            Some(ReferenceDto {
                id: fixed_uuid(),
                kind: "parent".into(),
                target_task_id: Some(fixed_uuid()),
                target_readable_id: Some("ATL-10".into()),
                target_document_id: None,
                target_title: None,
                target_resolved: true,
                created_by: actor(),
                created_at: now(),
            })
        } else {
            None
        };

        PromotionDto {
            task: make_task_dto(),
            parent_reference,
            checklist_item: make_checklist_item(false, None),
        }
    }

    #[test]
    fn promotion_includes_task_and_checklist_item() {
        let val = project_promotion(make_promotion(false));
        assert!(val.get("task").is_some(), "promotion must include task");
        assert_eq!(val["task"]["readable_id"], "ATL-1");
        assert!(
            val.get("checklist_item").is_some(),
            "promotion must include checklist_item"
        );
    }

    #[test]
    fn promotion_includes_parent_reference_when_present() {
        let val = project_promotion(make_promotion(true));
        assert!(val.get("parent_reference").is_some());
        assert_eq!(val["parent_reference"]["kind"], "parent");
    }

    #[test]
    fn promotion_omits_parent_reference_when_absent() {
        let val = project_promotion(make_promotion(false));
        assert!(val.get("parent_reference").is_none());
    }

    // -----------------------------------------------------------------------
    // project_status_template
    // -----------------------------------------------------------------------

    use atlas_api::dtos::status_templates::StatusTemplateDto;

    fn make_status_template(color: Option<&str>) -> StatusTemplateDto {
        StatusTemplateDto {
            id: fixed_uuid(),
            workspace_id: fixed_uuid(),
            name: "In Progress".into(),
            color: color.map(String::from),
            position_key: "a0".into(),
            created_at: now(),
            updated_at: now(),
        }
    }

    #[test]
    fn status_template_projection_includes_required_fields() {
        let val = project_status_template(make_status_template(None));
        assert_eq!(val["name"], "In Progress");
        assert_eq!(val["position_key"], "a0");
        assert!(!val["id"].is_null());
        assert!(!val["updated_at"].is_null());
    }

    #[test]
    fn status_template_projection_drops_workspace_id_and_created_at() {
        let val = project_status_template(make_status_template(None));
        assert!(val.get("workspace_id").is_none());
        assert!(val.get("created_at").is_none());
    }

    #[test]
    fn status_template_projection_includes_color_when_present() {
        let val = project_status_template(make_status_template(Some("blue")));
        assert_eq!(val["color"], "blue");
    }

    #[test]
    fn status_template_projection_omits_color_when_absent() {
        let val = project_status_template(make_status_template(None));
        assert!(val.get("color").is_none());
    }

    // -----------------------------------------------------------------------
    // project_audit_entry
    // -----------------------------------------------------------------------

    use atlas_api::dtos::audit::AuditEntryDto;

    fn make_audit_entry_user(
        action: &str,
        target_type: &str,
        target_id: Option<Uuid>,
        target_label: Option<&str>,
        metadata: serde_json::Value,
    ) -> AuditEntryDto {
        AuditEntryDto {
            id: fixed_uuid(),
            workspace_id: Some(fixed_uuid()),
            actor: ActorDto {
                r#type: "user".into(),
                id: fixed_uuid(),
                display_name: Some("Alice".into()),
                key_type: None,
                account_status: Some("active".into()),
            },
            action: action.into(),
            target_type: target_type.into(),
            target_id,
            target_label: target_label.map(String::from),
            metadata,
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    fn make_audit_entry_api_key(action: &str) -> AuditEntryDto {
        AuditEntryDto {
            id: fixed_uuid(),
            workspace_id: None,
            actor: ActorDto {
                r#type: "api_key".into(),
                id: fixed_uuid(),
                display_name: Some("ci-bot".into()),
                key_type: Some("bot".into()),
                account_status: None,
            },
            action: action.into(),
            target_type: "api_key".into(),
            target_id: Some(fixed_uuid()),
            target_label: None,
            metadata: serde_json::json!({}),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn audit_entry_drops_internal_id() {
        let entry = make_audit_entry_user("membership.role_changed", "user", None, None, json!({}));
        let val = project_audit_entry(entry);
        assert!(val.get("id").is_none(), "internal id must be dropped");
    }

    #[test]
    fn audit_entry_drops_workspace_id() {
        let entry = make_audit_entry_user("membership.role_changed", "user", None, None, json!({}));
        let val = project_audit_entry(entry);
        assert!(
            val.get("workspace_id").is_none(),
            "workspace_id must be dropped"
        );
    }

    #[test]
    fn audit_entry_user_actor_surfaces_display_name_and_account_status() {
        let entry = make_audit_entry_user(
            "membership.role_changed",
            "user",
            Some(fixed_uuid()),
            Some("Bob"),
            json!({"old_role": "member", "new_role": "admin"}),
        );
        let val = project_audit_entry(entry);
        assert_eq!(val["actor"]["type"], "user");
        assert_eq!(val["actor"]["display_name"], "Alice");
        assert_eq!(val["actor"]["account_status"], "active");
        assert!(val["actor"].get("key_type").is_none());
    }

    #[test]
    fn audit_entry_api_key_actor_surfaces_key_type_no_account_status() {
        let entry = make_audit_entry_api_key("api_key.revoked");
        let val = project_audit_entry(entry);
        assert_eq!(val["actor"]["type"], "api_key");
        assert_eq!(val["actor"]["display_name"], "ci-bot");
        assert_eq!(val["actor"]["key_type"], "bot");
        assert!(val["actor"].get("account_status").is_none());
    }

    #[test]
    fn audit_entry_includes_action_target_type_created_at() {
        let entry = make_audit_entry_user("membership.role_changed", "user", None, None, json!({}));
        let val = project_audit_entry(entry);
        assert_eq!(val["action"], "membership.role_changed");
        assert_eq!(val["target_type"], "user");
        assert!(val.get("created_at").is_some());
    }

    #[test]
    fn audit_entry_includes_target_id_when_present() {
        let tid = fixed_uuid();
        let entry = make_audit_entry_user("user.disabled", "user", Some(tid), None, json!({}));
        let val = project_audit_entry(entry);
        assert_eq!(val["target_id"].as_str().unwrap(), tid.to_string());
    }

    #[test]
    fn audit_entry_omits_target_id_when_absent() {
        let entry = make_audit_entry_user("user.disabled", "user", None, None, json!({}));
        let val = project_audit_entry(entry);
        assert!(val.get("target_id").is_none());
    }

    #[test]
    fn audit_entry_includes_target_label_when_present() {
        let entry = make_audit_entry_user(
            "membership.removed",
            "user",
            Some(fixed_uuid()),
            Some("bob"),
            json!({}),
        );
        let val = project_audit_entry(entry);
        assert_eq!(val["target_label"], "bob");
    }

    #[test]
    fn audit_entry_omits_target_label_when_absent() {
        let entry = make_audit_entry_api_key("api_key.created");
        let val = project_audit_entry(entry);
        assert!(val.get("target_label").is_none());
    }

    #[test]
    fn audit_entry_metadata_passthrough() {
        let meta = json!({"old_role": "member", "new_role": "admin"});
        let entry =
            make_audit_entry_user("membership.role_changed", "user", None, None, meta.clone());
        let val = project_audit_entry(entry);
        assert_eq!(val["metadata"], meta);
    }

    #[test]
    fn audit_entry_empty_metadata_passthrough() {
        let entry = make_audit_entry_user("user.disabled", "user", None, None, json!({}));
        let val = project_audit_entry(entry);
        assert_eq!(val["metadata"], json!({}));
    }
}
