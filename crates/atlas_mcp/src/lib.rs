#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod response;

use atlas_api::dtos::boards_tasks::{
    AddAssigneeRequest, CreateBoardRequest, CreateChecklistItemRequest, CreateColumnRequest,
    CreateReferenceRequest, CreateSubtaskRequest, CreateTaskRequest, MoveTaskRequest,
    PromoteChecklistItemRequest, TaskPropertiesDto, UpdateBoardRequest, UpdateChecklistItemRequest,
    UpdateColumnRequest, UpdateTaskRequest, WorkspaceTaskQueryParams,
};
use atlas_api::dtos::documents::{
    CreateDocumentRequest, MoveDocumentRequest, UpdateContentRequest, UpdateDocumentRequest,
};
use atlas_api::dtos::folders::{CreateFolderRequest, MoveFolderRequest, RenameFolderRequest};
use atlas_api::dtos::saved_searches::{CreateSavedSearchRequest, RenameSavedSearchRequest};
use atlas_api::dtos::status_templates::{CreateStatusTemplateRequest, UpdateStatusTemplateRequest};
use atlas_api::dtos::tags::{CreateTagRequest, UpdateTagRequest};
use atlas_api::dtos::task_views::{
    CreateTaskViewRequest, TaskViewFiltersDto, UpdateTaskViewRequest,
};
use atlas_api::dtos::{CreateProjectRequest, UpdateProjectRequest};
use atlas_client::AtlasClient;
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerInfo},
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

/// Preserves the distinction between an absent field and an explicit JSON `null`.
///
/// Standard serde maps both to `None` for `Option<T>`. This deserializer instead
/// maps an explicit `null` to `Some(Value::Null)` so callers can express "clear
/// this field" vs "leave this field unchanged" in PATCH requests.
fn present_value<'de, D>(de: D) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(de).map(Some)
}

use response::{
    Detail, enrich_client_error, envelope_page, map_present_value, match_columns_by_name,
    parse_csv, parse_detail, project_activity_entry, project_assignee, project_attachment,
    project_backlink, project_board_summary, project_checklist_item, project_column,
    project_document_compact, project_document_full, project_document_summary, project_folder,
    project_principal, project_project, project_promotion, project_reference,
    project_revision_content, project_revision_meta, project_saved_search, project_search_hit,
    project_status_template, project_tag, project_task_backlink, project_task_compact,
    project_task_full, project_task_row, project_task_view, project_workspace, require_confirm,
    resolve_column_id_on_board, validate_assignee_type, validate_priority, validate_reference_kind,
    validate_single_target, wrap_vec,
};

const ATLAS_INSTRUCTIONS: &str = "\
Atlas is a personal knowledge base of notes (markdown documents) and tasks (kanban \
boards). Work as an agent: discover with the read tools, then mutate with the write \
tools. Each tool's own description is authoritative; this preamble covers the \
conventions shared across all of them.\n\
\n\
Conventions:\n\
- Discover before acting: use `search` (keyword plus filters like status:open, tag:rust) \
and the list tools to find resources first. Identify tasks by readable_id (e.g. ATL-42) \
and documents by slug.\n\
- Pass names, not UUIDs, for boards / columns / assignees; on a miss the error lists the \
valid options.\n\
- List responses are paginated as {items, next_cursor, has_more}; reads are compact by \
default — pass detail=full for heavy fields (document content, task description).\n\
- PATCH updates are partial: omit a field to leave it unchanged, pass null to clear it.\n\
- Destructive deletes (task, document, folder, board, column, project) require confirm: true.\n\
- Editing document content is compare-and-swap: read with `get_document detail=full` to get \
head_revision_id and content, edit locally, then call `update_document_content` with \
base_revision_id = head_revision_id; on a revision_conflict, apply the returned \
base_to_current_patch and retry with current_revision_id.\n\
\n\
Tools by area (see each tool's own description for parameters):\n\
- Read: `search`, `get_document`, `list_tasks`, `get_task`.\n\
- Structure: `list_documents`, `list_folders`, `list_boards`, `list_columns`.\n\
- Workspace context: `list_workspaces`, `list_projects`, `list_members`, `list_tags`, \
`list_used_labels`, `list_saved_searches`, `list_task_views`.\n\
- Links and depth: `get_task_references`, `get_task_backlinks`, `get_document_backlinks`, \
`list_checklist`, `list_activity`, `list_document_history`, `get_document_revision`, \
`list_attachments`.\n\
- Task writes: `create_task`, `update_task`, `move_task`, `delete_task`, \
`add_task_assignee`, `remove_task_assignee`.\n\
- Document and folder writes: `create_document`, `update_document_metadata`, \
`update_document_content`, `delete_document`, `move_document`, `copy_document`, \
`create_folder`, `rename_folder`, `move_folder`, `copy_folder`, `delete_folder`.\n\
- Board, column and tag writes: `create_board`, `update_board`, `delete_board`, \
`create_column`, `update_column`, `delete_column`, `create_tag`, `update_tag`, `delete_tag`.\n\
- Graph writes: `add_task_reference`, `remove_task_reference`, `add_checklist_item`, \
`update_checklist_item`, `delete_checklist_item`, `promote_checklist_item`, \
`create_subtask`, `promote_subtask`.\n\
- Workspace-settings writes: `create_project`, `update_project`, `delete_project`, \
`create_status_template`, `update_status_template`, `delete_status_template`, \
`create_saved_search`, `rename_saved_search`, `delete_saved_search`, `create_task_view`, \
`update_task_view`, `delete_task_view`.";

/// MCP server backed by an Atlas HTTP API endpoint.
///
/// In stdio mode, holds the single startup token for all tool calls.
/// In HTTP mode, `stdio_token` is `None` and each tool call resolves its
/// client from the per-request Bearer header (4b seam — see `resolve_client`).
/// Cloning shares the same `Arc<reqwest::Client>` pool.
#[derive(Clone)]
pub struct AtlasMcp {
    base_url: String,
    shared_http: std::sync::Arc<reqwest::Client>,
    /// Startup token from `ATLAS_TOKEN`; present in stdio mode, absent in HTTP mode.
    stdio_token: Option<String>,
}

impl AtlasMcp {
    /// Returns an `AtlasClient` built from the startup token, for pre-serve diagnostics.
    ///
    /// Returns `Err` when called in HTTP mode (no startup token).
    pub fn client(&self) -> Result<AtlasClient, &'static str> {
        let token = self
            .stdio_token
            .as_deref()
            .ok_or("client() called in HTTP mode where no startup token exists")?;
        Ok(AtlasClient::with_shared_pool(
            (*self.shared_http).clone(),
            &self.base_url,
            token,
        ))
    }

    /// Constructs an `AtlasMcp` for stdio mode with the given base URL and required API token.
    ///
    /// Returns an error if either argument is empty.
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> anyhow::Result<Self> {
        let base_url = base_url.into();
        let token = token.into();

        if base_url.is_empty() {
            anyhow::bail!("base_url must not be empty");
        }
        if token.is_empty() {
            anyhow::bail!("ATLAS_TOKEN must not be empty");
        }

        Ok(Self {
            base_url,
            shared_http: std::sync::Arc::new(reqwest::Client::new()),
            stdio_token: Some(token),
        })
    }

    /// Constructs an `AtlasMcp` for HTTP mode.
    ///
    /// No startup token is stored; each request supplies its own Bearer token,
    /// resolved per call in `resolve_client`.
    pub fn new_http(base_url: impl Into<String>) -> anyhow::Result<Self> {
        let base_url = base_url.into();

        if base_url.is_empty() {
            anyhow::bail!("base_url must not be empty");
        }

        Ok(Self {
            base_url,
            shared_http: std::sync::Arc::new(reqwest::Client::new()),
            stdio_token: None,
        })
    }

    /// Resolves the `AtlasClient` to use for this tool call.
    ///
    /// In stdio mode the stored startup token is used. In HTTP mode the token is
    /// extracted per-request from the `Authorization: Bearer <token>` header,
    /// accessible via `ctx.extensions.get::<http::request::Parts>()`. Returns
    /// `Err` when the header is absent, malformed, or carries a non-`atlas_`-prefixed token.
    fn resolve_client(&self, ctx: &RequestContext<RoleServer>) -> Result<AtlasClient, String> {
        if let Some(token) = &self.stdio_token {
            return Ok(AtlasClient::with_shared_pool(
                (*self.shared_http).clone(),
                &self.base_url,
                token,
            ));
        }

        let parts = ctx
            .extensions
            .get::<http::request::Parts>()
            .ok_or_else(|| {
                "missing HTTP request context: Bearer token cannot be read in this transport mode"
                    .to_string()
            })?;

        let header_value = parts
            .headers
            .get(http::header::AUTHORIZATION)
            .ok_or_else(|| {
                "missing Authorization header: provide 'Authorization: Bearer atlas_<token>'"
                    .to_string()
            })?
            .to_str()
            .map_err(|_| "Authorization header contains invalid characters".to_string())?;

        let token = parse_bearer_atlas_token(header_value)?;

        Ok(AtlasClient::with_shared_pool(
            (*self.shared_http).clone(),
            &self.base_url,
            token,
        ))
    }
}

/// Parses and validates an `Authorization` header value of the form `Bearer atlas_<token>`.
///
/// Returns the validated token slice (without the `Bearer ` prefix) on success.
/// Returns a descriptive error string if the value does not start with `Bearer `, if the
/// token portion does not start with `atlas_`, or if the remaining token is empty.
pub fn parse_bearer_atlas_token(header_value: &str) -> Result<&str, String> {
    let token = header_value.strip_prefix("Bearer ").ok_or_else(|| {
        "Authorization header must use the Bearer scheme: 'Authorization: Bearer atlas_<token>'"
            .to_string()
    })?;

    if token.is_empty() {
        return Err(
            "Bearer token is empty: provide 'Authorization: Bearer atlas_<token>'".to_string(),
        );
    }

    if !token.starts_with("atlas_") {
        return Err(
            "Bearer token must start with 'atlas_': received an unrecognized token format"
                .to_string(),
        );
    }

    Ok(token)
}

// ---------------------------------------------------------------------------
// Parameter structs
// ---------------------------------------------------------------------------

/// Parameters accepted by the `search` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// Workspace slug to search in.
    pub workspace: String,
    /// Query string. Supports token filters: `status:open`, `tag:rust`, `priority:high`.
    pub query: String,
    /// Kind: `all` (default), `note`, or `task`.
    #[serde(default)]
    pub type_filter: Option<String>,
    /// Sort: `relevance` (default) or `updated`.
    #[serde(default)]
    pub sort: Option<String>,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `get_document` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetDocumentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID (both resolve).
    pub slug: String,
    /// `compact` (default) = metadata only; `full` = include markdown content + frontmatter.
    #[serde(default)]
    pub detail: Option<String>,
}

/// Parameters accepted by the `list_tasks` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTasksParams {
    /// Workspace slug.
    pub workspace: String,
    /// Column/status name, partial match (e.g. "todo" matches "Todo" or "Todo Later").
    /// If `board` is given, resolved within that board; otherwise matched across all boards.
    /// Providing `board` is strongly recommended to avoid an expensive workspace-wide walk.
    #[serde(default)]
    pub status: Option<String>,
    /// Board name (partial match) or UUID to scope to one board.
    #[serde(default)]
    pub board: Option<String>,
    /// `me` | `user:{uuid}` | `api_key:{uuid}`.
    #[serde(default)]
    pub assignee: Option<String>,
    /// Comma-separated priorities: `low`, `medium`, `high`, `urgent`.
    #[serde(default)]
    pub priority: Option<String>,
    /// Comma-separated labels; tasks must carry ALL listed labels.
    #[serde(default)]
    pub label: Option<String>,
    /// Sort: `updated_at_desc` (default) | `updated_at_asc` | `priority_desc` | `title_asc`.
    #[serde(default)]
    pub sort: Option<String>,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `get_task` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTaskParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// `compact` (default) = identifying fields; `full` = adds description, references, subtasks.
    #[serde(default)]
    pub detail: Option<String>,
}

/// Parameters accepted by the `list_documents` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDocumentsParams {
    /// Workspace slug.
    pub workspace: String,
    /// Project slug. Document listing is per-project; use `search` for cross-project discovery.
    pub project: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `list_folders` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListFoldersParams {
    /// Workspace slug.
    pub workspace: String,
    /// Project slug. Folder listing is per-project.
    pub project: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `list_boards` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListBoardsParams {
    /// Workspace slug.
    pub workspace: String,
    /// Project slug. Board listing is per-project.
    pub project: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `list_columns` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListColumnsParams {
    /// Workspace slug.
    pub workspace: String,
    /// Board name (partial match) or UUID. Returns all columns of that board.
    pub board: String,
}

/// Parameters accepted by the `list_tags` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTagsParams {
    /// Workspace slug.
    pub workspace: String,
}

/// Parameters accepted by the `list_used_labels` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListUsedLabelsParams {
    /// Workspace slug.
    pub workspace: String,
}

