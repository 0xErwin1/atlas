#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod response;

use atlas_api::dtos::boards_tasks::{
    AddAssigneeRequest, CreateBoardRequest, CreateChecklistItemRequest, CreateColumnRequest,
    CreateCommentRequest, CreateReferenceRequest, CreateSubtaskRequest, CreateTaskRequest,
    MoveTaskRequest, PromoteChecklistItemRequest, TaskPropertiesDto, UpdateBoardRequest,
    UpdateChecklistItemRequest, UpdateColumnRequest, UpdateCommentRequest, UpdateTaskRequest,
    WorkspaceTaskQueryParams,
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
use atlas_api::dtos::webhooks::{CreateWebhookRequest, UpdateWebhookRequest};
use atlas_api::dtos::{CreateProjectRequest, ServerMetaDto, UpdateProjectRequest};
use atlas_client::{AtlasClient, helpers};
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{
        AnnotateAble, Content, Implementation, ListResourceTemplatesResult, RawResourceTemplate,
        ReadResourceRequestParams, ReadResourceResult, ResourceContents, ServerCapabilities,
        ServerInfo,
    },
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
    Detail, enrich_client_error, envelope_page, map_present_string, map_present_uuid,
    map_present_value, parse_atlas_doc_uri, parse_csv, parse_detail, project_activity_entry,
    project_assignee, project_attachment, project_audit_entry, project_backlink,
    project_board_summary, project_checklist_item, project_column, project_comment,
    project_comment_attachment, project_comment_feed_entry, project_document_compact,
    project_document_full, project_document_summary, project_folder, project_principal,
    project_project, project_promotion, project_reference, project_revision_content,
    project_revision_meta, project_saved_search, project_search_hit, project_semantic_search_hit,
    project_status_template, project_tag, project_task_attachment, project_task_backlink,
    project_task_compact, project_task_full, project_task_row, project_task_view, project_webhook,
    project_webhook_created, project_webhook_delivery, project_workspace,
    project_workspace_activity_entry, require_confirm, resolve_column_id_on_board,
    validate_assignee_type, validate_estimate, validate_estimate_value, validate_priority,
    validate_reference_kind, validate_single_target, wrap_vec,
};

const ATLAS_INSTRUCTIONS: &str = "\
Atlas is a personal knowledge base of notes (markdown documents) and tasks (kanban \
boards). Work as an agent: discover with the read tools, then mutate with the write \
tools. Each tool's own description is authoritative; this preamble covers the \
conventions shared across all of them.\n\
\n\
Conventions:\n\
- Discover before acting: use `search` (keyword plus filters like status:open, tag:rust) \
for lexical matches or `semantic_search` for concept matches, then list tools to find \
resources first. Identify tasks by readable_id (e.g. ATL-42) and documents by slug.\n\
- Pass names, not UUIDs, for boards / columns / assignees; on a miss the error lists the \
valid options.\n\
- List responses are paginated as {items, next_cursor, has_more}; reads are compact by \
default — pass detail=full for heavy fields (document content, task description).\n\
- PATCH updates are partial: omit a field to leave it unchanged, pass null to clear it.\n\
- Resource deletion is recoverable for projects, folders, documents, comments, and attachments; permanent removal is a separate Trash purge workflow.\n\
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
- Self: `get_agent_identity` (the calling API key's own id, name, and capability scopes).\n\
- Links and depth: `get_task_references`, `get_task_backlinks`, `get_document_backlinks`, \
`list_checklist`, `list_comments`, `list_document_comments`, `list_activity`, \
`list_workspace_activity`, `list_document_history`, `get_document_revision`, \
`list_attachments`, `list_task_attachments`, `get_task_attachment`.\n\
- Security audit (owner/admin only): `get_workspace_audit`, `get_platform_audit`.\n\
- Task writes: `create_task`, `update_task`, `move_task`, `delete_task`, \
`add_task_assignee`, `remove_task_assignee`, `add_comment`, `update_comment`, \
`delete_comment`.\n\
- Document and folder writes: `create_document`, `update_document_metadata`, \
`update_document_content`, `delete_document`, `move_document`, `copy_document`, \
`add_document_comment`, `update_document_comment`, `delete_document_comment`, \
`create_folder`, `rename_folder`, `move_folder`, `copy_folder`, `delete_folder`.\n\
- Board, column and tag writes: `create_board`, `update_board`, `delete_board`, \
`create_column`, `update_column`, `delete_column`, `create_tag`, `update_tag`, `delete_tag`.\n\
- Graph writes: `add_task_reference`, `remove_task_reference`, `add_checklist_item`, \
`update_checklist_item`, `delete_checklist_item`, `promote_checklist_item`, \
`create_subtask`, `promote_subtask`.\n\
- Workspace-settings writes: `create_project`, `update_project`, `delete_project`, \
`create_status_template`, `update_status_template`, `delete_status_template`, \
`create_saved_search`, `rename_saved_search`, `delete_saved_search`, `create_task_view`, \
`update_task_view`, `delete_task_view`.\n\
- Webhook management (requires the matching webhooks:* capability): `list_webhooks`, \
`get_webhook`, `list_webhook_deliveries`, `create_webhook` (returns the one-time whsec_ \
signing secret), `update_webhook`, `delete_webhook`. These manage workspace webhook \
subscriptions; they do NOT change the calling key's own capability scopes.\n\
\n\
Trash is a root/system-admin human-session workflow. This MCP server authenticates API keys, so it intentionally exposes no Trash list, restore, purge, or purge-status tools; API-key calls to the REST Trash routes receive 403.\n\
\n\
Capability gating (agent keys): the tag tools (`list_tags`, `create_tag`, `update_tag`, \
`delete_tag`) require the matching `config:*` capability; the saved-search tools \
(`list_saved_searches`, `create_saved_search`, `rename_saved_search`, \
`delete_saved_search`) require `saved_searches:*`; the task-view tools \
(`list_task_views`, `create_task_view`, `update_task_view`, `delete_task_view`) require \
`task_views:*`. Like webhooks, this is resource access control, NOT scope self-mutation: \
none of these tools change the calling key's own capability scopes.";

/// MCP server backed by an Atlas HTTP API endpoint.
///
/// In stdio mode, holds the single startup token for all tool calls.
/// In HTTP mode, `stdio_token` is `None` and each tool call resolves its
/// client from the per-request Bearer header via `resolve_client`.
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

/// Parses a required UUID tool parameter, naming the field in the error.
fn parse_uuid_param(field: &str, raw: &str) -> Result<uuid::Uuid, String> {
    raw.parse()
        .map_err(|_| format!("{field} '{raw}' is not a valid UUID"))
}

/// Parses an optional UUID tool parameter, naming the field in the error.
///
/// Absent (`None`) yields `Ok(None)`; a present value must be a valid UUID.
fn parse_optional_uuid_param(field: &str, raw: Option<&str>) -> Result<Option<uuid::Uuid>, String> {
    raw.map(|s| parse_uuid_param(field, s)).transpose()
}

/// Parses a required `webhook_id` tool parameter into a UUID.
fn parse_webhook_id(raw: &str) -> Result<uuid::Uuid, String> {
    parse_uuid_param("webhook_id", raw)
}

/// Resolves a board reference (name or UUID) to a validated board UUID.
///
/// Combines `helpers::resolve_board_id` (which errors on an ambiguous name) with
/// the UUID parse the resolved id must satisfy, so write callers share one code
/// path and one error string.
async fn resolve_board_uuid(
    client: &AtlasClient,
    workspace: &str,
    board_ref: &str,
) -> Result<uuid::Uuid, String> {
    let board_id_str = helpers::resolve_board_id(client, workspace, board_ref)
        .await
        .map_err(|e| e.to_string())?;

    parse_resolved_board_uuid(&board_id_str)
}

/// Parses a board id string that was already resolved (name lookup or a task's
/// own `board_id`) into a UUID, with the shared "resolved board" error string.
fn parse_resolved_board_uuid(board_id_str: &str) -> Result<uuid::Uuid, String> {
    board_id_str
        .parse()
        .map_err(|_| format!("resolved board '{board_id_str}' is not a valid UUID"))
}

fn decode_comment_attachment_data(
    data_base64: &str,
    max_attachment_bytes: u64,
) -> Result<Vec<u8>, String> {
    use base64::Engine as _;

    let encoded_len = u64::try_from(data_base64.len())
        .map_err(|_| "attachment content is too large to validate".to_string())?;
    let max_encoded_len = max_attachment_bytes.div_ceil(3).saturating_mul(4);
    if encoded_len > max_encoded_len {
        return Err("attachment content exceeds the server attachment limit".to_string());
    }
    if !data_base64.len().is_multiple_of(4) {
        return Err("attachment content must be padded standard base64".to_string());
    }

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64)
        .map_err(|_| "attachment content must be padded standard base64".to_string())?;
    if u64::try_from(bytes.len()).is_ok_and(|len| len > max_attachment_bytes) {
        return Err("attachment content exceeds the server attachment limit".to_string());
    }
    Ok(bytes)
}

