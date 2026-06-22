#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod response;

use std::sync::Arc;

use atlas_api::dtos::boards_tasks::{
    AddAssigneeRequest, CreateTaskRequest, MoveTaskRequest, TaskPropertiesDto, UpdateTaskRequest,
    WorkspaceTaskQueryParams,
};
use atlas_client::AtlasClient;
use rmcp::{
    ServerHandler,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use response::{
    Detail, enrich_client_error, envelope_page, map_present_value, match_columns_by_name,
    parse_csv, parse_detail, project_activity_entry, project_assignee, project_attachment,
    project_backlink, project_board_summary, project_checklist_item, project_column,
    project_document_compact, project_document_full, project_document_summary, project_folder,
    project_principal, project_project, project_reference, project_revision_content,
    project_revision_meta, project_saved_search, project_search_hit, project_tag,
    project_task_backlink, project_task_compact, project_task_full, project_task_row,
    project_task_view, project_workspace, require_confirm, resolve_column_id_on_board,
    validate_assignee_type, validate_priority, wrap_vec,
};

const ATLAS_INSTRUCTIONS: &str = "\
Atlas is a personal knowledge base for notes and tasks. \
Use `search` to retrieve content by keyword or structured filters \
(status:open, tag:rust, etc.) before acting on it. \
Use `get_document` to read a note's content, `list_tasks` to browse tasks \
with status/board/assignee/label filters, and `get_task` for a single task's details. \
Use `list_documents` to enumerate documents in a project, `list_folders` for the \
folder hierarchy, `list_boards` to discover boards, and `list_columns` to map \
column names to IDs for use in status filters. \
For workspace context discovery use: `list_workspaces` to find available workspaces, \
`list_projects` to enumerate projects in a workspace, `list_members` to resolve \
member names to IDs for assignee filters, `list_tags` for the tag registry, \
`list_used_labels` for labels currently in use on tasks, `list_saved_searches` and \
`list_task_views` for the caller's saved filters. \
For link graph exploration use: `get_task_references` for a task's OUTBOUND references \
(tasks/documents the task points to), `get_task_backlinks` for INBOUND references \
(other tasks that point to this task), and `get_document_backlinks` for documents/tasks \
that link to a given document. \
For task depth use: `list_checklist` for a task's checklist items, `list_activity` for \
its change history (who moved/assigned/commented). \
For document depth use: `list_document_history` to browse revision metadata, \
`get_document_revision` to fetch the full content of a specific historical revision \
(by seq number from `list_document_history`), and `list_attachments` to enumerate \
file attachments on a document. \
For task mutations use: `create_task` (board + column resolved by name), `update_task` \
(PATCH semantics — omit a field to leave it unchanged, pass null to clear), `move_task` \
(target column resolved by name; errors with the column list on a miss), `delete_task` \
(requires confirm: true), `add_task_assignee` / `remove_task_assignee` for assignees. \
Prefer narrow queries over broad ones; follow up with targeted reads rather than \
enumerating all results.";

/// MCP server backed by an Atlas HTTP API endpoint.
///
/// Holds a single shared client, built once on construction and reused across
/// all tool calls. Cloning the handler shares the same underlying client.
#[derive(Clone)]
pub struct AtlasMcp {
    client: Arc<AtlasClient>,
}

impl AtlasMcp {
    /// Returns a reference to the underlying HTTP client, for pre-serve diagnostics.
    pub fn client(&self) -> &AtlasClient {
        &self.client
    }

    /// Constructs an `AtlasMcp` with the given base URL and required API token.
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

        let mut client = AtlasClient::new(base_url);
        client.set_token(token);

        Ok(Self {
            client: Arc::new(client),
        })
    }
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
    #[serde(default)]
    pub priority: Option<serde_json::Value>,
    /// Due date (RFC 3339). Omit to leave unchanged. Pass JSON null to clear.
    #[serde(default)]
    pub due_date: Option<serde_json::Value>,
    /// Estimate (story points). Omit to leave unchanged. Pass JSON null to clear.
    #[serde(default)]
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
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> Result<String, String> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = self
            .client
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
    ) -> Result<String, String> {
        let doc = self
            .client
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
    ) -> Result<String, String> {
        let priorities = params
            .priority
            .as_deref()
            .map(parse_csv)
            .unwrap_or_default();

        let labels = params.label.as_deref().map(parse_csv).unwrap_or_default();

        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let column_ids = if let Some(status_name) = &params.status {
            self.resolve_column_ids(&params.workspace, params.board.as_deref(), status_name)
                .await?
        } else {
            Vec::new()
        };

        let board_id = if let Some(board) = &params.board {
            Some(self.resolve_board_id(&params.workspace, board).await?)
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

        let page = self
            .client
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
    ) -> Result<String, String> {
        let task = self
            .client
            .get_task(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| format!("get_task '{}' failed: {e}", params.readable_id))?;

        let result = match parse_detail(params.detail.as_deref()) {
            Detail::Compact => project_task_compact(&task),
            Detail::Full => {
                let refs = self
                    .client
                    .list_references(&params.workspace, &params.readable_id)
                    .await
                    .map(|v| v.into_iter().map(project_reference).collect::<Vec<_>>())
                    .map_err(|e| format!("list_references failed: {e}"));

                let subtasks = self
                    .client
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
    ) -> Result<String, String> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = self
            .client
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
    ) -> Result<String, String> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = self
            .client
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
    ) -> Result<String, String> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = self
            .client
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
    ) -> Result<String, String> {
        let board_id_str = self
            .resolve_board_id(&params.workspace, &params.board)
            .await?;

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board_id '{board_id_str}' is not a valid UUID"))?;

        let cols = self
            .client
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
    ) -> Result<String, String> {
        let tags = self
            .client
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
    ) -> Result<String, String> {
        let labels = self
            .client
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
    ) -> Result<String, String> {
        let members = self
            .client
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
    ) -> Result<String, String> {
        let workspaces = self
            .client
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
    ) -> Result<String, String> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = self
            .client
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
    ) -> Result<String, String> {
        let searches = self
            .client
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
    ) -> Result<String, String> {
        let views = self
            .client
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
    ) -> Result<String, String> {
        let refs = self
            .client
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
    ) -> Result<String, String> {
        let page = self
            .client
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
    ) -> Result<String, String> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = self
            .client
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
    ) -> Result<String, String> {
        let items = self
            .client
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
    ) -> Result<String, String> {
        let page = self
            .client
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
    ) -> Result<String, String> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = self
            .client
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
    ) -> Result<String, String> {
        let rev = self
            .client
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
    ) -> Result<String, String> {
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = self
            .client
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
    ) -> Result<String, String> {
        let board_id_str = self
            .resolve_board_id(&params.workspace, &params.board)
            .await?;
        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let cols = self
            .client
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

        let task = self
            .client
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
    ) -> Result<String, String> {
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

        let task = self
            .client
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
    ) -> Result<String, String> {
        let board_ref = params.board.as_deref().unwrap_or(&params.readable_id);

        let board_id_str = if params.board.is_some() {
            self.resolve_board_id(&params.workspace, board_ref).await?
        } else {
            // No board supplied: fetch the task first to get its board_id.
            let task = self
                .client
                .get_task(&params.workspace, &params.readable_id)
                .await
                .map_err(|e| enrich_client_error(e, "get_task"))?;
            task.board_id.to_string()
        };

        let board_uuid: uuid::Uuid = board_id_str
            .parse()
            .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))?;

        let cols = self
            .client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

        let column_id = resolve_column_id_on_board(&params.column, &cols)?;

        let body = MoveTaskRequest {
            column_id,
            before: None,
            after: None,
        };

        let task = self
            .client
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
    ) -> Result<String, String> {
        require_confirm(params.confirm, "task", &params.readable_id)?;

        self.client
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
    ) -> Result<String, String> {
        validate_assignee_type(&params.assignee_type)?;

        let assignee_id: uuid::Uuid = params
            .assignee_id
            .parse()
            .map_err(|_| format!("assignee_id '{}' is not a valid UUID", params.assignee_id))?;

        let body = AddAssigneeRequest {
            assignee_type: params.assignee_type,
            assignee_id,
        };

        let assignee = self
            .client
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
    ) -> Result<String, String> {
        self.client
            .remove_assignee(&params.workspace, &params.readable_id, &params.assignee_ref)
            .await
            .map_err(|e| enrich_client_error(e, "remove_task_assignee"))?;

        let result = json!({
            "removed": true,
            "assignee_ref": params.assignee_ref,
        });
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
        ws: &str,
        board: Option<&str>,
        status_name: &str,
    ) -> Result<Vec<String>, String> {
        if let Some(board_ref) = board {
            let board_id = self.resolve_board_id(ws, board_ref).await?;
            let board_uuid: uuid::Uuid = board_id
                .parse()
                .map_err(|_| format!("resolved board_id '{board_id}' is not a valid UUID"))?;
            let cols = self
                .client
                .list_columns(ws, board_uuid)
                .await
                .map_err(|e| format!("list_columns failed: {e}"))?;
            return Ok(match_columns_by_name(status_name, &cols));
        }

        // No board given: workspace-wide walk (D-WSCOL — O(projects + boards) GETs).
        let mut all_cols = Vec::new();
        let mut project_cursor: Option<String> = None;

        loop {
            let projects = self
                .client
                .list_projects(ws, project_cursor.as_deref(), Some(200))
                .await
                .map_err(|e| format!("list_projects failed: {e}"))?;

            for project in &projects.items {
                let mut board_cursor: Option<String> = None;
                loop {
                    let boards = self
                        .client
                        .list_boards(ws, &project.slug, board_cursor.as_deref(), Some(200))
                        .await
                        .map_err(|e| {
                            format!("list_boards for project '{}' failed: {e}", project.slug)
                        })?;

                    for board in &boards.items {
                        let cols = self.client.list_columns(ws, board.id).await.map_err(|e| {
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
    async fn resolve_board_id(&self, ws: &str, board_ref: &str) -> Result<String, String> {
        if uuid::Uuid::parse_str(board_ref).is_ok() {
            return Ok(board_ref.to_string());
        }

        let needle = board_ref.to_ascii_lowercase();
        let mut project_cursor: Option<String> = None;

        loop {
            let projects = self
                .client
                .list_projects(ws, project_cursor.as_deref(), Some(200))
                .await
                .map_err(|e| format!("list_projects failed: {e}"))?;

            for project in &projects.items {
                let mut board_cursor: Option<String> = None;
                loop {
                    let boards = self
                        .client
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
    fn clone_shares_client_arc() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let cloned = server.clone();
        assert!(std::ptr::eq(
            server.client() as *const AtlasClient,
            cloned.client() as *const AtlasClient
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
}