/// Parameters accepted by the `list_members` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMembersParams {
    /// Workspace slug.
    pub workspace: String,
}

/// Parameters accepted by the `list_workspaces` tool.
///
/// No parameters required — returns all workspaces accessible to the caller.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWorkspacesParams {}

/// Parameters accepted by the `list_projects` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListProjectsParams {
    /// Workspace slug.
    pub workspace: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `list_saved_searches` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSavedSearchesParams {
    /// Workspace slug.
    pub workspace: String,
}

/// Parameters accepted by the `list_task_views` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTaskViewsParams {
    /// Workspace slug.
    pub workspace: String,
}

/// Parameters accepted by the `get_task_references` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTaskReferencesParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`. Returns OUTBOUND references — links this task creates
    /// to other tasks or documents. Use `get_task_backlinks` to find tasks that point TO this one.
    pub readable_id: String,
}

/// Parameters accepted by the `get_task_backlinks` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTaskBacklinksParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`. Returns INBOUND backlinks — other tasks that reference
    /// this task. Use `get_task_references` to see what this task points to (outbound).
    pub readable_id: String,
}

/// Parameters accepted by the `get_document_backlinks` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetDocumentBacklinksParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID. Returns all documents and tasks that contain a link to this document.
    pub slug: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `list_checklist` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListChecklistParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
}

/// Parameters accepted by the `list_activity` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListActivityParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
}

/// Parameters accepted by the `list_document_history` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDocumentHistoryParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID.
    pub slug: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `get_document_revision` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetDocumentRevisionParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID.
    pub slug: String,
    /// Revision sequence number from `list_document_history`.
    pub seq: i64,
}

/// Parameters accepted by the `list_attachments` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListAttachmentsParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID whose attachments to list.
    pub slug: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 20, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

// ---------------------------------------------------------------------------
// Write tool parameter structs — Task writes (batch 3a)
// ---------------------------------------------------------------------------

/// Parameters accepted by the `create_task` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTaskParams {
    /// Workspace slug.
    pub workspace: String,
    /// Board name (partial match) or UUID containing the target column.
    pub board: String,
    /// Column name (exact or partial) within the board where the task is created.
    pub column: String,
    /// Task title.
    pub title: String,
    /// Optional markdown description.
    #[serde(default)]
    pub description: Option<String>,
    /// Priority: `low`, `medium`, `high`, or `urgent`.
    #[serde(default)]
    pub priority: Option<String>,
    /// Labels to attach to the task (replaces any existing labels).
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Effort estimate in story-point units (non-negative integer).
    #[serde(default)]
    pub estimate: Option<i32>,
    /// Due date in RFC 3339 format (e.g. `2024-12-31T23:59:59Z`).
    #[serde(default)]
    pub due_date: Option<String>,
}

/// Parameters accepted by the `update_task` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTaskParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// New title. Omit to leave unchanged.
    #[serde(default)]
    pub title: Option<String>,
    /// New description. Omit to leave unchanged.
    #[serde(default)]
    pub description: Option<String>,
    /// Priority. Omit to leave unchanged. Pass JSON null to clear. Value must be one of low, medium, high, urgent.
    #[serde(default, deserialize_with = "present_value")]
    pub priority: Option<serde_json::Value>,
    /// Due date (RFC 3339). Omit to leave unchanged. Pass JSON null to clear.
    #[serde(default, deserialize_with = "present_value")]
    pub due_date: Option<serde_json::Value>,
    /// Estimate (story points). Omit to leave unchanged. Pass JSON null to clear.
    #[serde(default, deserialize_with = "present_value")]
    pub estimate: Option<serde_json::Value>,
    /// Labels. When provided, replaces the entire label set. Omit to leave unchanged.
    #[serde(default)]
    pub labels: Option<Vec<String>>,
}

/// Parameters accepted by the `move_task` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveTaskParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// Target column name. Must match exactly one column on the board; errors with the column list on a miss.
    pub column: String,
    /// Board name (partial match) or UUID to scope column resolution. Required when the column name is ambiguous across boards.
    #[serde(default)]
    pub board: Option<String>,
}

/// Parameters accepted by the `delete_task` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteTaskParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// Set to `true` to confirm deletion. This is a destructive, non-auto-reversible operation.
    pub confirm: bool,
}

/// Parameters accepted by the `add_task_assignee` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTaskAssigneeParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// Assignee type: `user` or `api_key`.
    pub assignee_type: String,
    /// UUID string of the user or API key to assign (from `list_members`).
    pub assignee_id: String,
}

/// Parameters accepted by the `remove_task_assignee` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveTaskAssigneeParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// Assignee reference (UUID of the user or API key to remove).
    pub assignee_ref: String,
}

/// Parameters accepted by the `create_document` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateDocumentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Project slug. The document is created inside this project.
    pub project: String,
    /// Document title.
    pub title: String,
    /// UUID string of the folder to place the document in. Omit to place at project root.
    #[serde(default)]
    pub folder_id: Option<String>,
    /// Initial markdown content. Omit for an empty document.
    #[serde(default)]
    pub content: Option<String>,
}

/// Parameters accepted by the `update_document_metadata` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateDocumentMetadataParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID.
    pub slug: String,
    /// New title. Omit to leave unchanged.
    #[serde(default)]
    pub title: Option<String>,
    /// UUID string of the parent folder. Omit to leave unchanged.
    #[serde(default)]
    pub folder_id: Option<String>,
}

/// Parameters accepted by the `update_document_content` tool.
///
/// Uses compare-and-swap (CAS) semantics. Before calling this tool:
/// 1. Call `get_document` with `detail=full` to obtain `head_revision_id` and the current content.
/// 2. Edit the content locally.
/// 3. Call this tool with `base_revision_id = head_revision_id`.
///
/// On a `revision_conflict` error, the response includes `current_revision_id`,
/// `current_seq`, and `base_to_current_patch`. Apply the patch to your edited content
/// and retry with `base_revision_id = current_revision_id`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateDocumentContentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID.
    pub slug: String,
    /// New full markdown content for the document.
    pub content: String,
    /// The `head_revision_id` UUID string from a previous `get_document` call. Must match
    /// the current server-side head or the request returns a revision_conflict error.
    pub base_revision_id: String,
}

/// Parameters accepted by the `delete_document` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteDocumentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID.
    pub slug: String,
    /// Set to `true` to confirm deletion. This is a destructive, non-auto-reversible operation.
    pub confirm: bool,
}

/// Parameters accepted by the `move_document` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveDocumentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID.
    pub slug: String,
    /// UUID string of the destination folder. Omit to move to the project root.
    #[serde(default)]
    pub folder_id: Option<String>,
}

/// Parameters accepted by the `copy_document` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CopyDocumentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug or UUID of the source document.
    pub slug: String,
    /// UUID string of the destination folder for the copy. Omit to copy into the same folder as the source.
    #[serde(default)]
    pub folder_id: Option<String>,
}

/// Parameters accepted by the `create_folder` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateFolderParams {
    /// Workspace slug.
    pub workspace: String,
    /// Project slug. The folder is created inside this project.
    pub project: String,
    /// Folder name.
    pub name: String,
    /// UUID string of the parent folder. Omit to create at the project root.
    #[serde(default)]
    pub parent_folder_id: Option<String>,
}

/// Parameters accepted by the `rename_folder` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameFolderParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID string of the folder to rename.
    pub folder_id: String,
    /// New name for the folder.
    pub name: String,
}

/// Parameters accepted by the `move_folder` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MoveFolderParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID string of the folder to move.
    pub folder_id: String,
    /// UUID string of the new parent folder. Omit to move to the project root.
    /// Note: folder ordering is not supported — the moved folder's position within its
    /// new parent is determined by the server.
    #[serde(default)]
    pub parent_folder_id: Option<String>,
}

/// Parameters accepted by the `copy_folder` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CopyFolderParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID string of the folder to copy (recursively copies sub-folders and documents).
    pub folder_id: String,
    /// UUID string of the parent folder for the copy. Omit to copy under the same parent as the source.
    #[serde(default)]
    pub parent_folder_id: Option<String>,
}

/// Parameters accepted by the `delete_folder` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteFolderParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID string of the folder to delete.
    pub folder_id: String,
    /// Set to `true` to confirm deletion. The folder row is soft-deleted; documents inside
    /// keep their folder_id and may become orphaned from navigation.
    pub confirm: bool,
}

// ---------------------------------------------------------------------------
// Batch 3c param structs — Board / Column / Tag writes
// ---------------------------------------------------------------------------

/// Parameters accepted by the `create_board` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateBoardParams {
    /// Workspace slug.
    pub workspace: String,
    /// Project slug that will own the new board.
    pub project: String,
    /// Name of the new board.
    pub name: String,
}

/// Parameters accepted by the `update_board` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateBoardParams {
    /// Workspace slug.
    pub workspace: String,
    /// Board name (partial match) or UUID string to identify the board to update.
    pub board: String,
    /// New name for the board.
    #[serde(default)]
    pub name: Option<String>,
}

/// Parameters accepted by the `delete_board` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteBoardParams {
    /// Workspace slug.
    pub workspace: String,
    /// Board name (partial match) or UUID string identifying the board to delete.
    pub board: String,
    /// Set to `true` to confirm deletion. Soft-deletes only the board row; columns
    /// and tasks become unreachable from listings but their rows persist.
    pub confirm: bool,
}

/// Parameters accepted by the `create_column` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateColumnParams {
    /// Workspace slug.
    pub workspace: String,
    /// Board name (partial match) or UUID string identifying the target board.
    pub board: String,
    /// Name of the new column.
    pub name: String,
    /// Optional color swatch ID for the column.
    #[serde(default)]
    pub color: Option<String>,
    /// Optional position anchor: UUID/key of the column this new column should appear before.
    #[serde(default)]
    pub before: Option<String>,
    /// Optional position anchor: UUID/key of the column this new column should appear after.
    #[serde(default)]
    pub after: Option<String>,
}

/// Parameters accepted by the `update_column` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateColumnParams {
    /// Workspace slug.
    pub workspace: String,
    /// Board name (partial match) or UUID string identifying the board that owns the column.
    pub board: String,
    /// Column name (partial match, resolved on the board) identifying the column to update.
    pub column: String,
    /// New name for the column.
    #[serde(default)]
    pub name: Option<String>,
    /// Color swatch ID. Omit to leave unchanged. Pass JSON null to clear. Pass a string to set.
    #[serde(default, deserialize_with = "present_value")]
    pub color: Option<serde_json::Value>,
    /// Optional position anchor: UUID/key of the column this column should move before.
    #[serde(default)]
    pub before: Option<String>,
    /// Optional position anchor: UUID/key of the column this column should move after.
    #[serde(default)]
    pub after: Option<String>,
}

/// Parameters accepted by the `delete_column` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteColumnParams {
    /// Workspace slug.
    pub workspace: String,
    /// Board name (partial match) or UUID string identifying the board that owns the column.
    pub board: String,
    /// Column name (partial match, resolved on the board) identifying the column to delete.
    pub column: String,
    /// Set to `true` to confirm deletion. The server refuses deletion when the column still
    /// has tasks — move or delete the column's tasks first.
    pub confirm: bool,
}

/// Parameters accepted by the `create_tag` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTagParams {
    /// Workspace slug.
    pub workspace: String,
    /// Name of the tag to create. Idempotent by case-insensitive name: returns the existing
    /// tag when one already exists with the same name.
    pub name: String,
}

/// Parameters accepted by the `update_tag` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTagParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID string of the tag to update.
    pub tag_id: String,
    /// New name for the tag.
    #[serde(default)]
    pub name: Option<String>,
    /// New color for the tag. Omit to leave unchanged. Note: the tag color cannot be cleared
    /// once set (API limitation D-WRITE-UPDATETAG-COLOR-NOCLEAR) — supply a new color to
    /// change it, or omit to leave it as-is.
    #[serde(default)]
    pub color: Option<String>,
}

/// Parameters accepted by the `delete_tag` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteTagParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID string of the tag to delete. Soft-deletes the tag; existing task label strings
    /// are preserved even after the tag is deleted.
    pub tag_id: String,
}

// ---------------------------------------------------------------------------
// Batch 3d param structs — Graph writes: references, checklist, subtasks
// ---------------------------------------------------------------------------