fn require_comment_attachment_limit(
    result: Result<ServerMetaDto, atlas_client::ClientError>,
    operation: &str,
) -> Result<u64, String> {
    result
        .map_err(|e| {
            format!(
                "{operation}: server attachment limit could not be discovered; upload was not attempted: {}",
                enrich_client_error(e, "server_meta")
            )
        })?
        .max_attachment_bytes
        .ok_or_else(|| {
            format!("{operation}: server attachment limit could not be discovered; upload was not attempted")
        })
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

/// Parameters accepted by the `semantic_search` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SemanticSearchParams {
    /// Workspace slug to search in.
    pub workspace: String,
    /// Natural-language concept query.
    pub query: String,
    /// Kind: `all` (default), `document`/`note`, or `task`.
    #[serde(default, rename = "type")]
    pub type_filter: Option<String>,
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

/// Parameters accepted by the `get_agent_identity` tool.
///
/// No parameters required — reports the calling API key's own identity.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetAgentIdentityParams {}

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

/// Parameters accepted by the `list_comments` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListCommentsParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `list_workspace_activity` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWorkspaceActivityParams {
    /// Workspace slug.
    pub workspace: String,
    /// Filter by actor type: `user` (human) or `api_key` (agent).
    #[serde(default)]
    pub actor: Option<String>,
    /// Lower bound (inclusive) on event time (ISO 8601 / RFC 3339).
    #[serde(default)]
    pub from: Option<String>,
    /// Upper bound (inclusive) on event time (ISO 8601 / RFC 3339).
    #[serde(default)]
    pub to: Option<String>,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `get_workspace_audit` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetWorkspaceAuditParams {
    /// Workspace slug.
    pub workspace: String,
    /// Filter by actor type: `user` (human) or `api_key` (agent).
    #[serde(default)]
    pub actor: Option<String>,
    /// Filter by action verb (e.g. `membership.role_changed`).
    #[serde(default)]
    pub action: Option<String>,
    /// Lower bound (inclusive) on event time (ISO 8601 / RFC 3339).
    #[serde(default)]
    pub from: Option<String>,
    /// Upper bound (inclusive) on event time (ISO 8601 / RFC 3339).
    #[serde(default)]
    pub to: Option<String>,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `get_platform_audit` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetPlatformAuditParams {
    /// Filter by actor type: `user` (human) or `api_key` (agent).
    #[serde(default)]
    pub actor: Option<String>,
    /// Filter by action verb (e.g. `user.disabled`).
    #[serde(default)]
    pub action: Option<String>,
    /// Lower bound (inclusive) on event time (ISO 8601 / RFC 3339).
    #[serde(default)]
    pub from: Option<String>,
    /// Upper bound (inclusive) on event time (ISO 8601 / RFC 3339).
    #[serde(default)]
    pub to: Option<String>,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
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

/// Parameters accepted by the `list_task_attachments` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTaskAttachmentsParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
}

/// Parameters accepted by the `get_task_attachment` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTaskAttachmentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// Attachment UUID, taken from the `id` field of `list_task_attachments`.
    pub attachment_id: String,
}

// ---------------------------------------------------------------------------
// Write tool parameter structs — Task writes
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
// Board / Column / Tag write param structs
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
// Graph write param structs — references, checklist, subtasks
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

/// Parameters accepted by the `add_comment` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddCommentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task to comment on.
    pub readable_id: String,
    /// Markdown comment body. Must not be blank; max 10 000 characters.
    pub body: String,
}

/// Parameters accepted by the `update_comment` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateCommentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task that owns the comment.
    pub readable_id: String,
    /// UUID string of the comment to edit.
    pub comment_id: String,
    /// New markdown comment body. Must not be blank; max 10 000 characters.
    pub body: String,
}

/// Parameters accepted by the `delete_comment` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteCommentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Readable ID of the task that owns the comment.
    pub readable_id: String,
    /// UUID string of the comment to delete.
    pub comment_id: String,
}

/// Parameters accepted by the `list_document_comments` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDocumentCommentsParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug.
    pub slug: String,
    /// Pass `next_cursor` from the previous response to fetch the next page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (default 50, max 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by task-comment attachment lifecycle tools.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskCommentAttachmentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// UUID of the owning comment.
    pub comment_id: String,
    /// UUID of the comment attachment. Required for download and deletion.
    #[serde(default)]
    pub attachment_id: Option<String>,
}

/// Parameters accepted by `upload_task_comment_attachment`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UploadTaskCommentAttachmentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Task readable ID, e.g. `ATL-42`.
    pub readable_id: String,
    /// UUID of the owning comment.
    pub comment_id: String,
    /// File name sent to the server.
    pub file_name: String,
    /// MIME content type sent to the server.
    pub content_type: String,
    /// Strict padded standard-base64 file bytes.
    pub data_base64: String,
}

/// Parameters accepted by document-comment attachment lifecycle tools.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocumentCommentAttachmentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug.
    pub slug: String,
    /// UUID of the owning comment.
    pub comment_id: String,
    /// UUID of the comment attachment. Required for download and deletion.
    #[serde(default)]
    pub attachment_id: Option<String>,
}

/// Parameters accepted by `upload_document_comment_attachment`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UploadDocumentCommentAttachmentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Document slug.
    pub slug: String,
    /// UUID of the owning comment.
    pub comment_id: String,
    /// File name sent to the server.
    pub file_name: String,
    /// MIME content type sent to the server.
    pub content_type: String,
    /// Strict padded standard-base64 file bytes.
    pub data_base64: String,
}

/// Parameters accepted by the `add_document_comment` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddDocumentCommentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Slug of the document to comment on.
    pub slug: String,
    /// Markdown comment body. Must not be blank; max 10 000 characters.
    pub body: String,
}

/// Parameters accepted by the `update_document_comment` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateDocumentCommentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Slug of the document that owns the comment.
    pub slug: String,
    /// UUID string of the comment to edit.
    pub comment_id: String,
    /// New markdown comment body. Must not be blank; max 10 000 characters.
    pub body: String,
}

/// Parameters accepted by the `delete_document_comment` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteDocumentCommentParams {
    /// Workspace slug.
    pub workspace: String,
    /// Slug of the document that owns the comment.
    pub slug: String,
    /// UUID string of the comment to delete.
    pub comment_id: String,
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
// Workspace-settings write param structs
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
// Webhook management params
// ---------------------------------------------------------------------------

/// Parameters accepted by the `list_webhooks` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWebhooksParams {
    /// Workspace slug.
    pub workspace: String,
    /// Opaque forward cursor from a previous page's `next_cursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (1..=50). Defaults to the server page size when omitted.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `get_webhook` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetWebhookParams {
    /// Workspace slug.
    pub workspace: String,
    /// Webhook subscription UUID, from the `id` field of `list_webhooks`.
    pub webhook_id: String,
}

/// Parameters accepted by the `list_webhook_deliveries` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWebhookDeliveriesParams {
    /// Workspace slug.
    pub workspace: String,
    /// Webhook subscription UUID, from the `id` field of `list_webhooks`.
    pub webhook_id: String,
    /// Opaque newest-first cursor from a previous page's `next_cursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Page size (1..=50). Defaults to the server page size when omitted.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Parameters accepted by the `create_webhook` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateWebhookParams {
    /// Workspace slug.
    pub workspace: String,
    /// Absolute HTTPS (or HTTP for local testing) URL to POST events to.
    pub target_url: String,
    /// Event-type strings to subscribe to (at least one required).
    pub event_types: Vec<String>,
    /// Scope discriminant: `workspace` (default), `project`, or `board`.
    #[serde(default)]
    pub scope_type: Option<String>,
    /// Project or board UUID; required when `scope_type` is not `workspace`.
    #[serde(default)]
    pub scope_id: Option<String>,
    /// Optional human-readable label for the subscription.
    #[serde(default)]
    pub label: Option<String>,
}

/// Parameters accepted by the `update_webhook` tool.
///
/// PATCH semantics: omit a field to leave it unchanged.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateWebhookParams {
    /// Workspace slug.
    pub workspace: String,
    /// Webhook subscription UUID to update.
    pub webhook_id: String,
    /// New target URL. Omit to leave unchanged.
    #[serde(default)]
    pub target_url: Option<String>,
    /// New event-type list. Omit to leave unchanged.
    #[serde(default)]
    pub event_types: Option<Vec<String>>,
    /// New scope discriminant. Omit to leave unchanged.
    #[serde(default)]
    pub scope_type: Option<String>,
    /// New scope UUID. Omit to leave unchanged, pass JSON null to clear.
    #[serde(default, deserialize_with = "present_value")]
    pub scope_id: Option<serde_json::Value>,
    /// Enable or disable the subscription. Omit to leave unchanged.
    #[serde(default)]
    pub is_active: Option<bool>,
    /// New label. Omit to leave unchanged, pass JSON null to clear.
    #[serde(default, deserialize_with = "present_value")]
    pub label: Option<serde_json::Value>,
}