/// Parameters accepted by the `add_task_reference` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTaskReferenceParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task that owns the reference (e.g. `ATL-42`).
    pub readable_id: String,
    /// Reference kind. Must be one of: `relates`, `blocks`, `parent`, `spec`.
    pub kind: String,
    /// Readable ID of the target task (e.g. `ATL-10`). Supply exactly one of
    /// `target_task_readable_id` or `target_document_id`.
    #[serde(default)]
    pub target_task_readable_id: Option<String>,
    /// UUID string of the target document. Supply exactly one of
    /// `target_task_readable_id` or `target_document_id`.
    #[serde(default)]
    pub target_document_id: Option<String>,
}

/// Parameters accepted by the `remove_task_reference` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RemoveTaskReferenceParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task that owns the reference.
    pub readable_id: String,
    /// UUID string of the reference to remove (from `get_task_references`).
    pub reference_id: String,
}

/// Parameters accepted by the `add_checklist_item` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddChecklistItemParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task to add the checklist item to.
    pub readable_id: String,
    /// Title of the new checklist item.
    pub title: String,
    /// Optional position anchor: position key of the item this new item should appear before.
    #[serde(default)]
    pub before: Option<String>,
    /// Optional position anchor: position key of the item this new item should appear after.
    #[serde(default)]
    pub after: Option<String>,
}

/// Parameters accepted by the `update_checklist_item` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateChecklistItemParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task that owns the checklist item.
    pub readable_id: String,
    /// UUID string of the checklist item to update.
    pub item_id: String,
    /// New title for the item. Omit to leave unchanged.
    #[serde(default)]
    pub title: Option<String>,
    /// New checked state. Omit to leave unchanged.
    #[serde(default)]
    pub checked: Option<bool>,
    /// Optional position anchor: position key of the item this item should move before.
    #[serde(default)]
    pub before: Option<String>,
    /// Optional position anchor: position key of the item this item should move after.
    #[serde(default)]
    pub after: Option<String>,
}

/// Parameters accepted by the `delete_checklist_item` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteChecklistItemParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task that owns the checklist item.
    pub readable_id: String,
    /// UUID string of the checklist item to delete.
    pub item_id: String,
}

/// Parameters accepted by the `promote_checklist_item` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromoteChecklistItemParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task that owns the checklist item to promote.
    pub readable_id: String,
    /// UUID string of the checklist item to promote to a task.
    pub item_id: String,
    /// Board name (partial match) or UUID string for the new task's board.
    pub board: String,
    /// Column name (resolved on the board) for the new task.
    pub column: String,
}

/// Parameters accepted by the `create_subtask` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateSubtaskParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the parent task.
    pub readable_id: String,
    /// Title of the new subtask.
    pub title: String,
}

/// Parameters accepted by the `promote_subtask` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromoteSubtaskParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the subtask to promote to a top-level task.
    pub readable_id: String,
}

// ---------------------------------------------------------------------------
// Batch 3e — Workspace-settings write param structs
// ---------------------------------------------------------------------------

/// Parameters accepted by the `create_project` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateProjectParams {
    /// Workspace slug.
    pub workspace: String,
    /// Human-readable project name.
    pub name: String,
    /// URL-safe project slug (unique in the workspace).
    pub slug: String,
    /// Short prefix used for task IDs in this project (e.g. `ATL`).
    pub task_prefix: String,
    /// `private` | `workspace` (default) | `public`.
    #[serde(default)]
    pub visibility: Option<String>,
    /// `viewer` | `editor` (default). Only meaningful when visibility is not `public`.
    #[serde(default)]
    pub visibility_role: Option<String>,
}

/// Parameters accepted by the `update_project` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateProjectParams {
    /// Workspace slug.
    pub workspace: String,
    /// Project slug to update.
    pub slug: String,
    /// New project name. Omit to leave unchanged.
    #[serde(default)]
    pub name: Option<String>,
    /// New visibility. Omit to leave unchanged.
    #[serde(default)]
    pub visibility: Option<String>,
    /// New visibility_role. Omit to leave unchanged.
    #[serde(default)]
    pub visibility_role: Option<String>,
    /// New task prefix. Omit to leave unchanged.
    #[serde(default)]
    pub task_prefix: Option<String>,
}

/// Parameters accepted by the `delete_project` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteProjectParams {
    /// Workspace slug.
    pub workspace: String,
    /// Project slug to delete.
    pub slug: String,
    /// Must be `true` to proceed. Soft-deletes the project row; boards, tasks,
    /// and documents inside are not cascaded but become unreachable from listings.
    pub confirm: bool,
}

/// Parameters accepted by the `create_status_template` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateStatusTemplateParams {
    /// Workspace slug.
    pub workspace: String,
    /// Status template name.
    pub name: String,
    /// Optional color swatch identifier.
    #[serde(default)]
    pub color: Option<String>,
    /// Optional ID of the existing template to insert before.
    #[serde(default)]
    pub before: Option<String>,
    /// Optional ID of the existing template to insert after.
    #[serde(default)]
    pub after: Option<String>,
}

/// Parameters accepted by the `update_status_template` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateStatusTemplateParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID of the status template to update.
    pub id: String,
    /// New name. Omit to leave unchanged.
    #[serde(default)]
    pub name: Option<String>,
    /// Color swatch. Omit to leave unchanged. Pass JSON null to clear.
    #[serde(default, deserialize_with = "present_value")]
    pub color: Option<serde_json::Value>,
    /// Reorder: insert before this template ID.
    #[serde(default)]
    pub before: Option<String>,
    /// Reorder: insert after this template ID.
    #[serde(default)]
    pub after: Option<String>,
}

/// Parameters accepted by the `delete_status_template` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteStatusTemplateParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID of the status template to delete.
    pub id: String,
}

/// Parameters accepted by the `create_saved_search` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateSavedSearchParams {
    /// Workspace slug.
    pub workspace: String,
    /// Display name for the saved search.
    pub name: String,
    /// Query string (supports token filters such as `status:open tag:bug`).
    pub query: String,
}

/// Parameters accepted by the `rename_saved_search` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameSavedSearchParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID of the saved search to rename.
    pub id: String,
    /// New display name.
    pub name: String,
}

/// Parameters accepted by the `delete_saved_search` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteSavedSearchParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID of the saved search to delete.
    pub id: String,
}

/// Parameters accepted by the `create_task_view` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTaskViewParams {
    /// Workspace slug.
    pub workspace: String,
    /// Display name for the task view.
    pub name: String,
    /// Filter set as a JSON object. Pass `{}` for an all-workspace view.
    /// Supported keys: `sort`, `priorities` (array), `labels` (array),
    /// `column_ids` (array of UUIDs), `board_id` (UUID), `assignee` (string),
    /// `actor_type` (\"user\" | \"api_key\").
    pub filters: serde_json::Value,
}

/// Parameters accepted by the `update_task_view` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTaskViewParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID of the task view to update.
    pub id: String,
    /// New display name.
    pub name: String,
    /// New filter set (full replacement — all previous filters are replaced).
    pub filters: serde_json::Value,
}

/// Parameters accepted by the `delete_task_view` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteTaskViewParams {
    /// Workspace slug.
    pub workspace: String,
    /// UUID of the task view to delete.
    pub id: String,
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[tool_router]
impl AtlasMcp {
    #[tool(description = "Ping the Atlas MCP server")]
    fn ping(&self) -> String {
        "pong".to_string()
    }