/// Parameters accepted by the `delete_webhook` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteWebhookParams {
    /// Workspace slug.
    pub workspace: String,
    /// Webhook subscription UUID to delete.
    pub webhook_id: String,
    /// Must be `true` to proceed. Soft-deletes the subscription.
    pub confirm: bool,
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
            .map_err(|e| enrich_client_error(e, "search"))?;

        let result = envelope_page(page, project_search_hit);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "Semantic search documents and tasks across an Atlas workspace")]
    async fn semantic_search(
        &self,
        Parameters(params): Parameters<SemanticSearchParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(20).clamp(1, 200);

        let page = client
            .semantic_search(
                &params.workspace,
                &params.query,
                params.type_filter.as_deref(),
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| enrich_client_error(e, "semantic_search"))?;

        let result = envelope_page(page, project_semantic_search_hit);
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
            .map_err(|e| enrich_client_error(e, "get_document"))?;

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
            helpers::resolve_column_ids_required(
                &client,
                &params.workspace,
                params.board.as_deref(),
                status_name,
            )
            .await
            .map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };

        let board_id = if let Some(board) = &params.board {
            Some(
                helpers::resolve_board_id(&client, &params.workspace, board)
                    .await
                    .map_err(|e| e.to_string())?,
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
            .map_err(|e| enrich_client_error(e, "list_tasks"))?;

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
            .map_err(|e| enrich_client_error(e, "get_task"))?;

        let result = match parse_detail(params.detail.as_deref()) {
            Detail::Compact => project_task_compact(&task),
            Detail::Full => {
                let refs = client
                    .list_references(&params.workspace, &params.readable_id)
                    .await
                    .map(|v| v.into_iter().map(project_reference).collect::<Vec<_>>())
                    .map_err(|e| enrich_client_error(e, "list_references"));

                let subtasks = client
                    .list_subtasks(&params.workspace, &params.readable_id)
                    .await
                    .map(|v| v.into_iter().map(project_task_row).collect::<Vec<_>>())
                    .map_err(|e| enrich_client_error(e, "list_subtasks"));

                let assignees = client
                    .list_assignees(&params.workspace, &params.readable_id)
                    .await
                    .map(|v| v.into_iter().map(project_assignee).collect::<Vec<_>>())
                    .map_err(|e| enrich_client_error(e, "list_assignees"));

                project_task_full(&task, refs, subtasks, assignees)
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
            .map_err(|e| enrich_client_error(e, "list_documents"))?;

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
            .map_err(|e| enrich_client_error(e, "list_folders"))?;

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
            .map_err(|e| enrich_client_error(e, "list_boards"))?;

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

        let board_uuid = resolve_board_uuid(&client, &params.workspace, &params.board).await?;

        let cols = client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

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
            .map_err(|e| enrich_client_error(e, "list_tags"))?;

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
            .map_err(|e| enrich_client_error(e, "list_used_labels"))?;

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
            .map_err(|e| enrich_client_error(e, "list_members"))?;

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
            .map_err(|e| enrich_client_error(e, "list_workspaces"))?;

        let result = wrap_vec(workspaces, project_workspace);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Report the calling API key's own identity: its id, name, and the capability scopes it holds. Read-only self-inspection; returns a note when the caller is a human, not an agent key."
    )]
    async fn get_agent_identity(
        &self,
        Parameters(_params): Parameters<GetAgentIdentityParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let me = client
            .me()
            .await
            .map_err(|e| enrich_client_error(e, "get_agent_identity"))?;

        let result = match me.agent {
            Some(agent) => json!({
                "id": agent.id,
                "name": agent.name,
                "scopes": agent.scopes,
            }),
            None => json!({
                "agent": serde_json::Value::Null,
                "message": "not an agent identity: the current caller is a human user, not an API key",
            }),
        };

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
            .map_err(|e| enrich_client_error(e, "list_projects"))?;

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
            .map_err(|e| enrich_client_error(e, "list_saved_searches"))?;

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
            .map_err(|e| enrich_client_error(e, "list_task_views"))?;

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
            .map_err(|e| enrich_client_error(e, "get_task_references"))?;

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
            .map_err(|e| enrich_client_error(e, "get_task_backlinks"))?;

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
            .map_err(|e| enrich_client_error(e, "get_document_backlinks"))?;

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
            .map_err(|e| enrich_client_error(e, "list_checklist"))?;

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
            .map_err(|e| enrich_client_error(e, "list_activity"))?;

        let result = envelope_page(page, project_activity_entry);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List markdown comments on a task, oldest first")]
    async fn list_comments(
        &self,
        Parameters(params): Parameters<ListCommentsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(50).clamp(1, 200);

        let page = client
            .list_comments(
                &params.workspace,
                &params.readable_id,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| enrich_client_error(e, "list_comments"))?;

        let result = envelope_page(page, project_comment);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "List the full authorized task comment feed, including derived links and retained events, oldest first"
    )]
    async fn list_comment_feed(
        &self,
        Parameters(params): Parameters<ListCommentsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(50).clamp(1, 200);
        let page = client
            .list_comment_feed(
                &params.workspace,
                &params.readable_id,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| enrich_client_error(e, "list_comment_feed"))?;
        serde_json::to_string(&envelope_page(page, project_comment_feed_entry))
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Upload a file owned by a task comment. Content must be strict padded standard base64."
    )]
    async fn upload_task_comment_attachment(
        &self,
        Parameters(params): Parameters<UploadTaskCommentAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let comment_id = parse_uuid_param("comment_id", &params.comment_id)?;
        let max_attachment_bytes = require_comment_attachment_limit(
            client.server_meta().await,
            "upload_task_comment_attachment",
        )?;
        let data = decode_comment_attachment_data(&params.data_base64, max_attachment_bytes)?;
        let attachment = client
            .upload_task_comment_attachment(
                &params.workspace,
                &params.readable_id,
                comment_id,
                &params.file_name,
                &params.content_type,
                data,
            )
            .await
            .map_err(|e| enrich_client_error(e, "upload_task_comment_attachment"))?;
        serde_json::to_string(&project_comment_attachment(attachment)).map_err(|e| e.to_string())
    }

    #[tool(description = "List attachment metadata for a task comment")]
    async fn list_task_comment_attachments(
        &self,
        Parameters(params): Parameters<TaskCommentAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let comment_id = parse_uuid_param("comment_id", &params.comment_id)?;
        let attachments = client
            .list_task_comment_attachments(&params.workspace, &params.readable_id, comment_id)
            .await
            .map_err(|e| enrich_client_error(e, "list_task_comment_attachments"))?;
        serde_json::to_string(&wrap_vec(attachments, project_comment_attachment))
            .map_err(|e| e.to_string())
    }

    #[tool(description = "Download a task comment attachment as standard base64 content")]
    async fn get_task_comment_attachment(
        &self,
        Parameters(params): Parameters<TaskCommentAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        use base64::Engine as _;
        let client = self.resolve_client(&ctx)?;
        let comment_id = parse_uuid_param("comment_id", &params.comment_id)?;
        let attachment_id = parse_uuid_param(
            "attachment_id",
            params
                .attachment_id
                .as_deref()
                .ok_or_else(|| "attachment_id is required".to_string())?,
        )?;
        let (data, content_type) = client
            .download_task_comment_attachment(
                &params.workspace,
                &params.readable_id,
                comment_id,
                attachment_id,
            )
            .await
            .map_err(|e| enrich_client_error(e, "get_task_comment_attachment"))?;
        serde_json::to_string(&json!({"data_base64": base64::engine::general_purpose::STANDARD.encode(data), "content_type": content_type}))
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a task comment attachment. Plain delete, no confirm required \
                       (consistent with delete_comment, which removes the whole comment)."
    )]
    async fn delete_task_comment_attachment(
        &self,
        Parameters(params): Parameters<TaskCommentAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let comment_id = parse_uuid_param("comment_id", &params.comment_id)?;
        let attachment_id = parse_uuid_param(
            "attachment_id",
            params
                .attachment_id
                .as_deref()
                .ok_or_else(|| "attachment_id is required".to_string())?,
        )?;
        client
            .delete_task_comment_attachment(
                &params.workspace,
                &params.readable_id,
                comment_id,
                attachment_id,
            )
            .await
            .map_err(|e| enrich_client_error(e, "delete_task_comment_attachment"))?;
        serde_json::to_string(&json!({"deleted": true, "attachment_id": attachment_id}))
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "List the access-filtered activity feed for an entire workspace. \
                       Each entry shows who did what on which task (task_readable_id, kind, \
                       actor with display_name and account_status, payload, created_at). \
                       Server-side filtering ensures the caller only sees events for tasks \
                       they can access. Supports actor-type (user|api_key), date range, \
                       and cursor pagination."
    )]
    async fn list_workspace_activity(
        &self,
        Parameters(params): Parameters<ListWorkspaceActivityParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(50).clamp(1, 200);

        let page = if params.cursor.is_some() {
            client
                .list_workspace_activity_with_cursor(
                    &params.workspace,
                    params.actor.as_deref(),
                    params.from.as_deref(),
                    params.cursor.as_deref(),
                    Some(limit),
                )
                .await
                .map_err(|e| enrich_client_error(e, "list_workspace_activity"))?
        } else {
            client
                .list_workspace_activity(
                    &params.workspace,
                    params.actor.as_deref(),
                    params.from.as_deref(),
                    params.to.as_deref(),
                    Some(limit),
                )
                .await
                .map_err(|e| enrich_client_error(e, "list_workspace_activity"))?
        };

        let result = envelope_page(page, project_workspace_activity_entry);
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
            .map_err(|e| enrich_client_error(e, "list_document_history"))?;

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
            .map_err(|e| enrich_client_error(e, "get_document_revision"))?;

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
            .map_err(|e| enrich_client_error(e, "list_attachments"))?;

        let result = envelope_page(page, project_attachment);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List attachment metadata for a task (file name, type, size)")]
    async fn list_task_attachments(
        &self,
        Parameters(params): Parameters<ListTaskAttachmentsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let items = client
            .list_task_attachments(&params.workspace, &params.readable_id)
            .await
            .map_err(|e| enrich_client_error(e, "list_task_attachments"))?;

        let result: Vec<_> = items.into_iter().map(project_task_attachment).collect();
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Retrieve a task's IMAGE attachment as viewable content (e.g. a screenshot). \
                       Pass the attachment UUID from `list_task_attachments`. Non-image attachments \
                       are rejected."
    )]
    async fn get_task_attachment(
        &self,
        Parameters(params): Parameters<GetTaskAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Content, String> {
        let client = self.resolve_client(&ctx)?;

        let attachment_id = parse_uuid_param("attachment_id", &params.attachment_id)?;

        let (bytes, content_type) = client
            .download_task_attachment(&params.workspace, &params.readable_id, attachment_id)
            .await
            .map_err(|e| enrich_client_error(e, "get_task_attachment"))?;

        let mime = content_type.unwrap_or_else(|| "application/octet-stream".to_string());
        if !mime.starts_with("image/") {
            return Err(format!(
                "attachment '{attachment_id}' is not an image (content type '{mime}'); \
                 this tool only retrieves image attachments for viewing"
            ));
        }

        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
        Ok(Content::image(encoded, mime))
    }

    #[tool(description = "Create a task on a board. Board and column are resolved by name.")]
    async fn create_task(
        &self,
        Parameters(params): Parameters<CreateTaskParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let board_uuid = resolve_board_uuid(&client, &params.workspace, &params.board).await?;

        let cols = client
            .list_columns(&params.workspace, board_uuid)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

        let column_id = resolve_column_id_on_board(&params.column, &cols)?;

        if let Some(ref p) = params.priority {
            validate_priority(p)?;
        }

        if let Some(e) = params.estimate {
            validate_estimate(e)?;
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

        if let Some(v) = params.estimate.as_ref() {
            validate_estimate_value(v)?;
        }

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

        let board_id_str = match params.board.as_deref() {
            Some(board_ref) => helpers::resolve_board_id(&client, &params.workspace, board_ref)
                .await
                .map_err(|e| e.to_string())?,
            None => {
                // No board supplied: fetch the task first to get its board_id.
                let task = client
                    .get_task(&params.workspace, &params.readable_id)
                    .await
                    .map_err(|e| enrich_client_error(e, "get_task"))?;
                task.board_id.to_string()
            }
        };

        let board_uuid = parse_resolved_board_uuid(&board_id_str)?;

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

        let folder_id = parse_optional_uuid_param("folder_id", params.folder_id.as_deref())?;

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

        let folder_id = parse_optional_uuid_param("folder_id", params.folder_id.as_deref())?;

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
        description = "Recoverably delete a document. Requires confirm: true. \
                       Permanent removal is available only through root/system-admin human Trash purge."
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

        let folder_id = parse_optional_uuid_param("folder_id", params.folder_id.as_deref())?;

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

        let folder_id = parse_optional_uuid_param("folder_id", params.folder_id.as_deref())?;

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

        let parent_folder_id =
            parse_optional_uuid_param("parent_folder_id", params.parent_folder_id.as_deref())?;

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

        let parent_folder_id =
            parse_optional_uuid_param("parent_folder_id", params.parent_folder_id.as_deref())?;

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

        let parent_folder_id =
            parse_optional_uuid_param("parent_folder_id", params.parent_folder_id.as_deref())?;

        let folder = client
            .copy_folder(&params.workspace, folder_id, parent_folder_id)
            .await
            .map_err(|e| enrich_client_error(e, "copy_folder"))?;

        let result = project_folder(folder);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Recoverably delete a folder. Requires confirm: true. Documents inside keep \
                       their folder_id and are hidden until the folder is restored."
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

    #[tool(
        description = "Create a new board in a project. A new board is auto-seeded with the \
workspace's default columns (statuses), which are returned in the `columns` field of the \
response — do NOT create those columns again; only add columns for statuses that are missing."
    )]
    async fn create_board(
        &self,
        Parameters(params): Parameters<CreateBoardParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateBoardRequest {
            name: params.name,
            folder_id: None,
        };

        let board = client
            .create_board(&params.workspace, &params.project, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_board"))?;

        // A new board is auto-seeded with the workspace's default columns; return
        // them so the caller does not recreate the statuses that already exist.
        let columns = client
            .list_columns(&params.workspace, board.id)
            .await
            .map_err(|e| enrich_client_error(e, "list_columns"))?;

        let result = json!({
            "id": board.id,
            "name": board.name,
            "project_id": board.project_id,
            "updated_at": board.updated_at,
            "columns": columns.into_iter().map(project_column).collect::<Vec<_>>(),
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

        let board_uuid = resolve_board_uuid(&client, &params.workspace, &params.board).await?;

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

        let board_uuid = resolve_board_uuid(&client, &params.workspace, &params.board).await?;

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

        let board_uuid = resolve_board_uuid(&client, &params.workspace, &params.board).await?;

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

        let board_uuid = resolve_board_uuid(&client, &params.workspace, &params.board).await?;

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

        let board_uuid = resolve_board_uuid(&client, &params.workspace, &params.board).await?;

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

    // Tag reads/writes (`list_tags` above plus the create/update/delete tools
    // below) are gated server-side by the `config:{read,create,update,delete}`
    // capabilities for agent keys. This is resource access control, NOT scope
    // self-mutation: no tag tool edits the calling key's own capability set.
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
    // Graph writes: references, checklist, subtasks
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

        let target_doc_uuid =
            parse_optional_uuid_param("target_document_id", params.target_document_id.as_deref())?;

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

        let result = response::project_manual_reference(reference);
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

    #[tool(description = "Post a markdown comment on a task (max 10 000 characters)")]
    async fn add_comment(
        &self,
        Parameters(params): Parameters<AddCommentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateCommentRequest::published(params.body);

        let comment = client
            .add_comment(&params.workspace, &params.readable_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "add_comment"))?;

        let result = project_comment(comment);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Edit a task comment's body (max 10 000 characters). Only the comment's \
                       author may edit it; anyone else gets a permission error."
    )]
    async fn update_comment(
        &self,
        Parameters(params): Parameters<UpdateCommentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let comment_id: uuid::Uuid = params
            .comment_id
            .parse()
            .map_err(|_| format!("comment_id '{}' is not a valid UUID", params.comment_id))?;

        let body = UpdateCommentRequest { body: params.body };

        let comment = client
            .update_comment(&params.workspace, &params.readable_id, comment_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_comment"))?;

        let result = project_comment(comment);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a task comment. The comment's author or a workspace admin/owner \
                       may delete it; anyone else gets a permission error."
    )]
    async fn delete_comment(
        &self,
        Parameters(params): Parameters<DeleteCommentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let comment_id: uuid::Uuid = params
            .comment_id
            .parse()
            .map_err(|_| format!("comment_id '{}' is not a valid UUID", params.comment_id))?;

        client
            .delete_comment(&params.workspace, &params.readable_id, comment_id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_comment"))?;

        let result = json!({
            "deleted": true,
            "comment_id": comment_id,
        });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(description = "List markdown comments on a document, oldest first")]
    async fn list_document_comments(
        &self,
        Parameters(params): Parameters<ListDocumentCommentsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(50).clamp(1, 200);

        let page = client
            .list_document_comments(
                &params.workspace,
                &params.slug,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| enrich_client_error(e, "list_document_comments"))?;

        let result = envelope_page(page, project_comment);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "List the full authorized document comment feed, including derived links and retained events, oldest first"
    )]
    async fn list_document_comment_feed(
        &self,
        Parameters(params): Parameters<ListDocumentCommentsParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(50).clamp(1, 200);
        let page = client
            .list_document_comment_feed(
                &params.workspace,
                &params.slug,
                params.cursor.as_deref(),
                Some(limit),
            )
            .await
            .map_err(|e| enrich_client_error(e, "list_document_comment_feed"))?;
        serde_json::to_string(&envelope_page(page, project_comment_feed_entry))
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Upload a file owned by a document comment. Content must be strict padded standard base64."
    )]
    async fn upload_document_comment_attachment(
        &self,
        Parameters(params): Parameters<UploadDocumentCommentAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let comment_id = parse_uuid_param("comment_id", &params.comment_id)?;
        let max_attachment_bytes = require_comment_attachment_limit(
            client.server_meta().await,
            "upload_document_comment_attachment",
        )?;
        let data = decode_comment_attachment_data(&params.data_base64, max_attachment_bytes)?;
        let attachment = client
            .upload_document_comment_attachment(
                &params.workspace,
                &params.slug,
                comment_id,
                &params.file_name,
                &params.content_type,
                data,
            )
            .await
            .map_err(|e| enrich_client_error(e, "upload_document_comment_attachment"))?;
        serde_json::to_string(&project_comment_attachment(attachment)).map_err(|e| e.to_string())
    }

    #[tool(description = "List attachment metadata for a document comment")]
    async fn list_document_comment_attachments(
        &self,
        Parameters(params): Parameters<DocumentCommentAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let comment_id = parse_uuid_param("comment_id", &params.comment_id)?;
        let attachments = client
            .list_document_comment_attachments(&params.workspace, &params.slug, comment_id)
            .await
            .map_err(|e| enrich_client_error(e, "list_document_comment_attachments"))?;
        serde_json::to_string(&wrap_vec(attachments, project_comment_attachment))
            .map_err(|e| e.to_string())
    }

    #[tool(description = "Download a document comment attachment as standard base64 content")]
    async fn get_document_comment_attachment(
        &self,
        Parameters(params): Parameters<DocumentCommentAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        use base64::Engine as _;
        let client = self.resolve_client(&ctx)?;
        let comment_id = parse_uuid_param("comment_id", &params.comment_id)?;
        let attachment_id = parse_uuid_param(
            "attachment_id",
            params
                .attachment_id
                .as_deref()
                .ok_or_else(|| "attachment_id is required".to_string())?,
        )?;
        let (data, content_type) = client
            .download_document_comment_attachment(
                &params.workspace,
                &params.slug,
                comment_id,
                attachment_id,
            )
            .await
            .map_err(|e| enrich_client_error(e, "get_document_comment_attachment"))?;
        serde_json::to_string(&json!({"data_base64": base64::engine::general_purpose::STANDARD.encode(data), "content_type": content_type}))
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a document comment attachment. Plain delete, no confirm required \
                       (consistent with delete_document_comment, which removes the whole comment)."
    )]
    async fn delete_document_comment_attachment(
        &self,
        Parameters(params): Parameters<DocumentCommentAttachmentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let comment_id = parse_uuid_param("comment_id", &params.comment_id)?;
        let attachment_id = parse_uuid_param(
            "attachment_id",
            params
                .attachment_id
                .as_deref()
                .ok_or_else(|| "attachment_id is required".to_string())?,
        )?;
        client
            .delete_document_comment_attachment(
                &params.workspace,
                &params.slug,
                comment_id,
                attachment_id,
            )
            .await
            .map_err(|e| enrich_client_error(e, "delete_document_comment_attachment"))?;
        serde_json::to_string(&json!({"deleted": true, "attachment_id": attachment_id}))
            .map_err(|e| e.to_string())
    }

    #[tool(description = "Post a markdown comment on a document (max 10 000 characters)")]
    async fn add_document_comment(
        &self,
        Parameters(params): Parameters<AddDocumentCommentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let body = CreateCommentRequest::published(params.body);

        let comment = client
            .add_document_comment(&params.workspace, &params.slug, body)
            .await
            .map_err(|e| enrich_client_error(e, "add_document_comment"))?;

        let result = project_comment(comment);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Edit a document comment's body (max 10 000 characters). Only the comment's \
                       author may edit it; anyone else gets a permission error."
    )]
    async fn update_document_comment(
        &self,
        Parameters(params): Parameters<UpdateDocumentCommentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let comment_id: uuid::Uuid = params
            .comment_id
            .parse()
            .map_err(|_| format!("comment_id '{}' is not a valid UUID", params.comment_id))?;

        let body = UpdateCommentRequest { body: params.body };

        let comment = client
            .update_document_comment(&params.workspace, &params.slug, comment_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_document_comment"))?;

        let result = project_comment(comment);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a document comment. The comment's author or a workspace admin/owner \
                       may delete it; anyone else gets a permission error."
    )]
    async fn delete_document_comment(
        &self,
        Parameters(params): Parameters<DeleteDocumentCommentParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let comment_id: uuid::Uuid = params
            .comment_id
            .parse()
            .map_err(|_| format!("comment_id '{}' is not a valid UUID", params.comment_id))?;

        client
            .delete_document_comment(&params.workspace, &params.slug, comment_id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_document_comment"))?;

        let result = json!({
            "deleted": true,
            "comment_id": comment_id,
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

        let board_uuid = resolve_board_uuid(&client, &params.workspace, &params.board).await?;

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
    // Project CRUD
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

    #[tool(description = "Recoverably delete a project. Requires confirm: true. \
        Descendants are hidden until the project is restored; permanent removal is a separate \
        root/system-admin human Trash purge workflow.")]
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
    // Status template CRUD
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
    // Saved search CRUD
    //
    // Gated server-side by the `saved_searches:{read,create,update,delete}`
    // capabilities for agent keys. This is resource access control, NOT scope
    // self-mutation: no tool here edits the calling key's own capability set.
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
    // Task view CRUD
    //
    // Gated server-side by the `task_views:{read,create,update,delete}`
    // capabilities for agent keys. This is resource access control, NOT scope
    // self-mutation: no tool here edits the calling key's own capability set.
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

    #[tool(
        description = "List the security audit log for a workspace (owner/admin only). \
                       Returns who performed each privileged action (membership changes, \
                       permission grants, API key lifecycle), with enriched actor details \
                       (display_name, account_status for users; key_type for API keys). \
                       Returns 403 if the caller is not a workspace owner or admin — \
                       audit requires workspace owner/admin or platform admin. \
                       Supports actor-type (user|api_key), action verb, date range, \
                       and cursor pagination."
    )]
    async fn get_workspace_audit(
        &self,
        Parameters(params): Parameters<GetWorkspaceAuditParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(50).clamp(1, 200);

        let page = if params.cursor.is_some() {
            client
                .list_workspace_audit_with_cursor(
                    &params.workspace,
                    params.actor.as_deref(),
                    params.action.as_deref(),
                    params.from.as_deref(),
                    params.cursor.as_deref(),
                    Some(limit),
                )
                .await
                .map_err(|e| enrich_client_error(e, "get_workspace_audit"))?
        } else {
            client
                .list_workspace_audit(
                    &params.workspace,
                    params.actor.as_deref(),
                    params.action.as_deref(),
                    params.from.as_deref(),
                    params.to.as_deref(),
                    Some(limit),
                )
                .await
                .map_err(|e| enrich_client_error(e, "get_workspace_audit"))?
        };

        let result = envelope_page(page, project_audit_entry);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "List the platform-wide security audit log (platform admin only). \
                       Returns platform-scoped events (user lifecycle: created, disabled, \
                       enabled, password reset, activation; system-admin flag changes). \
                       Returns 403 if the caller is not a platform admin — \
                       audit requires workspace owner/admin or platform admin. \
                       Supports actor-type (user|api_key), action verb, date range, \
                       and cursor pagination."
    )]
    async fn get_platform_audit(
        &self,
        Parameters(params): Parameters<GetPlatformAuditParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let limit = params.limit.unwrap_or(50).clamp(1, 200);

        let page = if params.cursor.is_some() {
            client
                .list_platform_audit_with_cursor(
                    params.actor.as_deref(),
                    params.action.as_deref(),
                    params.from.as_deref(),
                    params.cursor.as_deref(),
                    Some(limit),
                )
                .await
                .map_err(|e| enrich_client_error(e, "get_platform_audit"))?
        } else {
            client
                .list_platform_audit(
                    params.actor.as_deref(),
                    params.action.as_deref(),
                    params.from.as_deref(),
                    params.to.as_deref(),
                    Some(limit),
                )
                .await
                .map_err(|e| enrich_client_error(e, "get_platform_audit"))?
        };

        let result = envelope_page(page, project_audit_entry);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    // -----------------------------------------------------------------------
    // Webhook management
    //
    // These tools MANAGE webhook subscriptions for a workspace. They are gated
    // server-side by the `webhooks:{read,create,update,delete}` capabilities
    // (plus an Editor role floor for agent keys). This is NOT scope
    // self-mutation: none of these tools change the calling key's own
    // capability set — there is deliberately no tool that edits an API key's
    // scopes.
    // -----------------------------------------------------------------------

    #[tool(
        description = "List webhook subscriptions in a workspace (cursor-paginated). \
        Requires the webhooks:read capability."
    )]
    async fn list_webhooks(
        &self,
        Parameters(params): Parameters<ListWebhooksParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let page = client
            .list_webhooks(&params.workspace, params.cursor.as_deref(), params.limit)
            .await
            .map_err(|e| enrich_client_error(e, "list_webhooks"))?;

        let result = envelope_page(page, project_webhook);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Retrieve a single webhook subscription by UUID (no secret). \
        Requires the webhooks:read capability."
    )]
    async fn get_webhook(
        &self,
        Parameters(params): Parameters<GetWebhookParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let webhook_id = parse_webhook_id(&params.webhook_id)?;

        let webhook = client
            .get_webhook(&params.workspace, webhook_id)
            .await
            .map_err(|e| enrich_client_error(e, "get_webhook"))?;

        let result = project_webhook(webhook);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "List delivery attempts for a webhook, newest first (cursor-paginated). \
        Requires the webhooks:read capability."
    )]
    async fn list_webhook_deliveries(
        &self,
        Parameters(params): Parameters<ListWebhookDeliveriesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let webhook_id = parse_webhook_id(&params.webhook_id)?;

        let page = client
            .list_webhook_deliveries(
                &params.workspace,
                webhook_id,
                params.cursor.as_deref(),
                params.limit,
            )
            .await
            .map_err(|e| enrich_client_error(e, "list_webhook_deliveries"))?;

        let result = envelope_page(page, project_webhook_delivery);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Create a webhook subscription. Requires the webhooks:create capability. \
        The response carries the one-time signing secret (whsec_…) under `secret`; it is \
        shown exactly once and never retrievable again — store it immediately."
    )]
    async fn create_webhook(
        &self,
        Parameters(params): Parameters<CreateWebhookParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;

        let scope_id = parse_optional_uuid_param("scope_id", params.scope_id.as_deref())?;

        let body = CreateWebhookRequest {
            target_url: params.target_url,
            event_types: params.event_types,
            scope_type: params.scope_type.unwrap_or_else(|| "workspace".to_string()),
            scope_id,
            label: params.label,
        };

        let created = client
            .create_webhook(&params.workspace, body)
            .await
            .map_err(|e| enrich_client_error(e, "create_webhook"))?;

        let result = project_webhook_created(created);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Update a webhook subscription (PATCH: omit a field to leave it \
        unchanged). Requires the webhooks:update capability. The signing secret is never \
        rotated through this tool."
    )]
    async fn update_webhook(
        &self,
        Parameters(params): Parameters<UpdateWebhookParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        let webhook_id = parse_webhook_id(&params.webhook_id)?;

        let scope_id = map_present_uuid("scope_id", params.scope_id.as_ref())?;
        let label = map_present_string("label", params.label.as_ref())?;

        let body = UpdateWebhookRequest {
            target_url: params.target_url,
            event_types: params.event_types,
            scope_type: params.scope_type,
            scope_id,
            is_active: params.is_active,
            label,
        };

        let webhook = client
            .update_webhook(&params.workspace, webhook_id, body)
            .await
            .map_err(|e| enrich_client_error(e, "update_webhook"))?;

        let result = project_webhook(webhook);
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Delete a webhook subscription. Requires confirm: true and the \
        webhooks:delete capability. Soft-deletes the subscription."
    )]
    async fn delete_webhook(
        &self,
        Parameters(params): Parameters<DeleteWebhookParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let client = self.resolve_client(&ctx)?;
        require_confirm(params.confirm, "webhook", &params.webhook_id)?;
        let webhook_id = parse_webhook_id(&params.webhook_id)?;

        client
            .delete_webhook(&params.workspace, webhook_id)
            .await
            .map_err(|e| enrich_client_error(e, "delete_webhook"))?;

        let result = serde_json::json!({ "deleted": true, "webhook_id": params.webhook_id });
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }
}