    #[tool(description = "Search documents and tasks across an Atlas workspace")]
    async fn search(
        &self,
        Parameters(params): Parameters<SearchParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .search(
                &params.workspace,
                &params.query,
                params.type_filter.as_deref(),
                params.sort.as_deref(),
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| format!("search failed: {e}"))?;

        let result = envelope_page(page, project_search_hit);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Retrieve an Atlas document by slug or UUID")]
    async fn get_document(
        &self,
        Parameters(params): Parameters<GetDocumentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let doc = client
            .get_document(&params.workspace, &params.slug)
            .await
            .map_err(|e| format!("get_document '{}' failed: {e}", params.slug))?;

        let result = match parse_detail(params.detail.as_deref()) {
            Detail::Compact => project_document_compact(doc),
            Detail::Full => project_document_full(doc),
        };

        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List tasks across an Atlas workspace with optional filters")]
    async fn list_tasks(
        &self,
        Parameters(params): Parameters<ListTasksParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let priorities = params
            .priority
            .as_deref()
            .map(parse_csv)
            .unwrap_or_default();

        let labels = params.label.as_deref().map(parse_csv).unwrap_or_default();

        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let column_ids = if let Some(status_name) = &params.status {
            self.resolve_column_ids(
                &client,
                &params.workspace,
                params.board.as_deref(),
                status_name,
            )
            .await?
        } else {
            Vec::new()
        };

        let board_id = if let Some(board) = &params.board {
            Some(
                self.resolve_board_id(&client, &params.workspace, board)
                    .await?,
            )
        } else {
            None
        };

        let query = WorkspaceTaskQueryParams {
            assignee: params.assignee,
            actor: None,
            column_ids,
            priorities,
            labels,
            board_id,
            sort: params.sort,
            cursor: params.cursor,
            limit: Some(limit),
        };

        let page = client
            .list_workspace_tasks(&params.workspace, &query)
            .await
            .map_err(|e| format!("list_tasks failed: {e}"))?;

        let result = envelope_page(page, project_task_row);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Retrieve a single Atlas task by readable ID")]
    async fn get_task(
        &self,
        Parameters(params): Parameters<GetTaskParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let task = client
            .get_task(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| format!("get_task '{}' failed: {e}", params.readable_id))?;

        let result = match parse_detail(params.detail.as_deref()) {
            Detail::Compact => project_task_compact(&task),
            Detail::Full => {
                let refs = client
                    .list_references(&params.workspace, &params.readable_id)
                    .await
                    .map(|v| v.into_iter().map(project_reference).collect::<Vec<_>>())
                    .map_err(|e| format!("list_references failed: {e}"));

                let subtasks = client
                    .list_subtasks(&params.workspace, &params.readable_id)
                    .await
                    .map(|v| v.into_iter().map(project_task_row).collect::<Vec<_>>())
                    .map_err(|e| format!("list_subtasks failed: {e}"));

                project_task_full(&task, refs, subtasks)
            }
        };

        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List documents in a project within an Atlas workspace")]
    async fn list_documents(
        &self,
        Parameters(params): Parameters<ListDocumentsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .list_documents(
                &params.workspace,
                &params.project,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| {
                format!(
                    "list_documents for project '{}' failed: {e}",
                    params.project
                )
            })?;

        let result = envelope_page(page, project_document_summary);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List folders in a project within an Atlas workspace")]
    async fn list_folders(
        &self,
        Parameters(params): Parameters<ListFoldersParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .list_folders(
                &params.workspace,
                &params.project,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| format!("list_folders for project '{}' failed: {e}", params.project))?;

        let result = envelope_page(page, project_folder);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List boards in a project within an Atlas workspace")]
    async fn list_boards(
        &self,
        Parameters(params): Parameters<ListBoardsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .list_boards(
                &params.workspace,
                &params.project,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| format!("list_boards for project '{}' failed: {e}", params.project))?;

        let result = envelope_page(page, project_board_summary);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List columns of a board; use column IDs in list_tasks status filters")]
    async fn list_columns(
        &self,
        Parameters(params): Parameters<ListColumnsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let board_id_str = self
            .resolve_board_id(&client, &params.workspace, &params.board)
            .await?;

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board_id '{board_id_str}' is not a valid UUID"))?;

        let cols = client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| format!("list_columns for board '{}' failed: {e}", params.board))?;

        let result = wrap_vec(cols, project_column);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List the registered tag registry for an Atlas workspace")]
    async fn list_tags(
        &self,
        Parameters(params): Parameters<ListTagsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let tags = client
            .list_tags(&params.workspace)
            .await
            .map_err(|e| format!("list_tags for workspace '{}' failed: {e}", params.workspace))?;

        let result = wrap_vec(tags, project_tag);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "List labels currently applied to tasks in an Atlas workspace (may include unregistered labels)"
    )]
    async fn list_used_labels(
        &self,
        Parameters(params): Parameters<ListUsedLabelsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let labels = client
            .list_used_labels(&params.workspace)
            .await
            .map_err(|e| {
                format!(
                    "list_used_labels for workspace '{}' failed: {e}",
                    params.workspace
                )
            })?;

        let result = wrap_vec(labels, |s| json!(s));
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "List workspace members and API-key principals; use IDs in assignee filters"
    )]
    async fn list_members(
        &self,
        Parameters(params): Parameters<ListMembersParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let members = client
            .list_workspace_members(&params.workspace)
            .await
            .map_err(|e| {
                format!(
                    "list_members for workspace '{}' failed: {e}",
                    params.workspace
                )
            })?;

        let result = wrap_vec(members, project_principal);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List all Atlas workspaces accessible to the caller")]
    async fn list_workspaces(
        &self,
        Parameters(_params): Parameters<ListWorkspacesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let workspaces = client
            .list_workspaces()
            .await
            .map_err(|e| format!("list_workspaces failed: {e}"))?;

        let result = wrap_vec(workspaces, project_workspace);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List projects in an Atlas workspace (cursor-paginated)")]
    async fn list_projects(
        &self,
        Parameters(params): Parameters<ListProjectsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .list_projects(&params.workspace, params.cursor.as_deref(), Some(limit))
            .await
            .map_err(|e| {
                format!(
                    "list_projects for workspace '{}' failed: {e}",
                    params.workspace
                )
            })?;

        let result = envelope_page(page, project_project);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List saved searches for an Atlas workspace")]
    async fn list_saved_searches(
        &self,
        Parameters(params): Parameters<ListSavedSearchesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let searches = client
            .list_saved_searches(&params.workspace)
            .await
            .map_err(|e| {
                format!(
                    "list_saved_searches for workspace '{}' failed: {e}",
                    params.workspace
                )
            })?;

        let result = wrap_vec(searches, project_saved_search);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List saved task views (filter presets) for an Atlas workspace")]
    async fn list_task_views(
        &self,
        Parameters(params): Parameters<ListTaskViewsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let views = client
            .list_task_views(&params.workspace)
            .await
            .map_err(|e| {
                format!(
                    "list_task_views for workspace '{}' failed: {e}",
                    params.workspace
                )
            })?;

        let result = wrap_vec(views, project_task_view);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "List OUTBOUND references from a task — tasks and documents this task links to"
    )]
    async fn get_task_references(
        &self,
        Parameters(params): Parameters<GetTaskReferencesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let refs = client
            .list_references(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| {
                format!(
                    "get_task_references for '{}' failed: {e}",
                    params.readable_id
                )
            })?;

        let result = wrap_vec(refs, project_reference);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List INBOUND backlinks to a task — other tasks that reference this task")]
    async fn get_task_backlinks(
        &self,
        Parameters(params): Parameters<GetTaskBacklinksParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let page = client
            .list_task_backlinks(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| {
                format!(
                    "get_task_backlinks for '{}' failed: {e}",
                    params.readable_id
                )
            })?;

        let result = envelope_page(page, project_task_backlink);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "List documents and tasks that link to a given document (inbound backlinks)"
    )]
    async fn get_document_backlinks(
        &self,
        Parameters(params): Parameters<GetDocumentBacklinksParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .list_backlinks(
                &params.workspace,
                &params.slug,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| format!("get_document_backlinks for '{}' failed: {e}", params.slug))?;

        let result = envelope_page(page, project_backlink);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List checklist items for a task")]
    async fn list_checklist(
        &self,
        Parameters(params): Parameters<ListChecklistParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let items = client
            .list_checklist(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| format!("list_checklist for '{}' failed: {e}", params.readable_id))?;

        let result = wrap_vec(items, project_checklist_item);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List the activity log for a task (moves, assignments, field changes)")]
    async fn list_activity(
        &self,
        Parameters(params): Parameters<ListActivityParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let page = client
            .list_activity(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| format!("list_activity for '{}' failed: {e}", params.readable_id))?;

        let result = envelope_page(page, project_activity_entry);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List revision metadata for a document (history of edits)")]
    async fn list_document_history(
        &self,
        Parameters(params): Parameters<ListDocumentHistoryParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .list_document_history(
                &params.workspace,
                &params.slug,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| format!("list_document_history for '{}' failed: {e}", params.slug))?;

        let result = envelope_page(page, project_revision_meta);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Fetch the full markdown content of a specific document revision by seq number"
    )]
    async fn get_document_revision(
        &self,
        Parameters(params): Parameters<GetDocumentRevisionParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let rev = client
            .get_revision_content(&params.workspace, &params.slug, params.seq)
            .await
            .map_err(|e| {
                format!(
                    "get_document_revision '{}' seq={} failed: {e}",
                    params.slug, params.seq
                )
            })?;

        let result = project_revision_content(rev);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List attachment metadata for a document (file name, type, size)")]
    async fn list_attachments(
        &self,
        Parameters(params): Parameters<ListAttachmentsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .list_attachments(
                &params.workspace,
                &params.slug,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| format!("list_attachments for '{}' failed: {e}", params.slug))?;

        let result = envelope_page(page, project_attachment);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Create a task on a board. Board and column are resolved by name.")]
    async fn create_task(
        &self,
        Parameters(params): Parameters<CreateTaskParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let board_id_str = self
            .resolve_board_id(&client, &params.workspace, &params.board)
            .await?;
        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let cols = client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

        let column_id = resolve_column_id_on_board(&params.column, &cols)?;

        if let Some(ref p) = params.priority {
            validate_priority(p)?;
        }

        let due_date = params
            .due_date
            .as_deref()
            .map(|s| {
                s.parse::<chrono::DateTime<chrono::Utc>>()
                    .map_err(|_| format!("due_date '{s}' is not a valid RFC 3339 timestamp"))
            })
            .transpose()?;

        let properties = if params.priority.is_some()
            || params.labels.is_some()
            || params.estimate.is_some()
            || due_date.is_some()
        {
            Some(TaskPropertiesDto {
                priority: params.priority,
                due_date,
                estimate: params.estimate,
                labels: params.labels.unwrap_or_default(),
                custom: None,
            })
        } else {
            None
        };

        let body = CreateTaskRequest {
            column_id,
            title: params.title,
            description: params.description,
            properties,
            before: None,
            after: None,
        };

        let task = client
            .create_task(&params.workspace, board_uuid, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_task"))?;

        let result = project_task_compact(&task);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Update a task. PATCH semantics: omit a field to leave it unchanged; \
                       pass JSON null to clear a clearable field (priority, due_date, estimate)."
    )]
    async fn update_task(
        &self,
        Parameters(params): Parameters<UpdateTaskParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let priority = map_present_value(params.priority.as_ref(), Some(validate_priority))?;
        let due_date = map_present_value(params.due_date.as_ref(), None)?;
        let estimate = map_present_value(params.estimate.as_ref(), None)?;

        let body = UpdateTaskRequest {
            title: params.title,
            description: params.description,
            priority,
            due_date,
            estimate,
            labels: params.labels,
            properties: None,
        };

        let task = client
            .update_task(&params.workspace, &params.readable_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_task"))?;

        let result = project_task_compact(&task);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Move a task to a different column (resolved by name). \
                       Errors with the board's column list when the column is not found.")]
    async fn move_task(
        &self,
        Parameters(params): Parameters<MoveTaskParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let board_ref = params.board.as_deref().unwrap_or(&params.readable_id);

        let board_id_str = if params.board.is_some() {
            self.resolve_board_id(&client, &params.workspace, board_ref)
                .await?
        } else {
            // No board supplied: fetch the task first to get its board_id.
            let task = client
                .get_task(&params.workspace, &params.readable_id)
                .await
                .map_err(|e| enrich_client_error(e, "get_task"))?;
            task.board_id.to_string()
        };

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let cols = client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

        let column_id = resolve_column_id_on_board(&params.column, &cols)?;

        let body = MoveTaskRequest {
            column_id,
            before: None,
            after: None,
        };

        let task = client
            .move_task(&params.workspace, &params.readable_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "move_task"))?;

        let result = project_task_compact(&task);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Delete a task permanently. Requires confirm: true. \
                       This operation is not auto-reversible.")]
    async fn delete_task(
        &self,
        Parameters(params): Parameters<DeleteTaskParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        require_confirm(params.confirm, "task", &params.readable_id)?;

        client
            .delete_task(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_task"))?;

        let result = json!({
            "deleted": true,
            "readable_id": params.readable_id,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Add an assignee (user or API key) to a task.")]
    async fn add_task_assignee(
        &self,
        Parameters(params): Parameters<AddTaskAssigneeParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        validate_assignee_type(&params.assignee_type)?;

        let assignee_id: uuid::Uuid = params
            .assignee_id
            .parse()
            .map_err(|_| format!("assignee_id '{}' is not a valid UUID", params.assignee_id))?;

        let body = AddAssigneeRequest {
            assignee_type: params.assignee_type,
            assignee_id,
        };

        let assignee = client
            .add_assignee(&params.workspace, &params.readable_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "add_task_assignee"))?;

        let result = project_assignee(assignee);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Remove an assignee from a task by their UUID reference.")]
    async fn remove_task_assignee(
        &self,
        Parameters(params): Parameters<RemoveTaskAssigneeParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        client
            .remove_assignee(&params.workspace, &params.readable_id, &params.assignee_ref)
            .await
            .map_err(|e| enrich_client_error(e, "remove_task_assignee"))?;

        let result = json!({
            "removed": true,
            "assignee_ref": params.assignee_ref,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Create a document in a project. Returns compact projection with head_revision_id."
    )]
    async fn create_document(
        &self,
        Parameters(params): Parameters<CreateDocumentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let folder_id = params
            .folder_id
            .as_deref()
            .map(|s| {
                s.parse::<uuid::Uuid>()
                    .map_err(|_| format!("folder_id '{s}' is not a valid UUID"))
            })
            .transpose()?;

        let body = CreateDocumentRequest {
            title: params.title,
            folder_id,
            content: params.content,
        };

        let doc = client
            .create_document(&params.workspace, &params.project, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_document"))?;

        let result = project_document_compact(doc);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Update document title or folder (metadata only). PATCH: omit fields to leave unchanged. Use update_document_content to change content."
    )]
    async fn update_document_metadata(
        &self,
        Parameters(params): Parameters<UpdateDocumentMetadataParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let folder_id = params
            .folder_id
            .as_deref()
            .map(|s| {
                s.parse::<uuid::Uuid>()
                    .map_err(|_| format!("folder_id '{s}' is not a valid UUID"))
            })
            .transpose()?;

        let body = UpdateDocumentRequest {
            title: params.title,
            folder_id,
        };

        let doc = client
            .update_document(&params.workspace, &params.slug, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_document_metadata"))?;

        let result = project_document_compact(doc);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Write new content to a document using compare-and-swap. \
                       Read with get_document detail=full to get head_revision_id + content, \
                       edit locally, then call with base_revision_id = head_revision_id. \
                       On revision_conflict: apply base_to_current_patch to your edit and \
                       retry with base_revision_id = current_revision_id."
    )]
    async fn update_document_content(
        &self,
        Parameters(params): Parameters<UpdateDocumentContentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let base_revision_id = params.base_revision_id.parse::<uuid::Uuid>().map_err(|_| {
            format!(
                "base_revision_id '{}' is not a valid UUID",
                params.base_revision_id
            )
        })?;

        let body = UpdateContentRequest {
            content: params.content,
            base_revision_id,
        };

        let doc = client
            .update_content(&params.workspace, &params.slug, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_document_content"))?;

        let result = project_document_compact(doc);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a document permanently. Requires confirm: true. \
                       This operation is not auto-reversible."
    )]
    async fn delete_document(
        &self,
        Parameters(params): Parameters<DeleteDocumentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        require_confirm(params.confirm, "document", &params.slug)?;

        client
            .delete_document(&params.workspace, &params.slug)
            .await
            .map_err(|e| enrich_client_error(e, "delete_document"))?;

        let result = json!({
            "deleted": true,
            "slug": params.slug,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Move a document to a different folder. Omit folder_id to move to the project root."
    )]
    async fn move_document(
        &self,
        Parameters(params): Parameters<MoveDocumentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let folder_id = params
            .folder_id
            .as_deref()
            .map(|s| {
                s.parse::<uuid::Uuid>()
                    .map_err(|_| format!("folder_id '{s}' is not a valid UUID"))
            })
            .transpose()?;

        let body = MoveDocumentRequest { folder_id };

        let doc = client
            .move_document(&params.workspace, &params.slug, body)
            .await
            .map_err(|e| enrich_client_error(e, "move_document"))?;

        let result = project_document_compact(doc);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Copy a document. Optional folder_id sets the destination; omit to copy into the same folder."
    )]
    async fn copy_document(
        &self,
        Parameters(params): Parameters<CopyDocumentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let folder_id = params
            .folder_id
            .as_deref()
            .map(|s| {
                s.parse::<uuid::Uuid>()
                    .map_err(|_| format!("folder_id '{s}' is not a valid UUID"))
            })
            .transpose()?;

        let doc = client
            .copy_document(&params.workspace, &params.slug, folder_id)
            .await
            .map_err(|e| enrich_client_error(e, "copy_document"))?;

        let result = project_document_compact(doc);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Create a folder inside a project. Optional parent_folder_id nests it; omit for project root."
    )]
    async fn create_folder(
        &self,
        Parameters(params): Parameters<CreateFolderParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let parent_folder_id = params
            .parent_folder_id
            .as_deref()
            .map(|s| {
                s.parse::<uuid::Uuid>()
                    .map_err(|_| format!("parent_folder_id '{s}' is not a valid UUID"))
            })
            .transpose()?;

        let body = CreateFolderRequest {
            name: params.name,
            parent_folder_id,
        };

        let folder = client
            .create_folder(&params.workspace, &params.project, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_folder"))?;

        let result = project_folder(folder);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Rename a folder.")]
    async fn rename_folder(
        &self,
        Parameters(params): Parameters<RenameFolderParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let folder_id = params
            .folder_id
            .parse::<uuid::Uuid>()
            .map_err(|_| format!("folder_id '{}' is not a valid UUID", params.folder_id))?;

        let body = RenameFolderRequest { name: params.name };

        let folder = client
            .rename_folder(&params.workspace, folder_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "rename_folder"))?;

        let result = project_folder(folder);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Move a folder to a new parent. Omit parent_folder_id to move to the project root. \
                       Note: ordering within the parent is not supported."
    )]
    async fn move_folder(
        &self,
        Parameters(params): Parameters<MoveFolderParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let folder_id = params
            .folder_id
            .parse::<uuid::Uuid>()
            .map_err(|_| format!("folder_id '{}' is not a valid UUID", params.folder_id))?;

        let parent_folder_id = params
            .parent_folder_id
            .as_deref()
            .map(|s| {
                s.parse::<uuid::Uuid>()
                    .map_err(|_| format!("parent_folder_id '{s}' is not a valid UUID"))
            })
            .transpose()?;

        let body = MoveFolderRequest { parent_folder_id };

        let folder = client
            .move_folder(&params.workspace, folder_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "move_folder"))?;

        let result = project_folder(folder);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Copy a folder (recursively copies sub-folders and documents). \
                       Optional parent_folder_id sets destination; omit to copy under the same parent."
    )]
    async fn copy_folder(
        &self,
        Parameters(params): Parameters<CopyFolderParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let folder_id = params
            .folder_id
            .parse::<uuid::Uuid>()
            .map_err(|_| format!("folder_id '{}' is not a valid UUID", params.folder_id))?;

        let parent_folder_id = params
            .parent_folder_id
            .as_deref()
            .map(|s| {
                s.parse::<uuid::Uuid>()
                    .map_err(|_| format!("parent_folder_id '{s}' is not a valid UUID"))
            })
            .transpose()?;

        let folder = client
            .copy_folder(&params.workspace, folder_id, parent_folder_id)
            .await
            .map_err(|e| enrich_client_error(e, "copy_folder"))?;

        let result = project_folder(folder);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a folder. Requires confirm: true. Documents inside keep \
                       their folder_id and may be orphaned from navigation after deletion."
    )]
    async fn delete_folder(
        &self,
        Parameters(params): Parameters<DeleteFolderParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        require_confirm(params.confirm, "folder", &params.folder_id)?;

        let folder_id = params
            .folder_id
            .parse::<uuid::Uuid>()
            .map_err(|_| format!("folder_id '{}' is not a valid UUID", params.folder_id))?;

        client
            .delete_folder(&params.workspace, folder_id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_folder"))?;

        let result = json!({
            "deleted": true,
            "folder_id": folder_id,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Create a new board in a project.")]
    async fn create_board(
        &self,
        Parameters(params): Parameters<CreateBoardParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateBoardRequest { name: params.name };

        let board = client
            .create_board(&params.workspace, &params.project, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_board"))?;

        let result = json!({
            "id": board.id,
            "name": board.name,
            "project_id": board.project_id,
            "updated_at": board.updated_at,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Rename a board. Board resolved by name (partial match) or UUID.")]
    async fn update_board(
        &self,
        Parameters(params): Parameters<UpdateBoardParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let board_id_str = self
            .resolve_board_id(&client, &params.workspace, &params.board)
            .await?;

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let body = UpdateBoardRequest { name: params.name };

        let board = client
            .update_board(&params.workspace, board_uuid, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_board"))?;

        let result = json!({
            "id": board.id,
            "name": board.name,
            "project_id": board.project_id,
            "updated_at": board.updated_at,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a board. Requires confirm: true. Soft-deletes only the board row; \
                       columns and tasks become unreachable from listings but their rows persist."
    )]
    async fn delete_board(
        &self,
        Parameters(params): Parameters<DeleteBoardParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        require_confirm(params.confirm, "board", &params.board)?;

        let board_id_str = self
            .resolve_board_id(&client, &params.workspace, &params.board)
            .await?;

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        client
            .delete_board(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "delete_board"))?;

        let result = json!({
            "deleted": true,
            "board_id": board_uuid,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Create a new column on a board. Optional color and ordering anchors (before/after)."
    )]
    async fn create_column(
        &self,
        Parameters(params): Parameters<CreateColumnParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let board_id_str = self
            .resolve_board_id(&client, &params.workspace, &params.board)
            .await?;

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let body = CreateColumnRequest {
            name: params.name,
            color: params.color,
            before: params.before,
            after: params.after,
        };

        let col = client
            .create_column(&params.workspace, board_uuid, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_column"))?;

        let result = project_column(col);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Update a column. Column resolved by name on the board. \
                       Color: omit to leave unchanged, pass null to clear, pass a string to set."
    )]
    async fn update_column(
        &self,
        Parameters(params): Parameters<UpdateColumnParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let board_id_str = self
            .resolve_board_id(&client, &params.workspace, &params.board)
            .await?;

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let cols = client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

        let column_id = resolve_column_id_on_board(&params.column, &cols)?;

        let color = map_present_value(params.color.as_ref(), None)?;

        let body = UpdateColumnRequest {
            name: params.name,
            color,
            before: params.before,
            after: params.after,
        };

        let col = client
            .update_column(&params.workspace, board_uuid, column_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_column"))?;

        let result = project_column(col);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a column. Requires confirm: true. The server refuses deletion \
                       when the column still has tasks — move or delete the tasks first."
    )]
    async fn delete_column(
        &self,
        Parameters(params): Parameters<DeleteColumnParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        require_confirm(params.confirm, "column", &params.column)?;

        let board_id_str = self
            .resolve_board_id(&client, &params.workspace, &params.board)
            .await?;

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let cols = client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

        let column_id = resolve_column_id_on_board(&params.column, &cols)?;

        client
            .delete_column(&params.workspace, board_uuid, column_id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_column"))?;

        let result = json!({
            "deleted": true,
            "column_id": column_id,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Create a workspace tag. Idempotent by case-insensitive name; \
                       returns the existing tag when one already exists."
    )]
    async fn create_tag(
        &self,
        Parameters(params): Parameters<CreateTagParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateTagRequest { name: params.name };

        let tag = client
            .create_tag(&params.workspace, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_tag"))?;

        let result = project_tag(tag);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Update a tag's name and/or color. Omit color to leave it unchanged. \
                       Note: a tag color cannot be cleared once set (set a new color to change it)."
    )]
    async fn update_tag(
        &self,
        Parameters(params): Parameters<UpdateTagParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let tag_id: uuid::Uuid = params
            .tag_id
            .parse()
            .map_err(|_| format!("tag_id '{}' is not a valid UUID", params.tag_id))?;

        let body = UpdateTagRequest {
            name: params.name,
            color: params.color,
        };

        let tag = client
            .update_tag(&params.workspace, tag_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_tag"))?;

        let result = project_tag(tag);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Soft-delete a workspace tag. Task label strings are preserved after deletion."
    )]
    async fn delete_tag(
        &self,
        Parameters(params): Parameters<DeleteTagParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let tag_id: uuid::Uuid = params
            .tag_id
            .parse()
            .map_err(|_| format!("tag_id '{}' is not a valid UUID", params.tag_id))?;

        client
            .delete_tag(&params.workspace, tag_id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_tag"))?;

        let result = json!({
            "deleted": true,
            "tag_id": tag_id,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // Batch 3d — Graph writes: references, checklist, subtasks
    // -----------------------------------------------------------------------

    #[tool(
        description = "Add a typed reference from a task to another task or document. \
                       kind must be one of: relates, blocks, parent, spec. \
                       Supply exactly one of target_task_readable_id or target_document_id."
    )]
    async fn add_task_reference(
        &self,
        Parameters(params): Parameters<AddTaskReferenceParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        validate_reference_kind(&params.kind)?;

        let target_doc_uuid: Option<uuid::Uuid> = params
            .target_document_id
            .as_deref()
            .map(|s| {
                s.parse()
                    .map_err(|_| format!("target_document_id '{s}' is not a valid UUID"))
            })
            .transpose()?;

        validate_single_target(
            params.target_task_readable_id.as_deref(),
            target_doc_uuid.as_ref(),
        )?;

        let body = CreateReferenceRequest {
            kind: params.kind,
            target_task_readable_id: params.target_task_readable_id,
            target_document_id: target_doc_uuid,
        };

        let reference = client
            .create_reference(&params.workspace, &params.readable_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "add_task_reference"))?;

        let result = project_reference(reference);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Remove an outbound reference from a task. \
                       reference_id is the UUID from get_task_references.")]
    async fn remove_task_reference(
        &self,
        Parameters(params): Parameters<RemoveTaskReferenceParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let reference_id: uuid::Uuid = params
            .reference_id
            .parse()
            .map_err(|_| format!("reference_id '{}' is not a valid UUID", params.reference_id))?;

        client
            .delete_reference(&params.workspace, &params.readable_id, reference_id)
            .await
            .map_err(|e| enrich_client_error(e, "remove_task_reference"))?;

        let result = json!({
            "removed": true,
            "reference_id": reference_id,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Add a checklist item to a task. Optional before/after ordering anchors.")]
    async fn add_checklist_item(
        &self,
        Parameters(params): Parameters<AddChecklistItemParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateChecklistItemRequest {
            title: params.title,
            before: params.before,
            after: params.after,
        };

        let item = client
            .create_checklist_item(&params.workspace, &params.readable_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "add_checklist_item"))?;

        let result = project_checklist_item(item);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Update a checklist item (PATCH). Omit title or checked to leave unchanged. \
                       Optional before/after ordering anchors."
    )]
    async fn update_checklist_item(
        &self,
        Parameters(params): Parameters<UpdateChecklistItemParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let item_id: uuid::Uuid = params
            .item_id
            .parse()
            .map_err(|_| format!("item_id '{}' is not a valid UUID", params.item_id))?;

        let body = UpdateChecklistItemRequest {
            title: params.title,
            checked: params.checked,
            before: params.before,
            after: params.after,
        };

        let item = client
            .update_checklist_item(&params.workspace, &params.readable_id, item_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_checklist_item"))?;

        let result = project_checklist_item(item);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Delete a checklist item from a task.")]
    async fn delete_checklist_item(
        &self,
        Parameters(params): Parameters<DeleteChecklistItemParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let item_id: uuid::Uuid = params
            .item_id
            .parse()
            .map_err(|_| format!("item_id '{}' is not a valid UUID", params.item_id))?;

        client
            .delete_checklist_item(&params.workspace, &params.readable_id, item_id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_checklist_item"))?;

        let result = json!({
            "deleted": true,
            "item_id": item_id,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Promote a checklist item to a full task on the specified board and column. \
                       Returns the new task and the updated checklist item."
    )]
    async fn promote_checklist_item(
        &self,
        Parameters(params): Parameters<PromoteChecklistItemParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let item_id: uuid::Uuid = params
            .item_id
            .parse()
            .map_err(|_| format!("item_id '{}' is not a valid UUID", params.item_id))?;

        let board_id_str = self
            .resolve_board_id(&client, &params.workspace, &params.board)
            .await?;

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let cols = client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

        let column_id = resolve_column_id_on_board(&params.column, &cols)?;

        let body = PromoteChecklistItemRequest {
            board_id: board_uuid,
            column_id,
        };

        let promotion = client
            .promote_checklist_item(&params.workspace, &params.readable_id, item_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "promote_checklist_item"))?;

        let result = project_promotion(promotion);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Create a subtask under a parent task.")]
    async fn create_subtask(
        &self,
        Parameters(params): Parameters<CreateSubtaskParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateSubtaskRequest {
            title: params.title,
        };

        let task = client
            .create_subtask(&params.workspace, &params.readable_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_subtask"))?;

        let result = project_task_compact(&task);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Promote a subtask to a top-level task, detaching it from its parent.")]
    async fn promote_subtask(
        &self,
        Parameters(params): Parameters<PromoteSubtaskParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let task = client
            .promote_subtask(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| enrich_client_error(e, "promote_subtask"))?;

        let result = project_task_compact(&task);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // Batch 3e — Project CRUD
    // -----------------------------------------------------------------------

    #[tool(description = "Create a new project in the workspace. \
        Returns the created project. \
        Slug must be URL-safe and unique within the workspace.")]
    async fn create_project(
        &self,
        Parameters(params): Parameters<CreateProjectParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateProjectRequest {
            name: params.name,
            slug: params.slug,
            task_prefix: params.task_prefix,
            visibility: params.visibility,
            visibility_role: params.visibility_role,
        };

        let project = client
            .create_project(&params.workspace, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_project"))?;

        let result = project_project(project);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Update a project's metadata (name, visibility, task_prefix). \
        PATCH semantics: omit a field to leave it unchanged. \
        Returns the updated project."
    )]
    async fn update_project(
        &self,
        Parameters(params): Parameters<UpdateProjectParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = UpdateProjectRequest {
            name: params.name,
            visibility: params.visibility,
            visibility_role: params.visibility_role,
            task_prefix: params.task_prefix,
        };

        let project = client
            .update_project(&params.workspace, &params.slug, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_project"))?;

        let result = project_project(project);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Delete a project. Requires confirm: true. \
        Soft-deletes only the project row; boards, tasks, and documents inside \
        are not cascaded but become unreachable from project listings.")]
    async fn delete_project(
        &self,
        Parameters(params): Parameters<DeleteProjectParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        require_confirm(params.confirm, "project", &params.slug)?;

        client
            .delete_project(&params.workspace, &params.slug)
            .await
            .map_err(|e| enrich_client_error(e, "delete_project"))?;

        let result = serde_json::json!({ "deleted": true, "slug": params.slug });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // Batch 3e — Status template CRUD
    // -----------------------------------------------------------------------

    #[tool(description = "Create a workspace status template. \
        Optional color swatch and ordering anchors (before/after). \
        Returns the created template.")]
    async fn create_status_template(
        &self,
        Parameters(params): Parameters<CreateStatusTemplateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateStatusTemplateRequest {
            name: params.name,
            color: params.color,
            before: params.before,
            after: params.after,
        };

        let template = client
            .create_status_template(&params.workspace, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_status_template"))?;

        let result = project_status_template(template);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Update a workspace status template. \
        name and color are optional PATCH fields; color accepts null to clear. \
        Returns the updated template.")]
    async fn update_status_template(
        &self,
        Parameters(params): Parameters<UpdateStatusTemplateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let id: uuid::Uuid = params
            .id
            .parse()
            .map_err(|_| format!("invalid UUID for id: '{}'", params.id))?;

        let color = map_present_value(params.color.as_ref(), None)?;

        let body = UpdateStatusTemplateRequest {
            name: params.name,
            color,
            before: params.before,
            after: params.after,
        };

        let template = client
            .update_status_template(&params.workspace, id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_status_template"))?;

        let result = project_status_template(template);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a workspace status template. Plain delete, no confirm required. \
        Returns {deleted: true, id}."
    )]
    async fn delete_status_template(
        &self,
        Parameters(params): Parameters<DeleteStatusTemplateParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let id: uuid::Uuid = params
            .id
            .parse()
            .map_err(|_| format!("invalid UUID for id: '{}'", params.id))?;

        client
            .delete_status_template(&params.workspace, id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_status_template"))?;

        let result = serde_json::json!({ "deleted": true, "id": id });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // Batch 3e — Saved search CRUD
    // -----------------------------------------------------------------------

    #[tool(description = "Create a saved search in the workspace. \
        Returns the created saved search with its id for future rename or delete.")]
    async fn create_saved_search(
        &self,
        Parameters(params): Parameters<CreateSavedSearchParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateSavedSearchRequest {
            name: params.name,
            query: params.query,
        };

        let search = client
            .create_saved_search(&params.workspace, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_saved_search"))?;

        let result = project_saved_search(search);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Rename a saved search. \
        To change the query, delete and recreate. \
        Returns the updated saved search.")]
    async fn rename_saved_search(
        &self,
        Parameters(params): Parameters<RenameSavedSearchParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let id: uuid::Uuid = params
            .id
            .parse()
            .map_err(|_| format!("invalid UUID for id: '{}'", params.id))?;

        let body = RenameSavedSearchRequest { name: params.name };

        let search = client
            .rename_saved_search(&params.workspace, id, body)
            .await
            .map_err(|e| enrich_client_error(e, "rename_saved_search"))?;

        let result = project_saved_search(search);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a saved search. Plain delete, no confirm required. \
        Returns {deleted: true, id}."
    )]
    async fn delete_saved_search(
        &self,
        Parameters(params): Parameters<DeleteSavedSearchParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let id: uuid::Uuid = params
            .id
            .parse()
            .map_err(|_| format!("invalid UUID for id: '{}'", params.id))?;

        client
            .delete_saved_search(&params.workspace, id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_saved_search"))?;

        let result = serde_json::json!({ "deleted": true, "id": id });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // Batch 3e — Task view CRUD
    // -----------------------------------------------------------------------

    #[tool(description = "Create a task view (filter preset) in the workspace. \
        Pass an empty filters object {} for an all-workspace view. \
        Returns the created task view.")]
    async fn create_task_view(
        &self,
        Parameters(params): Parameters<CreateTaskViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let filters: TaskViewFiltersDto =
            serde_json::from_value(params.filters).map_err(|e| format!("invalid filters: {e}"))?;

        let body = CreateTaskViewRequest {
            name: params.name,
            filters,
        };

        let view = client
            .create_task_view(&params.workspace, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_task_view"))?;

        let result = project_task_view(view);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Update a task view. Both name and filters are required — \
        this is a full replacement, not a PATCH. \
        Returns the updated task view."
    )]
    async fn update_task_view(
        &self,
        Parameters(params): Parameters<UpdateTaskViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let id: uuid::Uuid = params
            .id
            .parse()
            .map_err(|_| format!("invalid UUID for id: '{}'", params.id))?;

        let filters: TaskViewFiltersDto =
            serde_json::from_value(params.filters).map_err(|e| format!("invalid filters: {e}"))?;

        let body = UpdateTaskViewRequest {
            name: params.name,
            filters,
        };

        let view = client
            .update_task_view(&params.workspace, id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_task_view"))?;

        let result = project_task_view(view);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a task view. Plain delete, no confirm required. \
        Returns {deleted: true, id}."
    )]
    async fn delete_task_view(
        &self,
        Parameters(params): Parameters<DeleteTaskViewParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let id: uuid::Uuid = params
            .id
            .parse()
            .map_err(|_| format!("invalid UUID for id: '{}'", params.id))?;

        client
            .delete_task_view(&params.workspace, id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_task_view"))?;

        let result = serde_json::json!({ "deleted": true, "id": id });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }
}

impl AtlasMcp {
    /// Resolves a status/column name to matching column UUIDs.
    ///
    /// When `board` is provided (name or UUID), resolves within that one board using a
    /// single `list_columns` call. Otherwise walks all projects and all their boards to
    /// collect all matching columns — an O(projects × boards) GET sequence that is
    /// mitigated by encouraging callers to supply `board`.
    async fn resolve_column_ids(
        &self,
        client: &AtlasClient,
        ws: &str,
        board: Option<&str>,
        status_name: &str,
    ) -> Result<Vec<String>, String> {
        if let Some(board_ref) = board {
            let board_id = self.resolve_board_id(client, ws, board_ref).await?;
            let board_uuid: uuid::Uuid = board_id
                .parse()
                .map_err(|_| format!("resolved board_id '{board_id}' is not a valid UUID"))?;
            let cols = client
                .list_columns(ws, board_uuid)
                .await
                .map_err(|e| format!("list_columns failed: {e}"))?;
            return Ok(match_columns_by_name(status_name, &cols));
        }

        // No board given: workspace-wide walk (D-WSCOL — O(projects + boards) GETs).
        let mut all_cols = Vec::new();
        let mut project_cursor: Option<String> = None;

        loop {
            let projects = client
                .list_projects(ws, project_cursor.as_deref(), Some(200))
                .await
                .map_err(|e| format!("list_projects failed: {e}"))?;

            for project in &projects.items {
                let mut board_cursor: Option<String> = None;
                loop {
                    let boards = client
                        .list_boards(ws, &project.slug, board_cursor.as_deref(), Some(200))
                        .await
                        .map_err(|e| {
                            format!("list_boards for project '{}' failed: {e}", project.slug)
                        })?;

                    for board in &boards.items {
                        let cols = client.list_columns(ws, board.id).await.map_err(|e| {
                            format!("list_columns for board '{}' failed: {e}", board.name)
                        })?;
                        all_cols.extend(cols);
                    }

                    if !boards.has_more {
                        break;
                    }
                    board_cursor = boards.next_cursor;
                }
            }

            if !projects.has_more {
                break;
            }
            project_cursor = projects.next_cursor;
        }

        Ok(match_columns_by_name(status_name, &all_cols))
    }

    /// Resolves a board reference (name fragment or UUID string) to a UUID string.
    ///
    /// When the input parses as a UUID it is returned directly. Otherwise walks
    /// all projects' boards to find the first partial name match.
    async fn resolve_board_id(
        &self,
        client: &AtlasClient,
        ws: &str,
        board_ref: &str,
    ) -> Result<String, String> {
        if uuid::Uuid::parse_str(board_ref).is_ok() {
            return Ok(board_ref.to_string());
        }

        let needle = board_ref.to_ascii_lowercase();
        let mut project_cursor: Option<String> = None;

        loop {
            let projects = client
                .list_projects(ws, project_cursor.as_deref(), Some(200))
                .await
                .map_err(|e| format!("list_projects failed: {e}"))?;

            for project in &projects.items {
                let mut board_cursor: Option<String> = None;
                loop {
                    let boards = client
                        .list_boards(ws, &project.slug, board_cursor.as_deref(), Some(200))
                        .await
                        .map_err(|e| format!("list_boards failed: {e}"))?;

                    for board in &boards.items {
                        if board.name.to_ascii_lowercase().contains(&needle) {
                            return Ok(board.id.to_string());
                        }
                    }

                    if !boards.has_more {
                        break;
                    }
                    board_cursor = boards.next_cursor;
                }
            }

            if !projects.has_more {
                break;
            }
            project_cursor = projects.next_cursor;
        }

        Err(format!(
            "no board matching '{board_ref}' found in workspace '{ws}'"
        ))
    }
}

#[tool_handler]
impl ServerHandler for AtlasMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("atlas-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(ATLAS_INSTRUCTIONS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_missing_token() {
        let result = AtlasMcp::new("http://localhost:8080", "");
        assert!(result.is_err(), "empty token must be rejected");
    }

    #[test]
    fn rejects_missing_base_url() {
        let result = AtlasMcp::new("", "some-token");
        assert!(result.is_err(), "empty base_url must be rejected");
    }

    #[test]
    fn constructs_with_valid_args() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token");
        assert!(server.is_ok());
    }

    #[test]
    fn clone_shares_http_pool() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let cloned = server.clone();
        assert!(std::sync::Arc::ptr_eq(
            &server.shared_http,
            &cloned.shared_http
        ));
    }

    #[test]
    fn get_info_returns_correct_name_and_version() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        assert_eq!(info.server_info.name, "atlas-mcp");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(
            info.instructions.as_deref().is_some_and(|s| !s.is_empty()),
            "instructions must be Some and non-empty"
        );
    }

    #[test]
    fn get_info_instructions_reference_search_not_search_resources() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(
            instructions.contains("`search`"),
            "instructions must reference `search`, not `search_resources`"
        );
        assert!(
            !instructions.contains("search_resources"),
            "instructions must not mention search_resources"
        );
    }

    #[test]
    fn ping_returns_pong() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        assert_eq!(server.ping(), "pong");
    }

    #[test]
    fn search_params_deserializes_minimal() {
        let json = r#"{"workspace":"my-ws","query":"hello"}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
        assert_eq!(params.query, "hello");
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn search_params_deserializes_with_cursor() {
        let json = r#"{"workspace":"ws","query":"q","cursor":"abc123","limit":10}"#;
        let params: SearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.cursor.as_deref(), Some("abc123"));
        assert_eq!(params.limit, Some(10));
    }

    #[test]
    fn get_document_params_deserializes() {
        let json = r#"{"workspace":"ws","slug":"my-doc","detail":"full"}"#;
        let params: GetDocumentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert_eq!(params.detail.as_deref(), Some("full"));
    }

    #[test]
    fn list_tasks_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws"}"#;
        let params: ListTasksParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert!(params.status.is_none());
        assert!(params.board.is_none());
    }

    #[test]
    fn list_tasks_params_deserializes_with_filters() {
        let json = r#"{"workspace":"ws","status":"In Progress","board":"main","priority":"high,urgent","label":"backend"}"#;
        let params: ListTasksParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.status.as_deref(), Some("In Progress"));
        assert_eq!(params.board.as_deref(), Some("main"));
        assert_eq!(params.priority.as_deref(), Some("high,urgent"));
        assert_eq!(params.label.as_deref(), Some("backend"));
    }

    #[test]
    fn get_task_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-42","detail":"compact"}"#;
        let params: GetTaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.readable_id, "ATL-42");
        assert_eq!(params.detail.as_deref(), Some("compact"));
    }

    #[test]
    fn list_documents_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","project":"my-proj"}"#;
        let params: ListDocumentsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.project, "my-proj");
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn list_documents_params_deserializes_with_pagination() {
        let json = r#"{"workspace":"ws","project":"p","cursor":"tok","limit":50}"#;
        let params: ListDocumentsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.cursor.as_deref(), Some("tok"));
        assert_eq!(params.limit, Some(50));
    }

    #[test]
    fn list_folders_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","project":"my-proj"}"#;
        let params: ListFoldersParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.project, "my-proj");
        assert!(params.cursor.is_none());
    }

    #[test]
    fn list_boards_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","project":"my-proj"}"#;
        let params: ListBoardsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.project, "my-proj");
        assert!(params.limit.is_none());
    }

    #[test]
    fn list_columns_params_deserializes() {
        let json = r#"{"workspace":"ws","board":"Sprint Board"}"#;
        let params: ListColumnsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.board, "Sprint Board");
    }

    #[test]
    fn list_columns_params_accepts_uuid_board() {
        let json = r#"{"workspace":"ws","board":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: ListColumnsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.board, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn list_tags_params_deserializes() {
        let json = r#"{"workspace":"my-ws"}"#;
        let params: ListTagsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
    }

    #[test]
    fn list_used_labels_params_deserializes() {
        let json = r#"{"workspace":"my-ws"}"#;
        let params: ListUsedLabelsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
    }

    #[test]
    fn list_members_params_deserializes() {
        let json = r#"{"workspace":"my-ws"}"#;
        let params: ListMembersParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
    }

    #[test]
    fn list_workspaces_params_deserializes_empty_object() {
        let json = r#"{}"#;
        let _params: ListWorkspacesParams = serde_json::from_str(json).unwrap();
    }

    #[test]
    fn list_projects_params_deserializes_minimal() {
        let json = r#"{"workspace":"my-ws"}"#;
        let params: ListProjectsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn list_projects_params_deserializes_with_pagination() {
        let json = r#"{"workspace":"ws","cursor":"tok","limit":50}"#;
        let params: ListProjectsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.cursor.as_deref(), Some("tok"));
        assert_eq!(params.limit, Some(50));
    }

    #[test]
    fn list_saved_searches_params_deserializes() {
        let json = r#"{"workspace":"my-ws"}"#;
        let params: ListSavedSearchesParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
    }

    #[test]
    fn list_task_views_params_deserializes() {
        let json = r#"{"workspace":"my-ws"}"#;
        let params: ListTaskViewsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
    }

    #[test]
    fn get_task_references_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-42"}"#;
        let params: GetTaskReferencesParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.readable_id, "ATL-42");
    }

    #[test]
    fn get_task_backlinks_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-7"}"#;
        let params: GetTaskBacklinksParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.readable_id, "ATL-7");
    }

    #[test]
    fn get_document_backlinks_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","slug":"my-doc"}"#;
        let params: GetDocumentBacklinksParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.slug, "my-doc");
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn get_document_backlinks_params_deserializes_with_pagination() {
        let json = r#"{"workspace":"ws","slug":"doc","cursor":"tok","limit":50}"#;
        let params: GetDocumentBacklinksParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.cursor.as_deref(), Some("tok"));
        assert_eq!(params.limit, Some(50));
    }

    #[test]
    fn get_info_instructions_reference_link_tools() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(
            instructions.contains("`get_task_references`"),
            "instructions must mention get_task_references"
        );
        assert!(
            instructions.contains("`get_task_backlinks`"),
            "instructions must mention get_task_backlinks"
        );
        assert!(
            instructions.contains("`get_document_backlinks`"),
            "instructions must mention get_document_backlinks"
        );
    }

    #[test]
    fn list_checklist_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-5"}"#;
        let params: ListChecklistParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.readable_id, "ATL-5");
    }

    #[test]
    fn list_activity_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-10"}"#;
        let params: ListActivityParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.readable_id, "ATL-10");
    }

    #[test]
    fn list_document_history_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","slug":"my-doc"}"#;
        let params: ListDocumentHistoryParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn get_document_revision_params_deserializes() {
        let json = r#"{"workspace":"ws","slug":"my-doc","seq":7}"#;
        let params: GetDocumentRevisionParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert_eq!(params.seq, 7);
    }

    #[test]
    fn list_attachments_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","slug":"my-doc"}"#;
        let params: ListAttachmentsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn get_info_instructions_reference_depth_tools() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(
            instructions.contains("`list_checklist`"),
            "instructions must mention list_checklist"
        );
        assert!(
            instructions.contains("`list_activity`"),
            "instructions must mention list_activity"
        );
        assert!(
            instructions.contains("`list_document_history`"),
            "instructions must mention list_document_history"
        );
        assert!(
            instructions.contains("`get_document_revision`"),
            "instructions must mention get_document_revision"
        );
        assert!(
            instructions.contains("`list_attachments`"),
            "instructions must mention list_attachments"
        );
    }

    // -----------------------------------------------------------------------
    // Batch 3b: document + folder write params
    // -----------------------------------------------------------------------

    #[test]
    fn create_document_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","project":"my-proj","title":"New Note"}"#;
        let params: CreateDocumentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.project, "my-proj");
        assert_eq!(params.title, "New Note");
        assert!(params.folder_id.is_none());
        assert!(params.content.is_none());
    }

    #[test]
    fn create_document_params_deserializes_full() {
        let json = r##"{"workspace":"ws","project":"p","title":"T","folder_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","content":"# Hello"}"##;
        let params: CreateDocumentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.content.as_deref(), Some("# Hello"));
        assert!(params.folder_id.is_some());
    }

    #[test]
    fn update_document_metadata_params_all_optional() {
        let json = r#"{"workspace":"ws","slug":"my-doc"}"#;
        let params: UpdateDocumentMetadataParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert!(params.title.is_none());
        assert!(params.folder_id.is_none());
    }

    #[test]
    fn update_document_metadata_params_with_title() {
        let json = r#"{"workspace":"ws","slug":"my-doc","title":"Renamed"}"#;
        let params: UpdateDocumentMetadataParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.title.as_deref(), Some("Renamed"));
    }

    #[test]
    fn update_document_content_params_deserializes() {
        let json = r##"{"workspace":"ws","slug":"my-doc","content":"# Updated","base_revision_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"##;
        let params: UpdateDocumentContentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert_eq!(params.content, "# Updated");
        assert_eq!(
            params.base_revision_id,
            "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"
        );
    }

    #[test]
    fn delete_document_params_deserializes() {
        let json = r#"{"workspace":"ws","slug":"my-doc","confirm":true}"#;
        let params: DeleteDocumentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert!(params.confirm);
    }

    #[test]
    fn move_document_params_optional_folder_id() {
        let json = r#"{"workspace":"ws","slug":"my-doc"}"#;
        let params: MoveDocumentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert!(params.folder_id.is_none());
    }

    #[test]
    fn copy_document_params_optional_folder_id() {
        let json = r#"{"workspace":"ws","slug":"my-doc","folder_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: CopyDocumentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "my-doc");
        assert!(params.folder_id.is_some());
    }

    #[test]
    fn create_folder_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","project":"my-proj","name":"Designs"}"#;
        let params: CreateFolderParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "Designs");
        assert!(params.parent_folder_id.is_none());
    }

    #[test]
    fn create_folder_params_deserializes_with_parent() {
        let json = r#"{"workspace":"ws","project":"p","name":"Sub","parent_folder_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: CreateFolderParams = serde_json::from_str(json).unwrap();
        assert!(params.parent_folder_id.is_some());
    }

    #[test]
    fn rename_folder_params_deserializes() {
        let json = r#"{"workspace":"ws","folder_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","name":"Renamed"}"#;
        let params: RenameFolderParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "Renamed");
    }

    #[test]
    fn move_folder_params_optional_parent() {
        let json = r#"{"workspace":"ws","folder_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: MoveFolderParams = serde_json::from_str(json).unwrap();
        assert!(params.parent_folder_id.is_none());
    }

    #[test]
    fn copy_folder_params_optional_parent() {
        let json = r#"{"workspace":"ws","folder_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","parent_folder_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: CopyFolderParams = serde_json::from_str(json).unwrap();
        assert!(params.parent_folder_id.is_some());
    }

    #[test]
    fn delete_folder_params_deserializes() {
        let json = r#"{"workspace":"ws","folder_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","confirm":true}"#;
        let params: DeleteFolderParams = serde_json::from_str(json).unwrap();
        assert!(params.confirm);
    }

    #[test]
    fn get_info_instructions_reference_document_write_tools() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(
            instructions.contains("`update_document_content`"),
            "instructions must mention update_document_content"
        );
        assert!(
            instructions.contains("`create_document`"),
            "instructions must mention create_document"
        );
        assert!(
            instructions.contains("`delete_folder`"),
            "instructions must mention delete_folder"
        );
    }

    // -----------------------------------------------------------------------
    // Batch 3c: board / column / tag write params
    // -----------------------------------------------------------------------

    #[test]
    fn create_board_params_deserializes() {
        let json = r#"{"workspace":"ws","project":"my-proj","name":"Sprint Board"}"#;
        let params: CreateBoardParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.project, "my-proj");
        assert_eq!(params.name, "Sprint Board");
    }

    #[test]
    fn update_board_params_all_optional_name() {
        let json = r#"{"workspace":"ws","board":"Sprint Board"}"#;
        let params: UpdateBoardParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.board, "Sprint Board");
        assert!(params.name.is_none());
    }

    #[test]
    fn update_board_params_with_name() {
        let json = r#"{"workspace":"ws","board":"old-name","name":"New Name"}"#;
        let params: UpdateBoardParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name.as_deref(), Some("New Name"));
    }

    #[test]
    fn delete_board_params_requires_confirm() {
        let json = r#"{"workspace":"ws","board":"Sprint Board","confirm":true}"#;
        let params: DeleteBoardParams = serde_json::from_str(json).unwrap();
        assert!(params.confirm);
        assert_eq!(params.board, "Sprint Board");
    }

    #[test]
    fn create_column_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","board":"Sprint Board","name":"To Do"}"#;
        let params: CreateColumnParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "To Do");
        assert!(params.color.is_none());
        assert!(params.before.is_none());
        assert!(params.after.is_none());
    }

    #[test]
    fn create_column_params_deserializes_with_optional_fields() {
        let json = r##"{"workspace":"ws","board":"Sprint Board","name":"In Progress","color":"#3B82F6","before":"abc123"}"##;
        let params: CreateColumnParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.color.as_deref(), Some("#3B82F6"));
        assert_eq!(params.before.as_deref(), Some("abc123"));
    }

    #[test]
    fn update_column_params_color_absent_is_none() {
        let json = r#"{"workspace":"ws","board":"Sprint Board","column":"To Do"}"#;
        let params: UpdateColumnParams = serde_json::from_str(json).unwrap();
        assert!(params.color.is_none());
        assert!(params.name.is_none());
    }

    #[test]
    fn update_column_params_color_explicit_null_is_some_null() {
        let json = r#"{"workspace":"ws","board":"b","column":"col","color":null}"#;
        let params: UpdateColumnParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.color, Some(serde_json::Value::Null));
    }

    #[test]
    fn update_column_params_color_set() {
        let json = r##"{"workspace":"ws","board":"b","column":"col","color":"#FF5733"}"##;
        let params: UpdateColumnParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.color, Some(serde_json::json!("#FF5733")));
    }

    #[test]
    fn update_task_params_clearable_absent_is_none() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1"}"#;
        let params: UpdateTaskParams = serde_json::from_str(json).unwrap();
        assert!(params.priority.is_none());
        assert!(params.due_date.is_none());
        assert!(params.estimate.is_none());
    }

    #[test]
    fn update_task_params_clearable_explicit_null_is_some_null() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","priority":null,"due_date":null,"estimate":null}"#;
        let params: UpdateTaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.priority, Some(serde_json::Value::Null));
        assert_eq!(params.due_date, Some(serde_json::Value::Null));
        assert_eq!(params.estimate, Some(serde_json::Value::Null));
    }

    #[test]
    fn update_task_params_clearable_set() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","priority":"high","estimate":5}"#;
        let params: UpdateTaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.priority, Some(serde_json::json!("high")));
        assert_eq!(params.estimate, Some(serde_json::json!(5)));
        assert!(params.due_date.is_none());
    }

    #[test]
    fn delete_column_params_requires_confirm() {
        let json = r#"{"workspace":"ws","board":"Sprint Board","column":"Done","confirm":true}"#;
        let params: DeleteColumnParams = serde_json::from_str(json).unwrap();
        assert!(params.confirm);
        assert_eq!(params.column, "Done");
    }

    #[test]
    fn create_tag_params_deserializes() {
        let json = r#"{"workspace":"ws","name":"backend"}"#;
        let params: CreateTagParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "backend");
    }

    #[test]
    fn update_tag_params_all_optional() {
        let json = r#"{"workspace":"ws","tag_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: UpdateTagParams = serde_json::from_str(json).unwrap();
        assert!(params.name.is_none());
        assert!(params.color.is_none());
    }

    #[test]
    fn update_tag_params_with_name_and_color() {
        let json = r##"{"workspace":"ws","tag_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","name":"frontend","color":"#3B82F6"}"##;
        let params: UpdateTagParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name.as_deref(), Some("frontend"));
        assert_eq!(params.color.as_deref(), Some("#3B82F6"));
    }

    #[test]
    fn delete_tag_params_deserializes() {
        let json = r#"{"workspace":"ws","tag_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: DeleteTagParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.tag_id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn get_info_instructions_reference_board_write_tools() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(
            instructions.contains("`create_board`"),
            "instructions must mention create_board"
        );
        assert!(
            instructions.contains("`delete_column`"),
            "instructions must mention delete_column"
        );
        assert!(
            instructions.contains("`create_tag`"),
            "instructions must mention create_tag"
        );
        assert!(
            instructions.contains("`delete_tag`"),
            "instructions must mention delete_tag"
        );
    }

    // -----------------------------------------------------------------------
    // Batch 3d: graph write params
    // -----------------------------------------------------------------------

    #[test]
    fn add_task_reference_params_task_target() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","kind":"relates","target_task_readable_id":"ATL-2"}"#;
        let params: AddTaskReferenceParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.kind, "relates");
        assert_eq!(params.target_task_readable_id.as_deref(), Some("ATL-2"));
        assert!(params.target_document_id.is_none());
    }

    #[test]
    fn add_task_reference_params_document_target() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","kind":"spec","target_document_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: AddTaskReferenceParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.kind, "spec");
        assert!(params.target_task_readable_id.is_none());
        assert!(params.target_document_id.is_some());
    }

    #[test]
    fn remove_task_reference_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","reference_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: RemoveTaskReferenceParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.readable_id, "ATL-1");
        assert_eq!(params.reference_id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn add_checklist_item_params_minimal() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","title":"Write tests"}"#;
        let params: AddChecklistItemParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.title, "Write tests");
        assert!(params.before.is_none());
        assert!(params.after.is_none());
    }

    #[test]
    fn add_checklist_item_params_with_ordering() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","title":"Step 2","after":"aaa0"}"#;
        let params: AddChecklistItemParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.after.as_deref(), Some("aaa0"));
    }

    #[test]
    fn update_checklist_item_params_all_optional() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","item_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: UpdateChecklistItemParams = serde_json::from_str(json).unwrap();
        assert!(params.title.is_none());
        assert!(params.checked.is_none());
        assert!(params.before.is_none());
        assert!(params.after.is_none());
    }

    #[test]
    fn update_checklist_item_params_set_checked_and_title() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","item_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","title":"New title","checked":true}"#;
        let params: UpdateChecklistItemParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.title.as_deref(), Some("New title"));
        assert_eq!(params.checked, Some(true));
    }

    #[test]
    fn delete_checklist_item_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","item_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: DeleteChecklistItemParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.readable_id, "ATL-1");
        assert_eq!(params.item_id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn promote_checklist_item_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","item_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","board":"Sprint Board","column":"To Do"}"#;
        let params: PromoteChecklistItemParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.board, "Sprint Board");
        assert_eq!(params.column, "To Do");
    }

    #[test]
    fn create_subtask_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","title":"Implement error handling"}"#;
        let params: CreateSubtaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.title, "Implement error handling");
    }

    #[test]
    fn promote_subtask_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-5"}"#;
        let params: PromoteSubtaskParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.readable_id, "ATL-5");
    }

    #[test]
    fn get_info_instructions_reference_graph_write_tools() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(
            instructions.contains("`add_task_reference`"),
            "instructions must mention add_task_reference"
        );
        assert!(
            instructions.contains("`remove_task_reference`"),
            "instructions must mention remove_task_reference"
        );
        assert!(
            instructions.contains("`add_checklist_item`"),
            "instructions must mention add_checklist_item"
        );
        assert!(
            instructions.contains("`promote_checklist_item`"),
            "instructions must mention promote_checklist_item"
        );
        assert!(
            instructions.contains("`create_subtask`"),
            "instructions must mention create_subtask"
        );
        assert!(
            instructions.contains("`promote_subtask`"),
            "instructions must mention promote_subtask"
        );
    }

    // -----------------------------------------------------------------------
    // Batch 3e param deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn create_project_params_deserializes_required_fields() {
        let json =
            r#"{"workspace":"acme","name":"Platform","slug":"platform","task_prefix":"PLT"}"#;
        let params: CreateProjectParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "Platform");
        assert_eq!(params.slug, "platform");
        assert_eq!(params.task_prefix, "PLT");
        assert!(params.visibility.is_none());
        assert!(params.visibility_role.is_none());
    }

    #[test]
    fn create_project_params_deserializes_optional_fields() {
        let json = r#"{"workspace":"acme","name":"Private","slug":"private","task_prefix":"PRV","visibility":"private","visibility_role":"editor"}"#;
        let params: CreateProjectParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.visibility.as_deref(), Some("private"));
        assert_eq!(params.visibility_role.as_deref(), Some("editor"));
    }

    #[test]
    fn update_project_params_all_optional_omitted() {
        let json = r#"{"workspace":"acme","slug":"platform"}"#;
        let params: UpdateProjectParams = serde_json::from_str(json).unwrap();
        assert!(params.name.is_none());
        assert!(params.visibility.is_none());
        assert!(params.task_prefix.is_none());
    }

    #[test]
    fn delete_project_params_deserializes() {
        let json = r#"{"workspace":"acme","slug":"platform","confirm":true}"#;
        let params: DeleteProjectParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.slug, "platform");
        assert!(params.confirm);
    }

    #[test]
    fn create_status_template_params_deserializes() {
        let json = r#"{"workspace":"acme","name":"In Review","color":"yellow"}"#;
        let params: CreateStatusTemplateParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "In Review");
        assert_eq!(params.color.as_deref(), Some("yellow"));
        assert!(params.before.is_none());
        assert!(params.after.is_none());
    }

    #[test]
    fn update_status_template_color_absent_is_none() {
        let json =
            r#"{"workspace":"acme","id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","name":"Done"}"#;
        let params: UpdateStatusTemplateParams = serde_json::from_str(json).unwrap();
        assert!(
            params.color.is_none(),
            "absent color must be None (leave unchanged)"
        );
    }

    #[test]
    fn update_status_template_color_null_clears() {
        let json =
            r#"{"workspace":"acme","id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","color":null}"#;
        let params: UpdateStatusTemplateParams = serde_json::from_str(json).unwrap();
        assert_eq!(
            params.color,
            Some(serde_json::Value::Null),
            "explicit null must be Some(Null) to signal clear"
        );
    }

    #[test]
    fn update_status_template_color_set() {
        let json =
            r#"{"workspace":"acme","id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","color":"blue"}"#;
        let params: UpdateStatusTemplateParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.color, Some(serde_json::Value::String("blue".into())));
    }

    #[test]
    fn delete_status_template_params_deserializes() {
        let json = r#"{"workspace":"acme","id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: DeleteStatusTemplateParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn create_saved_search_params_deserializes() {
        let json = r#"{"workspace":"acme","name":"Open bugs","query":"status:open tag:bug"}"#;
        let params: CreateSavedSearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "Open bugs");
        assert_eq!(params.query, "status:open tag:bug");
    }

    #[test]
    fn rename_saved_search_params_deserializes() {
        let json = r#"{"workspace":"acme","id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","name":"High-priority bugs"}"#;
        let params: RenameSavedSearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "High-priority bugs");
        assert_eq!(params.id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn delete_saved_search_params_deserializes() {
        let json = r#"{"workspace":"acme","id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: DeleteSavedSearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn create_task_view_params_deserializes_empty_filters() {
        let json = r#"{"workspace":"acme","name":"All Tasks","filters":{}}"#;
        let params: CreateTaskViewParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "All Tasks");
        assert!(params.filters.is_object());
    }

    #[test]
    fn create_task_view_params_deserializes_with_filters() {
        let json = r#"{"workspace":"acme","name":"High priority","filters":{"priorities":["high","urgent"],"sort":"priority_desc"}}"#;
        let params: CreateTaskViewParams = serde_json::from_str(json).unwrap();
        assert_eq!(
            params.filters.get("sort").and_then(|v| v.as_str()),
            Some("priority_desc")
        );
    }

    #[test]
    fn update_task_view_params_deserializes() {
        let json = r#"{"workspace":"acme","id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","name":"Renamed","filters":{}}"#;
        let params: UpdateTaskViewParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.name, "Renamed");
        assert_eq!(params.id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn delete_task_view_params_deserializes() {
        let json = r#"{"workspace":"acme","id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: DeleteTaskViewParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn get_info_instructions_reference_workspace_settings_write_tools() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        let instructions = info.instructions.as_deref().unwrap_or("");
        assert!(
            instructions.contains("`create_project`"),
            "instructions must mention create_project"
        );
        assert!(
            instructions.contains("`delete_project`"),
            "instructions must mention delete_project"
        );
        assert!(
            instructions.contains("`create_status_template`"),
            "instructions must mention create_status_template"
        );
        assert!(
            instructions.contains("`create_saved_search`"),
            "instructions must mention create_saved_search"
        );
        assert!(
            instructions.contains("`rename_saved_search`"),
            "instructions must mention rename_saved_search"
        );
        assert!(
            instructions.contains("`create_task_view`"),
            "instructions must mention create_task_view"
        );
        assert!(
            instructions.contains("`delete_task_view`"),
            "instructions must mention delete_task_view"
        );
    }

    // --- parse_bearer_atlas_token ---

    #[test]
    fn bearer_valid_token_is_accepted() {
        let result = parse_bearer_atlas_token("Bearer atlas_abc123");
        assert_eq!(result.unwrap(), "atlas_abc123");
    }

    #[test]
    fn bearer_valid_token_with_long_value() {
        let value = "Bearer atlas_very-long-token-value-XYZ-1234567890";
        let result = parse_bearer_atlas_token(value);
        assert_eq!(
            result.unwrap(),
            "atlas_very-long-token-value-XYZ-1234567890"
        );
    }

    #[test]
    fn bearer_missing_scheme_prefix_is_rejected() {
        let result = parse_bearer_atlas_token("atlas_abc123");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Bearer"), "error must mention Bearer scheme");
    }

    #[test]
    fn bearer_wrong_scheme_is_rejected() {
        let result = parse_bearer_atlas_token("Token atlas_abc123");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("Bearer"), "error must mention Bearer scheme");
    }

    #[test]
    fn bearer_empty_token_after_prefix_is_rejected() {
        let result = parse_bearer_atlas_token("Bearer ");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("empty"), "error must indicate empty token");
    }

    #[test]
    fn bearer_non_atlas_prefix_is_rejected() {
        let result = parse_bearer_atlas_token("Bearer sk_some_other_token");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("atlas_"),
            "error must indicate the required prefix"
        );
    }

    #[test]
    fn bearer_empty_string_is_rejected() {
        let result = parse_bearer_atlas_token("");
        assert!(result.is_err());
    }

    #[test]
    fn bearer_only_prefix_no_token_chars_is_rejected() {
        let result = parse_bearer_atlas_token("Bearer atlas_");
        assert_eq!(
            result.unwrap(),
            "atlas_",
            "atlas_ alone is accepted (validation of minimum length is atlas_server's responsibility)"
        );
    }

    #[test]
    fn bearer_extra_leading_whitespace_is_rejected() {
        let result = parse_bearer_atlas_token(" Bearer atlas_abc");
        assert!(result.is_err(), "leading whitespace should not be stripped");
    }
}