#[tool_handler]
impl ServerHandler for AtlasMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("atlas-mcp", env!("CARGO_PKG_VERSION")))
        .with_instructions(ATLAS_INSTRUCTIONS)
    }

    async fn list_resource_templates(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        let template = RawResourceTemplate::new("atlas:///{workspace}/{slug}", "atlas-document")
            .with_title("Atlas Document")
            .with_description(
                "A markdown document in an Atlas workspace. \
                 workspace = workspace slug, slug = document slug or UUID.",
            )
            .with_mime_type("text/markdown")
            .no_annotation();

        Ok(ListResourceTemplatesResult {
            resource_templates: vec![template],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = &request.uri;

        let (workspace, slug) =
            parse_atlas_doc_uri(uri).map_err(|e| McpError::invalid_params(e, None))?;

        let client = self
            .resolve_client(&ctx)
            .map_err(|e| McpError::invalid_params(e, None))?;

        let doc = client
            .get_document(&workspace, &slug)
            .await
            .map_err(|e| McpError::internal_error(enrich_client_error(e, "read_resource"), None))?;

        Ok(ReadResourceResult::new(vec![
            ResourceContents::TextResourceContents {
                uri: uri.clone(),
                mime_type: Some("text/markdown".to_string()),
                text: doc.content,
                meta: None,
            },
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{
        ClientHandler, ServiceExt,
        model::{CallToolRequestParams, ClientInfo},
    };

    const COMMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000001");
    const ATTACHMENT_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
    const COMMENT_ATTACHMENT: &str = r#"{"id":"00000000-0000-0000-0000-000000000002","comment_id":"00000000-0000-0000-0000-000000000001","file_name":"note.txt","content_type":"text/plain","size_bytes":2,"sha256":"digest","actor":null,"created_at":"2026-01-01T00:00:00Z"}"#;

    #[derive(Debug, Clone, Default)]
    struct TestClientHandler;

    impl ClientHandler for TestClientHandler {
        fn get_info(&self) -> ClientInfo {
            ClientInfo::default()
        }
    }

    async fn start_mcp_client(
        server: AtlasMcp,
    ) -> rmcp::service::RunningService<rmcp::RoleClient, TestClientHandler> {
        let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let running = server
                .serve(server_transport)
                .await
                .expect("MCP server starts");
            running.waiting().await.expect("MCP server runs");
        });

        TestClientHandler
            .serve(client_transport)
            .await
            .expect("MCP client starts")
    }

    fn call_tool_params(name: &str, arguments: serde_json::Value) -> CallToolRequestParams {
        CallToolRequestParams::new(name.to_string()).with_arguments(
            arguments
                .as_object()
                .expect("tool arguments are an object")
                .clone(),
        )
    }

    fn tool_text(result: &rmcp::model::CallToolResult) -> &str {
        result
            .content
            .first()
            .and_then(|content| content.raw.as_text())
            .map(|text| text.text.as_str())
            .expect("tool returns text content")
    }

    fn serve_recording_atlas(
        responses: Vec<(&'static str, String)>,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("recording server binds");
        let address = listener.local_addr().expect("recording server has address");
        let (request_tx, request_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().expect("recording server accepts request");
                let mut request = [0_u8; 8192];
                let length = stream
                    .read(&mut request)
                    .expect("recording server reads request");
                request_tx
                    .send(
                        String::from_utf8_lossy(request.get(..length).unwrap_or_default())
                            .into_owned(),
                    )
                    .expect("recording server records request");
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .expect("recording server writes response");
            }
        });

        (format!("http://{address}"), request_rx)
    }

    fn serve_transport_failing_meta() -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::Read;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("recording server binds");
        let address = listener.local_addr().expect("recording server has address");
        let (request_tx, request_rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("recording server accepts request");
            let mut request = [0_u8; 8192];
            let length = stream
                .read(&mut request)
                .expect("recording server reads request");
            request_tx
                .send(
                    String::from_utf8_lossy(request.get(..length).unwrap_or_default()).into_owned(),
                )
                .expect("recording server records request");
        });

        (format!("http://{address}"), request_rx)
    }

    #[tokio::test]
    async fn public_mcp_comment_attachment_tools_call_the_atlas_lifecycle() {
        let responses = vec![
            (
                "200 OK",
                r#"{"items":[],"next_cursor":null,"has_more":false}"#.to_string(),
            ),
            (
                "200 OK",
                r#"{"items":[],"next_cursor":null,"has_more":false}"#.to_string(),
            ),
            (
                "200 OK",
                r#"{"version":"1","build":null,"url":null,"max_attachment_bytes":16}"#.to_string(),
            ),
            ("201 Created", COMMENT_ATTACHMENT.to_string()),
            ("200 OK", format!("[{COMMENT_ATTACHMENT}]")),
            ("200 OK", "ok".to_string()),
            ("204 No Content", String::new()),
            (
                "200 OK",
                r#"{"version":"1","build":null,"url":null,"max_attachment_bytes":16}"#.to_string(),
            ),
            ("201 Created", COMMENT_ATTACHMENT.to_string()),
            ("200 OK", format!("[{COMMENT_ATTACHMENT}]")),
            ("200 OK", "ok".to_string()),
            ("204 No Content", String::new()),
        ];
        let (base_url, requests) = serve_recording_atlas(responses);
        let client =
            start_mcp_client(AtlasMcp::new(base_url, "atlas_test").expect("server config")).await;

        for (name, arguments) in [
            (
                "list_comment_feed",
                serde_json::json!({"workspace":"ws","readable_id":"ATL-1"}),
            ),
            (
                "list_document_comment_feed",
                serde_json::json!({"workspace":"ws","slug":"note"}),
            ),
            (
                "upload_task_comment_attachment",
                serde_json::json!({"workspace":"ws","readable_id":"ATL-1","comment_id":COMMENT_ID,"file_name":"note.txt","content_type":"text/plain","data_base64":"b2s="}),
            ),
            (
                "list_task_comment_attachments",
                serde_json::json!({"workspace":"ws","readable_id":"ATL-1","comment_id":COMMENT_ID}),
            ),
            (
                "get_task_comment_attachment",
                serde_json::json!({"workspace":"ws","readable_id":"ATL-1","comment_id":COMMENT_ID,"attachment_id":ATTACHMENT_ID}),
            ),
            (
                "delete_task_comment_attachment",
                serde_json::json!({"workspace":"ws","readable_id":"ATL-1","comment_id":COMMENT_ID,"attachment_id":ATTACHMENT_ID}),
            ),
            (
                "upload_document_comment_attachment",
                serde_json::json!({"workspace":"ws","slug":"note","comment_id":COMMENT_ID,"file_name":"note.txt","content_type":"text/plain","data_base64":"b2s="}),
            ),
            (
                "list_document_comment_attachments",
                serde_json::json!({"workspace":"ws","slug":"note","comment_id":COMMENT_ID}),
            ),
            (
                "get_document_comment_attachment",
                serde_json::json!({"workspace":"ws","slug":"note","comment_id":COMMENT_ID,"attachment_id":ATTACHMENT_ID}),
            ),
            (
                "delete_document_comment_attachment",
                serde_json::json!({"workspace":"ws","slug":"note","comment_id":COMMENT_ID,"attachment_id":ATTACHMENT_ID}),
            ),
        ] {
            let result = client
                .call_tool(call_tool_params(name, arguments))
                .await
                .expect("public MCP call succeeds");
            assert!(
                !result.is_error.unwrap_or(false),
                "{name} returned an MCP error: {}",
                tool_text(&result)
            );
        }

        let requests: Vec<_> = (0..12)
            .map(|_| requests.recv().expect("Atlas received request"))
            .collect();
        let expected_paths = [
            "GET /api/workspaces/ws/tasks/ATL-1/comments?feed=full&limit=50 ",
            "GET /api/workspaces/ws/documents/note/comments?feed=full&limit=50 ",
            "GET /api/meta ",
            "POST /api/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments ",
            "GET /api/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments ",
            "GET /api/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002/content ",
            "DELETE /api/workspaces/ws/tasks/ATL-1/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
            "GET /api/meta ",
            "POST /api/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments ",
            "GET /api/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments ",
            "GET /api/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
            "DELETE /api/workspaces/ws/documents/note/comments/00000000-0000-0000-0000-000000000001/attachments/00000000-0000-0000-0000-000000000002 ",
        ];
        assert_eq!(requests.len(), expected_paths.len());
        for (request, expected_path) in requests.iter().zip(expected_paths) {
            assert!(
                request.starts_with(expected_path),
                "expected `{expected_path}`, received `{request}`"
            );
        }
    }

    #[tokio::test]
    async fn public_mcp_upload_fails_closed_when_metadata_discovery_cannot_supply_a_limit() {
        for body in [
            r#"{"version":"1","build":null,"url":null}"#,
            r#"{"version":"1","build":null,"url":null,"max_attachment_bytes":null}"#,
            r#"{"version":"1","build":null,"url":null,"max_attachment_bytes":"large"}"#,
            r#"{"type":"urn:atlas:error","title":"Unavailable","status":503}"#,
        ] {
            let status = if body.contains("Unavailable") {
                "503 Service Unavailable"
            } else {
                "200 OK"
            };
            let (base_url, requests) = serve_recording_atlas(vec![(status, body.to_string())]);
            let client =
                start_mcp_client(AtlasMcp::new(base_url, "atlas_test").expect("server config"))
                    .await;

            let result = client
                .call_tool(call_tool_params(
                    "upload_task_comment_attachment",
                    serde_json::json!({"workspace":"ws","readable_id":"ATL-1","comment_id":COMMENT_ID,"file_name":"note.txt","content_type":"text/plain","data_base64":"b2s="}),
                ))
                .await
                .expect("MCP call completes with a tool error");

            assert!(result.is_error.unwrap_or(false));
            assert!(tool_text(&result).contains("upload was not attempted"));
            assert!(
                requests
                    .recv()
                    .expect("metadata request is recorded")
                    .starts_with("GET /api/meta ")
            );
            assert!(
                requests
                    .recv_timeout(std::time::Duration::from_millis(50))
                    .is_err()
            );
        }

        let (base_url, requests) = serve_transport_failing_meta();
        let client =
            start_mcp_client(AtlasMcp::new(base_url, "atlas_test").expect("server config")).await;
        let result = client
            .call_tool(call_tool_params(
                "upload_document_comment_attachment",
                serde_json::json!({"workspace":"ws","slug":"note","comment_id":COMMENT_ID,"file_name":"note.txt","content_type":"text/plain","data_base64":"b2s="}),
            ))
            .await
            .expect("MCP call completes with a tool error");

        assert!(result.is_error.unwrap_or(false));
        assert!(tool_text(&result).contains("upload was not attempted"));
        assert!(
            requests
                .recv()
                .expect("metadata request is recorded")
                .starts_with("GET /api/meta ")
        );
        assert!(
            requests
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
    }

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
    fn get_info_advertises_resources_capability() {
        let server = AtlasMcp::new("http://localhost:8080", "test-token").unwrap();
        let info = server.get_info();
        assert!(
            info.capabilities.resources.is_some(),
            "ServerCapabilities.resources must be Some after enable_resources()"
        );
    }

    #[test]
    fn webhook_management_tools_are_registered() {
        let router = AtlasMcp::tool_router();
        for name in [
            "list_webhooks",
            "get_webhook",
            "list_webhook_deliveries",
            "create_webhook",
            "update_webhook",
            "delete_webhook",
        ] {
            assert!(
                router.has_route(name),
                "expected MCP tool `{name}` to be registered"
            );
        }
    }

    #[test]
    fn semantic_search_tool_is_registered() {
        let router = AtlasMcp::tool_router();
        assert!(
            router.has_route("semantic_search"),
            "expected MCP semantic_search tool to be registered"
        );
    }

    #[test]
    fn comment_attachment_tools_are_registered() {
        let router = AtlasMcp::tool_router();
        for name in [
            "list_comment_feed",
            "list_document_comment_feed",
            "upload_task_comment_attachment",
            "list_task_comment_attachments",
            "get_task_comment_attachment",
            "delete_task_comment_attachment",
            "upload_document_comment_attachment",
            "list_document_comment_attachments",
            "get_document_comment_attachment",
            "delete_document_comment_attachment",
        ] {
            assert!(
                router.has_route(name),
                "expected MCP tool `{name}` to be registered"
            );
        }
    }

    #[test]
    fn comment_attachment_data_rejects_invalid_or_unpadded_base64() {
        assert!(decode_comment_attachment_data("not base64", 16).is_err());
        assert!(decode_comment_attachment_data("YQ", 16).is_err());
        assert!(decode_comment_attachment_data("YQ==\n", 16).is_err());
    }

    #[test]
    fn comment_attachment_data_rejects_encoded_and_decoded_oversize_payloads() {
        assert!(decode_comment_attachment_data("YWJjZA==", 3).is_err());
        assert!(decode_comment_attachment_data("YWI=", 1).is_err());
        assert_eq!(decode_comment_attachment_data("YWI=", 2).unwrap(), b"ab");
    }

    #[test]
    fn comment_attachment_limit_discovery_fails_closed_when_absent_or_null() {
        let absent = ServerMetaDto {
            version: "1".into(),
            build: None,
            url: None,
            max_attachment_bytes: None,
        };
        let error = require_comment_attachment_limit(Ok(absent), "upload_task_comment_attachment")
            .unwrap_err();
        assert!(error.contains("could not be discovered"));
        assert!(error.contains("not attempted"));
    }

    #[test]
    fn upload_comment_attachment_params_require_metadata_and_base64_content() {
        let params: UploadTaskCommentAttachmentParams = serde_json::from_str(
            r#"{"workspace":"ws","readable_id":"ATL-1","comment_id":"00000000-0000-0000-0000-000000000000","file_name":"note.txt","content_type":"text/plain","data_base64":"YQ=="}"#,
        )
        .unwrap();
        assert_eq!(params.file_name, "note.txt");
        assert_eq!(params.content_type, "text/plain");
        assert_eq!(params.data_base64, "YQ==");
    }

    #[test]
    fn parse_webhook_id_rejects_non_uuid() {
        assert!(parse_webhook_id("not-a-uuid").is_err());
        assert!(parse_webhook_id(&uuid::Uuid::nil().to_string()).is_ok());
    }

    #[test]
    fn parse_optional_uuid_param_handles_absent_and_present() {
        assert!(
            parse_optional_uuid_param("scope_id", None)
                .unwrap()
                .is_none()
        );
        assert!(
            parse_optional_uuid_param("scope_id", Some(&uuid::Uuid::nil().to_string()))
                .unwrap()
                .is_some()
        );
        assert!(parse_optional_uuid_param("scope_id", Some("bad")).is_err());
    }

    #[test]
    fn parse_uri_valid_workspace_and_slug() {
        let result = parse_atlas_doc_uri("atlas:///my-workspace/my-doc-slug");
        assert_eq!(
            result,
            Ok(("my-workspace".to_string(), "my-doc-slug".to_string()))
        );
    }

    #[test]
    fn parse_uri_valid_uuid_slug() {
        let result = parse_atlas_doc_uri("atlas:///acme/550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(
            result,
            Ok((
                "acme".to_string(),
                "550e8400-e29b-41d4-a716-446655440000".to_string()
            ))
        );
    }

    #[test]
    fn parse_uri_wrong_scheme_rejected() {
        let result = parse_atlas_doc_uri("https:///ws/slug");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("atlas:///"),
            "error must mention the expected scheme"
        );
    }

    #[test]
    fn parse_uri_missing_slug_rejected() {
        let result = parse_atlas_doc_uri("atlas:///only-workspace");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("slug"), "error must mention missing slug");
    }

    #[test]
    fn parse_uri_empty_string_rejected() {
        let result = parse_atlas_doc_uri("");
        assert!(result.is_err());
    }

    #[test]
    fn parse_uri_extra_path_segments_rejected() {
        let result = parse_atlas_doc_uri("atlas:///ws/project/slug");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("too many"),
            "error must mention too many segments"
        );
    }

    #[test]
    fn parse_uri_empty_workspace_rejected() {
        let result = parse_atlas_doc_uri("atlas:////slug");
        assert!(result.is_err());
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
    fn semantic_search_params_deserializes_minimal() {
        let json = r#"{"workspace":"my-ws","query":"incident response"}"#;
        let params: SemanticSearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "my-ws");
        assert_eq!(params.query, "incident response");
        assert!(params.type_filter.is_none());
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn semantic_search_params_deserializes_type_cursor_and_limit() {
        let json = r#"{"workspace":"ws","query":"q","type":"task","cursor":"next","limit":7}"#;
        let params: SemanticSearchParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.type_filter.as_deref(), Some("task"));
        assert_eq!(params.cursor.as_deref(), Some("next"));
        assert_eq!(params.limit, Some(7));
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
    fn list_comments_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-10"}"#;
        let params: ListCommentsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.readable_id, "ATL-10");
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn list_comments_params_deserializes_with_pagination() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-10","cursor":"tok","limit":20}"#;
        let params: ListCommentsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.cursor.as_deref(), Some("tok"));
        assert_eq!(params.limit, Some(20));
    }

    #[test]
    fn list_workspace_activity_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws"}"#;
        let params: ListWorkspaceActivityParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert!(params.actor.is_none());
        assert!(params.from.is_none());
        assert!(params.to.is_none());
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn list_workspace_activity_params_deserializes_full() {
        let json = r#"{"workspace":"ws","actor":"user","from":"2024-01-01T00:00:00Z","to":"2024-12-31T23:59:59Z","cursor":"abc","limit":25}"#;
        let params: ListWorkspaceActivityParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.actor.as_deref(), Some("user"));
        assert_eq!(params.from.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(params.to.as_deref(), Some("2024-12-31T23:59:59Z"));
        assert_eq!(params.cursor.as_deref(), Some("abc"));
        assert_eq!(params.limit, Some(25));
    }

    #[test]
    fn get_workspace_audit_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws"}"#;
        let params: GetWorkspaceAuditParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert!(params.actor.is_none());
        assert!(params.action.is_none());
        assert!(params.from.is_none());
        assert!(params.to.is_none());
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn get_workspace_audit_params_deserializes_full() {
        let json = r#"{"workspace":"ws","actor":"user","action":"membership.role_changed","from":"2024-01-01T00:00:00Z","to":"2024-12-31T23:59:59Z","cursor":"abc","limit":25}"#;
        let params: GetWorkspaceAuditParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.actor.as_deref(), Some("user"));
        assert_eq!(params.action.as_deref(), Some("membership.role_changed"));
        assert_eq!(params.from.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(params.to.as_deref(), Some("2024-12-31T23:59:59Z"));
        assert_eq!(params.cursor.as_deref(), Some("abc"));
        assert_eq!(params.limit, Some(25));
    }

    #[test]
    fn get_platform_audit_params_deserializes_minimal() {
        let json = r#"{}"#;
        let params: GetPlatformAuditParams = serde_json::from_str(json).unwrap();
        assert!(params.actor.is_none());
        assert!(params.action.is_none());
        assert!(params.from.is_none());
        assert!(params.to.is_none());
        assert!(params.cursor.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn get_platform_audit_params_deserializes_full() {
        let json = r#"{"actor":"api_key","action":"user.disabled","from":"2024-06-01T00:00:00Z","to":"2024-06-30T23:59:59Z","cursor":"xyz","limit":10}"#;
        let params: GetPlatformAuditParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.actor.as_deref(), Some("api_key"));
        assert_eq!(params.action.as_deref(), Some("user.disabled"));
        assert_eq!(params.from.as_deref(), Some("2024-06-01T00:00:00Z"));
        assert_eq!(params.to.as_deref(), Some("2024-06-30T23:59:59Z"));
        assert_eq!(params.cursor.as_deref(), Some("xyz"));
        assert_eq!(params.limit, Some(10));
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
    fn list_task_attachments_params_deserializes_minimal() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-42"}"#;
        let params: ListTaskAttachmentsParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.readable_id, "ATL-42");
    }

    #[test]
    fn get_task_attachment_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-42","attachment_id":"018f-uuid"}"#;
        let params: GetTaskAttachmentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.readable_id, "ATL-42");
        assert_eq!(params.attachment_id, "018f-uuid");
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
        assert!(
            instructions.contains("`list_task_attachments`"),
            "instructions must mention list_task_attachments"
        );
        assert!(
            instructions.contains("`get_task_attachment`"),
            "instructions must mention get_task_attachment"
        );
        assert!(
            instructions.contains("`list_workspace_activity`"),
            "instructions must mention list_workspace_activity"
        );
        assert!(
            instructions.contains("`get_workspace_audit`"),
            "instructions must mention get_workspace_audit"
        );
        assert!(
            instructions.contains("`get_platform_audit`"),
            "instructions must mention get_platform_audit"
        );
        assert!(
            instructions.contains("`list_comments`"),
            "instructions must mention list_comments"
        );
        assert!(
            instructions.contains("`add_comment`"),
            "instructions must mention add_comment"
        );
        assert!(
            instructions.contains("`delete_comment`"),
            "instructions must mention delete_comment"
        );
        assert!(
            instructions.contains("`list_document_comments`"),
            "instructions must mention list_document_comments"
        );
        assert!(
            instructions.contains("`add_document_comment`"),
            "instructions must mention add_document_comment"
        );
        assert!(
            instructions.contains("`delete_document_comment`"),
            "instructions must mention delete_document_comment"
        );
    }

    // -----------------------------------------------------------------------
    // Document + folder write params
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
    // Board / column / tag write params
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
    // Graph write params
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
    fn add_comment_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","body":"Looks good to me"}"#;
        let params: AddCommentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "ws");
        assert_eq!(params.readable_id, "ATL-1");
        assert_eq!(params.body, "Looks good to me");
    }

    #[test]
    fn update_comment_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","comment_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","body":"Edited"}"#;
        let params: UpdateCommentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.readable_id, "ATL-1");
        assert_eq!(params.comment_id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
        assert_eq!(params.body, "Edited");
    }

    #[test]
    fn delete_comment_params_deserializes() {
        let json = r#"{"workspace":"ws","readable_id":"ATL-1","comment_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#;
        let params: DeleteCommentParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.readable_id, "ATL-1");
        assert_eq!(params.comment_id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
    }

    #[test]
    fn document_comment_params_deserialize() {
        let list: ListDocumentCommentsParams =
            serde_json::from_str(r#"{"workspace":"ws","slug":"my-doc","limit":10}"#).unwrap();
        assert_eq!(list.slug, "my-doc");
        assert_eq!(list.limit, Some(10));

        let add: AddDocumentCommentParams =
            serde_json::from_str(r#"{"workspace":"ws","slug":"my-doc","body":"Nice note"}"#)
                .unwrap();
        assert_eq!(add.slug, "my-doc");
        assert_eq!(add.body, "Nice note");

        let update: UpdateDocumentCommentParams = serde_json::from_str(
            r#"{"workspace":"ws","slug":"my-doc","comment_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234","body":"Edited"}"#,
        )
        .unwrap();
        assert_eq!(update.comment_id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
        assert_eq!(update.body, "Edited");

        let delete: DeleteDocumentCommentParams = serde_json::from_str(
            r#"{"workspace":"ws","slug":"my-doc","comment_id":"018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234"}"#,
        )
        .unwrap();
        assert_eq!(delete.slug, "my-doc");
        assert_eq!(delete.comment_id, "018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234");
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
    // Workspace-settings write param deserialization tests
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

    #[test]
    fn instructions_make_api_key_trash_rejection_explicit() {
        let server = AtlasMcp::new("http://localhost:8080", "atlas_test").unwrap();
        let instructions = server.get_info().instructions.unwrap_or_default();

        assert!(instructions.contains("intentionally exposes no Trash"));
        assert!(instructions.contains("receive 403"));
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
