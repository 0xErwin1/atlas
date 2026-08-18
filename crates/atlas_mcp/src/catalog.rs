//! The operation catalog behind the consolidated tool surface.
//!
//! Every capability Atlas exposes is one `(verb, resource)` pair here. The
//! twelve advertised tools are the verbs; the catalog is what turns a resource
//! name into the parameters it accepts, and what a caller gets back when it
//! names one that does not exist.
//!
//! Keeping the per-resource detail here rather than in the tool descriptions is
//! the point of the consolidation: a client pays for twelve schemas up front and
//! looks the rest up on demand, instead of carrying every schema in its context
//! before it asks a single question.

use rmcp::handler::server::wrapper::Parameters;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    AddChecklistItemParams, AddCommentParams, AddDocumentCommentParams, AddTaskAssigneeParams,
    AddTaskReferenceParams, BatchAddTaskReferencesParams, CopyDocumentParams, CopyFolderParams,
    CreateBoardParams, CreateColumnParams, CreateDocumentParams, CreateFolderParams,
    CreatePlatformStatusTemplateParams, CreateProjectParams, CreateSavedSearchParams,
    CreateStatusTemplateParams, CreateSubtaskParams, CreateTagParams, CreateTaskParams,
    CreateTaskViewParams, CreateWebhookParams, DeleteBoardParams, DeleteChecklistItemParams,
    DeleteColumnParams, DeleteCommentParams, DeleteDocumentCommentParams, DeleteDocumentParams,
    DeleteFolderParams, DeletePlatformStatusTemplateParams, DeleteProjectParams,
    DeleteSavedSearchParams, DeleteStatusTemplateParams, DeleteTagParams, DeleteTaskParams,
    DeleteTaskViewParams, DeleteWebhookParams, DocumentCommentAttachmentParams,
    EditDocumentLinesParams, GetAgentIdentityParams, GetDocumentBacklinksParams, GetDocumentParams,
    GetDocumentRevisionParams, GetPlatformAuditParams, GetTaskAttachmentParams,
    GetTaskBacklinksParams, GetTaskGraphParams, GetTaskParams, GetTaskReferencesParams,
    GetWebhookParams, GetWorkspaceAuditParams, ListActivityParams, ListAttachmentsParams,
    ListBoardsParams, ListChecklistParams, ListColumnsParams, ListCommentsParams,
    ListDocumentCommentsParams, ListDocumentHistoryParams, ListDocumentsParams, ListFoldersParams,
    ListMembersParams, ListProjectsParams, ListSavedSearchesParams, ListStatusTemplatesParams,
    ListTagsParams, ListTaskAttachmentsParams, ListTaskViewsParams, ListTasksParams,
    ListUsedLabelsParams, ListWebhookDeliveriesParams, ListWebhooksParams,
    ListWorkspaceActivityParams, ListWorkspacesParams, MoveDocumentParams,
    MoveDocumentsBatchParams, MoveFolderParams, MoveTaskParams, PromoteChecklistItemParams,
    PromoteSubtaskParams, ReadDocumentLinesParams, RemoveTaskAssigneeParams,
    RemoveTaskReferenceParams, RenameFolderParams, RenameSavedSearchParams,
    SearchDocumentContentParams, SearchParams, SetTaskParentParams, TaskCommentAttachmentParams,
    UpdateBoardParams, UpdateChecklistItemParams, UpdateColumnParams, UpdateCommentParams,
    UpdateDocumentCommentParams, UpdateDocumentContentParams, UpdateDocumentMetadataParams,
    UpdatePlatformStatusTemplateParams, UpdateProjectParams, UpdateStatusTemplateParams,
    UpdateTagParams, UpdateTaskParams, UpdateTaskViewParams, UpdateWebhookParams,
    UploadDocumentCommentAttachmentParams, UploadTaskCommentAttachmentParams,
};

/// One callable capability: a verb, the resource it acts on, and its schema.
pub(crate) struct Operation {
    pub(crate) verb: &'static str,
    pub(crate) resource: &'static str,
    /// The name this capability was advertised under before the catalog was
    /// consolidated. Kept so a caller migrating from the old surface can map a
    /// name it already knows onto the verb that now carries it.
    pub(crate) legacy_name: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) schema: fn() -> Value,
}

fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default()
}

/// The schema of a resource that takes no parameters of its own.
fn no_params_schema() -> Value {
    serde_json::json!({ "type": "object", "properties": {} })
}

pub(crate) const OPERATIONS: &[Operation] = &[
    Operation {
        verb: "find",
        resource: "search",
        legacy_name: "search",
        summary: "Search documents and tasks across an Atlas workspace. Retrieval mode is chosen from the query unless `mode` says otherwise.",
        schema: schema_of::<SearchParams>,
    },
    Operation {
        verb: "find",
        resource: "tasks",
        legacy_name: "list_tasks",
        summary: "List tasks across an Atlas workspace with optional filters",
        schema: schema_of::<ListTasksParams>,
    },
    Operation {
        verb: "find",
        resource: "documents",
        legacy_name: "list_documents",
        summary: "List documents in a project within an Atlas workspace. Omit `unfiled` for any document, pass true for unfiled documents, or false for filed documents. Pass `preview: true` to add a short body preview per row, enough to tell same-titled documents apart without a full read.",
        schema: schema_of::<ListDocumentsParams>,
    },
    Operation {
        verb: "find",
        resource: "folders",
        legacy_name: "list_folders",
        summary: "List folders in a project within an Atlas workspace",
        schema: schema_of::<ListFoldersParams>,
    },
    Operation {
        verb: "find",
        resource: "boards",
        legacy_name: "list_boards",
        summary: "List boards in a project within an Atlas workspace",
        schema: schema_of::<ListBoardsParams>,
    },
    Operation {
        verb: "find",
        resource: "columns",
        legacy_name: "list_columns",
        summary: "List columns of a board; use column IDs in list_tasks status filters",
        schema: schema_of::<ListColumnsParams>,
    },
    Operation {
        verb: "find",
        resource: "tags",
        legacy_name: "list_tags",
        summary: "List the registered tag registry for an Atlas workspace",
        schema: schema_of::<ListTagsParams>,
    },
    Operation {
        verb: "find",
        resource: "used_labels",
        legacy_name: "list_used_labels",
        summary: "List labels currently applied to tasks in an Atlas workspace (may include unregistered labels)",
        schema: schema_of::<ListUsedLabelsParams>,
    },
    Operation {
        verb: "find",
        resource: "members",
        legacy_name: "list_members",
        summary: "List workspace members and API-key principals; use IDs in assignee filters",
        schema: schema_of::<ListMembersParams>,
    },
    Operation {
        verb: "find",
        resource: "workspaces",
        legacy_name: "list_workspaces",
        summary: "List all Atlas workspaces accessible to the caller",
        schema: schema_of::<ListWorkspacesParams>,
    },
    Operation {
        verb: "find",
        resource: "projects",
        legacy_name: "list_projects",
        summary: "List projects in an Atlas workspace (cursor-paginated)",
        schema: schema_of::<ListProjectsParams>,
    },
    Operation {
        verb: "find",
        resource: "saved_searches",
        legacy_name: "list_saved_searches",
        summary: "List saved searches for an Atlas workspace",
        schema: schema_of::<ListSavedSearchesParams>,
    },
    Operation {
        verb: "find",
        resource: "task_views",
        legacy_name: "list_task_views",
        summary: "List saved task views (filter presets) for an Atlas workspace",
        schema: schema_of::<ListTaskViewsParams>,
    },
    Operation {
        verb: "find",
        resource: "checklist",
        legacy_name: "list_checklist",
        summary: "List checklist items for a task",
        schema: schema_of::<ListChecklistParams>,
    },
    Operation {
        verb: "find",
        resource: "status_templates",
        legacy_name: "list_status_templates",
        summary: "List the status templates of a workspace, ordered by position. Returns each template's id, name, color, position_key and updated_at — the id is what update_status_template and delete_status_template take, and the position_key is what before/after anchors take.",
        schema: schema_of::<ListStatusTemplatesParams>,
    },
    Operation {
        verb: "find",
        resource: "platform_status_templates",
        legacy_name: "list_platform_status_templates",
        summary: "List the Atlas-wide default statuses new workspaces are seeded from. Requires a root/system-admin session token; API keys receive 403.",
        schema: no_params_schema,
    },
    Operation {
        verb: "find",
        resource: "webhooks",
        legacy_name: "list_webhooks",
        summary: "List webhook subscriptions in a workspace (cursor-paginated). Requires the webhooks:read capability.",
        schema: schema_of::<ListWebhooksParams>,
    },
    Operation {
        verb: "find",
        resource: "webhook_deliveries",
        legacy_name: "list_webhook_deliveries",
        summary: "List delivery attempts for a webhook, newest first (cursor-paginated). Requires the webhooks:read capability.",
        schema: schema_of::<ListWebhookDeliveriesParams>,
    },
    Operation {
        verb: "get",
        resource: "document",
        legacy_name: "get_document",
        summary: "Retrieve an Atlas document by slug or UUID",
        schema: schema_of::<GetDocumentParams>,
    },
    Operation {
        verb: "get",
        resource: "task",
        legacy_name: "get_task",
        summary: "Retrieve a single Atlas task by readable ID",
        schema: schema_of::<GetTaskParams>,
    },
    Operation {
        verb: "get",
        resource: "task_references",
        legacy_name: "get_task_references",
        summary: "List OUTBOUND references from a task — tasks and documents this task links to",
        schema: schema_of::<GetTaskReferencesParams>,
    },
    Operation {
        verb: "get",
        resource: "task_graph",
        legacy_name: "get_task_graph",
        summary: "Graph of what a task links to: references in both directions, sub-tasks, and linked documents, up to `depth` edges away",
        schema: schema_of::<GetTaskGraphParams>,
    },
    Operation {
        verb: "get",
        resource: "task_backlinks",
        legacy_name: "get_task_backlinks",
        summary: "List INBOUND backlinks to a task — other tasks that reference this task",
        schema: schema_of::<GetTaskBacklinksParams>,
    },
    Operation {
        verb: "get",
        resource: "document_backlinks",
        legacy_name: "get_document_backlinks",
        summary: "List documents and tasks that link to a given document (inbound backlinks)",
        schema: schema_of::<GetDocumentBacklinksParams>,
    },
    Operation {
        verb: "get",
        resource: "document_revision",
        legacy_name: "get_document_revision",
        summary: "Fetch the full markdown content of a specific document revision by seq number",
        schema: schema_of::<GetDocumentRevisionParams>,
    },
    Operation {
        verb: "get",
        resource: "webhook",
        legacy_name: "get_webhook",
        summary: "Retrieve a single webhook subscription by UUID (no secret). Requires the webhooks:read capability.",
        schema: schema_of::<GetWebhookParams>,
    },
    Operation {
        verb: "create",
        resource: "task",
        legacy_name: "create_task",
        summary: "Create a task on a board. Board and column are resolved by name.",
        schema: schema_of::<CreateTaskParams>,
    },
    Operation {
        verb: "create",
        resource: "subtask",
        legacy_name: "create_subtask",
        summary: "Create a subtask under a parent task. A subtask is an ordinary task, so it takes the same fields as create_task. Board is inherited from the parent; column defaults to the parent's column.",
        schema: schema_of::<CreateSubtaskParams>,
    },
    Operation {
        verb: "create",
        resource: "document",
        legacy_name: "create_document",
        summary: "Create a document in a project. Returns compact projection with head_revision_id.",
        schema: schema_of::<CreateDocumentParams>,
    },
    Operation {
        verb: "create",
        resource: "document_copy",
        legacy_name: "copy_document",
        summary: "Copy a document. Optional folder_id sets the destination; omit to copy into the same folder.",
        schema: schema_of::<CopyDocumentParams>,
    },
    Operation {
        verb: "create",
        resource: "folder",
        legacy_name: "create_folder",
        summary: "Create a folder inside a project. Optional parent_folder_id nests it; omit for project root.",
        schema: schema_of::<CreateFolderParams>,
    },
    Operation {
        verb: "create",
        resource: "folder_copy",
        legacy_name: "copy_folder",
        summary: "Copy a folder (recursively copies sub-folders and documents). Optional parent_folder_id sets destination; omit to copy under the same parent.",
        schema: schema_of::<CopyFolderParams>,
    },
    Operation {
        verb: "create",
        resource: "board",
        legacy_name: "create_board",
        summary: "Create a new board in a project. A new board is auto-seeded with the workspace's default columns (statuses), which are returned in the `columns` field of the response — do NOT create those columns again; only add columns for statuses that are missing.",
        schema: schema_of::<CreateBoardParams>,
    },
    Operation {
        verb: "create",
        resource: "column",
        legacy_name: "create_column",
        summary: "Create a new column on a board. Optional color and ordering anchors: before = position_key of the column this one follows, after = position_key of the column this one precedes.",
        schema: schema_of::<CreateColumnParams>,
    },
    Operation {
        verb: "create",
        resource: "tag",
        legacy_name: "create_tag",
        summary: "Create a workspace tag. Idempotent by case-insensitive name; returns the existing tag when one already exists.",
        schema: schema_of::<CreateTagParams>,
    },
    Operation {
        verb: "create",
        resource: "project",
        legacy_name: "create_project",
        summary: "Create a new project in the workspace. Returns the created project. Slug must be URL-safe and unique within the workspace.",
        schema: schema_of::<CreateProjectParams>,
    },
    Operation {
        verb: "create",
        resource: "status_template",
        legacy_name: "create_status_template",
        summary: "Create a workspace status template. Optional color swatch and ordering anchors: before = position_key of the template this one follows, after = position_key of the template this one precedes. Returns the created template.",
        schema: schema_of::<CreateStatusTemplateParams>,
    },
    Operation {
        verb: "create",
        resource: "platform_status_template",
        legacy_name: "create_platform_status_template",
        summary: "Create an Atlas-wide default status, appended last. Affects workspaces created afterwards, never existing ones. Requires a root/system-admin session token; API keys receive 403.",
        schema: schema_of::<CreatePlatformStatusTemplateParams>,
    },
    Operation {
        verb: "create",
        resource: "saved_search",
        legacy_name: "create_saved_search",
        summary: "Create a saved search in the workspace. Returns the created saved search with its id for future rename or delete.",
        schema: schema_of::<CreateSavedSearchParams>,
    },
    Operation {
        verb: "create",
        resource: "task_view",
        legacy_name: "create_task_view",
        summary: "Create a task view (filter preset) in the workspace. Pass an empty filters object {} for an all-workspace view. Returns the created task view.",
        schema: schema_of::<CreateTaskViewParams>,
    },
    Operation {
        verb: "create",
        resource: "webhook",
        legacy_name: "create_webhook",
        summary: "Create a webhook subscription. Requires the webhooks:create capability. The response carries the one-time signing secret (whsec_…) under `secret`; it is shown exactly once and never retrievable again — store it immediately.",
        schema: schema_of::<CreateWebhookParams>,
    },
    Operation {
        verb: "create",
        resource: "task_assignee",
        legacy_name: "add_task_assignee",
        summary: "Add an assignee (user or API key) to a task.",
        schema: schema_of::<AddTaskAssigneeParams>,
    },
    Operation {
        verb: "create",
        resource: "task_reference",
        legacy_name: "add_task_reference",
        summary: "Add a typed reference from a task to another task or document. kind must be one of: relates, blocks, parent, spec. Supply exactly one of target_task_readable_id or target_document_id.",
        schema: schema_of::<AddTaskReferenceParams>,
    },
    Operation {
        verb: "create",
        resource: "task_references_batch",
        legacy_name: "add_task_references_batch",
        summary: "Add up to 100 typed references to a task. Results remain ordered by input and each item is either a created reference or a structured problem.",
        schema: schema_of::<BatchAddTaskReferencesParams>,
    },
    Operation {
        verb: "create",
        resource: "checklist_item",
        legacy_name: "add_checklist_item",
        summary: "Add a checklist item to a task. Optional ordering anchors: before = position_key of the item this one follows, after = position_key of the item this one precedes.",
        schema: schema_of::<AddChecklistItemParams>,
    },
    Operation {
        verb: "update",
        resource: "task",
        legacy_name: "update_task",
        summary: "Update a task. PATCH semantics: omit a field to leave it unchanged; pass JSON null to clear a clearable field (priority, due_date, estimate).",
        schema: schema_of::<UpdateTaskParams>,
    },
    Operation {
        verb: "update",
        resource: "document_metadata",
        legacy_name: "update_document_metadata",
        summary: "Update document title or folder (metadata only). PATCH: omit fields to leave unchanged. Use update_document_content to change content.",
        schema: schema_of::<UpdateDocumentMetadataParams>,
    },
    Operation {
        verb: "update",
        resource: "board",
        legacy_name: "update_board",
        summary: "Rename a board. Board resolved by name (partial match) or UUID.",
        schema: schema_of::<UpdateBoardParams>,
    },
    Operation {
        verb: "update",
        resource: "column",
        legacy_name: "update_column",
        summary: "Update a column: rename, recolor, or reorder. Column resolved by name on the board. Color: omit to leave unchanged, pass null to clear, pass a string to set. Reorder with the anchors: before = position_key of the column this one follows, after = position_key of the column this one precedes.",
        schema: schema_of::<UpdateColumnParams>,
    },
    Operation {
        verb: "update",
        resource: "tag",
        legacy_name: "update_tag",
        summary: "Update a tag's name and/or color. Omit color to leave it unchanged. Note: a tag color cannot be cleared once set (set a new color to change it).",
        schema: schema_of::<UpdateTagParams>,
    },
    Operation {
        verb: "update",
        resource: "checklist_item",
        legacy_name: "update_checklist_item",
        summary: "Update a checklist item (PATCH). Omit title or checked to leave unchanged. Optional ordering anchors: before = position_key of the item this one follows, after = position_key of the item this one precedes.",
        schema: schema_of::<UpdateChecklistItemParams>,
    },
    Operation {
        verb: "update",
        resource: "project",
        legacy_name: "update_project",
        summary: "Update a project's metadata (name, visibility, task_prefix). PATCH semantics: omit a field to leave it unchanged. Returns the updated project.",
        schema: schema_of::<UpdateProjectParams>,
    },
    Operation {
        verb: "update",
        resource: "status_template",
        legacy_name: "update_status_template",
        summary: "Update a workspace status template: rename, recolor, or reorder. name and color are optional PATCH fields; color accepts null to clear. Reorder with the anchors: before = position_key of the template this one follows, after = position_key of the template this one precedes. Returns the updated template.",
        schema: schema_of::<UpdateStatusTemplateParams>,
    },
    Operation {
        verb: "update",
        resource: "platform_status_template",
        legacy_name: "update_platform_status_template",
        summary: "Update an Atlas-wide default status (rename, recolor, reorder). color accepts null to clear. Reorder with the anchors: before = position_key of the status this one follows, after = position_key of the status this one precedes. Requires a root/system-admin session token; API keys receive 403.",
        schema: schema_of::<UpdatePlatformStatusTemplateParams>,
    },
    Operation {
        verb: "update",
        resource: "task_view",
        legacy_name: "update_task_view",
        summary: "Update a task view. Both name and filters are required — this is a full replacement, not a PATCH. Returns the updated task view.",
        schema: schema_of::<UpdateTaskViewParams>,
    },
    Operation {
        verb: "update",
        resource: "webhook",
        legacy_name: "update_webhook",
        summary: "Update a webhook subscription (PATCH: omit a field to leave it unchanged). Requires the webhooks:update capability. The signing secret is never rotated through this tool.",
        schema: schema_of::<UpdateWebhookParams>,
    },
    Operation {
        verb: "update",
        resource: "folder_name",
        legacy_name: "rename_folder",
        summary: "Rename a folder.",
        schema: schema_of::<RenameFolderParams>,
    },
    Operation {
        verb: "update",
        resource: "saved_search_name",
        legacy_name: "rename_saved_search",
        summary: "Rename a saved search. To change the query, delete and recreate. Returns the updated saved search.",
        schema: schema_of::<RenameSavedSearchParams>,
    },
    Operation {
        verb: "delete",
        resource: "task",
        legacy_name: "delete_task",
        summary: "Delete a task permanently. Requires confirm: true. This operation is not auto-reversible.",
        schema: schema_of::<DeleteTaskParams>,
    },
    Operation {
        verb: "delete",
        resource: "document",
        legacy_name: "delete_document",
        summary: "Recoverably delete a document. Requires confirm: true. Permanent removal is available only through root/system-admin human Trash purge.",
        schema: schema_of::<DeleteDocumentParams>,
    },
    Operation {
        verb: "delete",
        resource: "folder",
        legacy_name: "delete_folder",
        summary: "Recoverably delete a folder. Requires confirm: true. Documents inside keep their folder_id and are hidden until the folder is restored.",
        schema: schema_of::<DeleteFolderParams>,
    },
    Operation {
        verb: "delete",
        resource: "board",
        legacy_name: "delete_board",
        summary: "Delete a board. Requires confirm: true. Soft-deletes only the board row; columns and tasks become unreachable from listings but their rows persist.",
        schema: schema_of::<DeleteBoardParams>,
    },
    Operation {
        verb: "delete",
        resource: "column",
        legacy_name: "delete_column",
        summary: "Delete a column. Requires confirm: true. The server refuses deletion when the column still has tasks — move or delete the tasks first.",
        schema: schema_of::<DeleteColumnParams>,
    },
    Operation {
        verb: "delete",
        resource: "tag",
        legacy_name: "delete_tag",
        summary: "Soft-delete a workspace tag. Task label strings are preserved after deletion.",
        schema: schema_of::<DeleteTagParams>,
    },
    Operation {
        verb: "delete",
        resource: "checklist_item",
        legacy_name: "delete_checklist_item",
        summary: "Delete a checklist item from a task.",
        schema: schema_of::<DeleteChecklistItemParams>,
    },
    Operation {
        verb: "delete",
        resource: "project",
        legacy_name: "delete_project",
        summary: "Recoverably delete a project. Requires confirm: true. Descendants are hidden until the project is restored; permanent removal is a separate root/system-admin human Trash purge workflow.",
        schema: schema_of::<DeleteProjectParams>,
    },
    Operation {
        verb: "delete",
        resource: "status_template",
        legacy_name: "delete_status_template",
        summary: "Delete a workspace status template. Plain delete, no confirm required. Returns {deleted: true, id}.",
        schema: schema_of::<DeleteStatusTemplateParams>,
    },
    Operation {
        verb: "delete",
        resource: "platform_status_template",
        legacy_name: "delete_platform_status_template",
        summary: "Delete an Atlas-wide default status. Plain delete, no confirm required. Existing workspaces keep the statuses they were seeded with. Requires a root/system-admin session token; API keys receive 403. Returns {deleted: true, id}.",
        schema: schema_of::<DeletePlatformStatusTemplateParams>,
    },
    Operation {
        verb: "delete",
        resource: "saved_search",
        legacy_name: "delete_saved_search",
        summary: "Delete a saved search. Plain delete, no confirm required. Returns {deleted: true, id}.",
        schema: schema_of::<DeleteSavedSearchParams>,
    },
    Operation {
        verb: "delete",
        resource: "task_view",
        legacy_name: "delete_task_view",
        summary: "Delete a task view. Plain delete, no confirm required. Returns {deleted: true, id}.",
        schema: schema_of::<DeleteTaskViewParams>,
    },
    Operation {
        verb: "delete",
        resource: "webhook",
        legacy_name: "delete_webhook",
        summary: "Delete a webhook subscription. Requires confirm: true and the webhooks:delete capability. Soft-deletes the subscription.",
        schema: schema_of::<DeleteWebhookParams>,
    },
    Operation {
        verb: "delete",
        resource: "task_assignee",
        legacy_name: "remove_task_assignee",
        summary: "Remove an assignee from a task by their UUID reference.",
        schema: schema_of::<RemoveTaskAssigneeParams>,
    },
    Operation {
        verb: "delete",
        resource: "task_reference",
        legacy_name: "remove_task_reference",
        summary: "Remove an outbound reference from a task. reference_id is the UUID from get_task_references.",
        schema: schema_of::<RemoveTaskReferenceParams>,
    },
    Operation {
        verb: "move",
        resource: "task",
        legacy_name: "move_task",
        summary: "Move a task to a different column (resolved by name). Errors with the board's column list when the column is not found.",
        schema: schema_of::<MoveTaskParams>,
    },
    Operation {
        verb: "move",
        resource: "document",
        legacy_name: "move_document",
        summary: "Move a document to a different folder. Omit folder_id to move to the project root.",
        schema: schema_of::<MoveDocumentParams>,
    },
    Operation {
        verb: "move",
        resource: "documents_batch",
        legacy_name: "move_documents_batch",
        summary: "Move up to 100 documents independently. Results remain ordered by input and each item is either a moved compact document or a structured problem.",
        schema: schema_of::<MoveDocumentsBatchParams>,
    },
    Operation {
        verb: "move",
        resource: "folder",
        legacy_name: "move_folder",
        summary: "Move a folder to a new parent. Omit parent_folder_id to move to the project root. Note: ordering within the parent is not supported.",
        schema: schema_of::<MoveFolderParams>,
    },
    Operation {
        verb: "move",
        resource: "task_parent",
        legacy_name: "set_task_parent",
        summary: "Convert an existing task into a subtask of another task. The task keeps its own board, column and position — only the parent link changes, so a subtask may live on a different board than its parent. Use promote_subtask to detach it again.",
        schema: schema_of::<SetTaskParentParams>,
    },
    Operation {
        verb: "move",
        resource: "checklist_item_promotion",
        legacy_name: "promote_checklist_item",
        summary: "Promote a checklist item to a full task on the specified board and column. Returns the new task and the updated checklist item.",
        schema: schema_of::<PromoteChecklistItemParams>,
    },
    Operation {
        verb: "move",
        resource: "subtask_promotion",
        legacy_name: "promote_subtask",
        summary: "Promote a subtask to a top-level task, detaching it from its parent.",
        schema: schema_of::<PromoteSubtaskParams>,
    },
    Operation {
        verb: "document_edit",
        resource: "read_lines",
        legacy_name: "read_document_lines",
        summary: "Read a bounded inclusive range of numbered document lines",
        schema: schema_of::<ReadDocumentLinesParams>,
    },
    Operation {
        verb: "document_edit",
        resource: "search_content",
        legacy_name: "search_document_content",
        summary: "Search a bounded document line range using literal text or a Rust regex",
        schema: schema_of::<SearchDocumentContentParams>,
    },
    Operation {
        verb: "document_edit",
        resource: "edit_lines",
        legacy_name: "edit_document_lines",
        summary: "Apply a CAS-protected insert, replace, or delete to document lines. Read a document first and pass its head_revision_id as base_revision_id. Insert requires position and content; replace requires start, end, and content; delete requires start and end. On revision_conflict, apply base_to_current_patch and retry with current_revision_id.",
        schema: schema_of::<EditDocumentLinesParams>,
    },
    Operation {
        verb: "document_edit",
        resource: "replace_content",
        legacy_name: "update_document_content",
        summary: "Write new content to a document using compare-and-swap. Read with get_document detail=full to get head_revision_id + content, edit locally, then call with base_revision_id = head_revision_id. On revision_conflict: apply base_to_current_patch to your edit and retry with base_revision_id = current_revision_id.",
        schema: schema_of::<UpdateDocumentContentParams>,
    },
    Operation {
        verb: "comment",
        resource: "task_list",
        legacy_name: "list_comments",
        summary: "List markdown comments on a task, oldest first",
        schema: schema_of::<ListCommentsParams>,
    },
    Operation {
        verb: "comment",
        resource: "task_feed",
        legacy_name: "list_comment_feed",
        summary: "List the full authorized task comment feed, including derived links and retained events, oldest first",
        schema: schema_of::<ListCommentsParams>,
    },
    Operation {
        verb: "comment",
        resource: "task_add",
        legacy_name: "add_comment",
        summary: "Post a markdown comment on a task (max 10 000 characters)",
        schema: schema_of::<AddCommentParams>,
    },
    Operation {
        verb: "comment",
        resource: "task_update",
        legacy_name: "update_comment",
        summary: "Edit a task comment's body (max 10 000 characters). Only the comment's author may edit it; anyone else gets a permission error.",
        schema: schema_of::<UpdateCommentParams>,
    },
    Operation {
        verb: "comment",
        resource: "task_delete",
        legacy_name: "delete_comment",
        summary: "Delete a task comment. The comment's author or a workspace admin/owner may delete it; anyone else gets a permission error.",
        schema: schema_of::<DeleteCommentParams>,
    },
    Operation {
        verb: "comment",
        resource: "document_list",
        legacy_name: "list_document_comments",
        summary: "List markdown comments on a document, oldest first",
        schema: schema_of::<ListDocumentCommentsParams>,
    },
    Operation {
        verb: "comment",
        resource: "document_feed",
        legacy_name: "list_document_comment_feed",
        summary: "List the full authorized document comment feed, including derived links and retained events, oldest first",
        schema: schema_of::<ListDocumentCommentsParams>,
    },
    Operation {
        verb: "comment",
        resource: "document_add",
        legacy_name: "add_document_comment",
        summary: "Post a markdown comment on a document (max 10 000 characters)",
        schema: schema_of::<AddDocumentCommentParams>,
    },
    Operation {
        verb: "comment",
        resource: "document_update",
        legacy_name: "update_document_comment",
        summary: "Edit a document comment's body (max 10 000 characters). Only the comment's author may edit it; anyone else gets a permission error.",
        schema: schema_of::<UpdateDocumentCommentParams>,
    },
    Operation {
        verb: "comment",
        resource: "document_delete",
        legacy_name: "delete_document_comment",
        summary: "Delete a document comment. The comment's author or a workspace admin/owner may delete it; anyone else gets a permission error.",
        schema: schema_of::<DeleteDocumentCommentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "document_list",
        legacy_name: "list_attachments",
        summary: "List attachment metadata for a document (file name, type, size)",
        schema: schema_of::<ListAttachmentsParams>,
    },
    Operation {
        verb: "attachment",
        resource: "task_list",
        legacy_name: "list_task_attachments",
        summary: "List attachment metadata for a task (file name, type, size)",
        schema: schema_of::<ListTaskAttachmentsParams>,
    },
    Operation {
        verb: "attachment",
        resource: "task_get",
        legacy_name: "get_task_attachment",
        summary: "Retrieve a task attachment as viewable content. Image attachments are returned as an image; textual attachments (text/*, JSON, XML, YAML, TOML) are returned as text. Other binary types are rejected. Pass the attachment UUID from `list_task_attachments`.",
        schema: schema_of::<GetTaskAttachmentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "task_comment_upload",
        legacy_name: "upload_task_comment_attachment",
        summary: "Upload a file owned by a task comment. Content must be strict padded standard base64.",
        schema: schema_of::<UploadTaskCommentAttachmentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "task_comment_list",
        legacy_name: "list_task_comment_attachments",
        summary: "List attachment metadata for a task comment",
        schema: schema_of::<TaskCommentAttachmentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "task_comment_get",
        legacy_name: "get_task_comment_attachment",
        summary: "Download a task comment attachment as standard base64 content",
        schema: schema_of::<TaskCommentAttachmentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "task_comment_delete",
        legacy_name: "delete_task_comment_attachment",
        summary: "Delete a task comment attachment. Plain delete, no confirm required (consistent with delete_comment, which removes the whole comment).",
        schema: schema_of::<TaskCommentAttachmentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "document_comment_upload",
        legacy_name: "upload_document_comment_attachment",
        summary: "Upload a file owned by a document comment. Content must be strict padded standard base64.",
        schema: schema_of::<UploadDocumentCommentAttachmentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "document_comment_list",
        legacy_name: "list_document_comment_attachments",
        summary: "List attachment metadata for a document comment",
        schema: schema_of::<DocumentCommentAttachmentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "document_comment_get",
        legacy_name: "get_document_comment_attachment",
        summary: "Download a document comment attachment as standard base64 content",
        schema: schema_of::<DocumentCommentAttachmentParams>,
    },
    Operation {
        verb: "attachment",
        resource: "document_comment_delete",
        legacy_name: "delete_document_comment_attachment",
        summary: "Delete a document comment attachment. Plain delete, no confirm required (consistent with delete_document_comment, which removes the whole comment).",
        schema: schema_of::<DocumentCommentAttachmentParams>,
    },
    Operation {
        verb: "activity",
        resource: "task",
        legacy_name: "list_activity",
        summary: "List the activity log for a task (moves, assignments, field changes)",
        schema: schema_of::<ListActivityParams>,
    },
    Operation {
        verb: "activity",
        resource: "workspace",
        legacy_name: "list_workspace_activity",
        summary: "List the access-filtered activity feed for an entire workspace. Each entry shows who did what on which task (task_readable_id, kind, actor with display_name and account_status, payload, created_at). Server-side filtering ensures the caller only sees events for tasks they can access. Supports actor-type (user|api_key), date range, and cursor pagination.",
        schema: schema_of::<ListWorkspaceActivityParams>,
    },
    Operation {
        verb: "activity",
        resource: "workspace_audit",
        legacy_name: "get_workspace_audit",
        summary: "List the security audit log for a workspace (owner/admin only). Returns who performed each privileged action (membership changes, permission grants, API key lifecycle), with enriched actor details (display_name, account_status for users; key_type for API keys). Returns 403 if the caller is not a workspace owner or admin — audit requires workspace owner/admin or platform admin. Supports actor-type (user|api_key), action verb, date range, and cursor pagination.",
        schema: schema_of::<GetWorkspaceAuditParams>,
    },
    Operation {
        verb: "activity",
        resource: "platform_audit",
        legacy_name: "get_platform_audit",
        summary: "List the platform-wide security audit log (platform admin only). Returns platform-scoped events (user lifecycle: created, disabled, enabled, password reset, activation; system-admin flag changes). Returns 403 if the caller is not a platform admin — audit requires workspace owner/admin or platform admin. Supports actor-type (user|api_key), action verb, date range, and cursor pagination.",
        schema: schema_of::<GetPlatformAuditParams>,
    },
    Operation {
        verb: "activity",
        resource: "document_history",
        legacy_name: "list_document_history",
        summary: "List revision metadata for a document (history of edits)",
        schema: schema_of::<ListDocumentHistoryParams>,
    },
    Operation {
        verb: "identity",
        resource: "ping",
        legacy_name: "ping",
        summary: "Ping the Atlas MCP server",
        schema: no_params_schema,
    },
    Operation {
        verb: "identity",
        resource: "agent",
        legacy_name: "get_agent_identity",
        summary: "Report the calling API key's own identity: its id, name, and the capability scopes it holds. Read-only self-inspection; returns a note when the caller is a human, not an agent key.",
        schema: schema_of::<GetAgentIdentityParams>,
    },
];

/// The verbs, in the order they are advertised.
pub(crate) const VERBS: &[&str] = &[
    "find",
    "get",
    "create",
    "update",
    "delete",
    "move",
    "document_edit",
    "comment",
    "attachment",
    "activity",
    "identity",
];

pub(crate) fn operations_for(verb: &str) -> impl Iterator<Item = &'static Operation> {
    OPERATIONS.iter().filter(move |op| op.verb == verb)
}

pub(crate) fn find_by_legacy_name(name: &str) -> Option<&'static Operation> {
    OPERATIONS.iter().find(|op| op.legacy_name == name)
}

pub(crate) fn find_operation(verb: &str, resource: &str) -> Option<&'static Operation> {
    OPERATIONS
        .iter()
        .find(|op| op.verb == verb && op.resource == resource)
}

/// The resource names a verb accepts, for a tool description or an error.
pub(crate) fn resource_names(verb: &str) -> String {
    operations_for(verb)
        .map(|op| op.resource)
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The parameter names a resource accepts, required ones marked.
///
/// Derived from the same schema the resource deserializes with, so it cannot
/// drift from what the call actually takes.
pub(crate) fn accepted_parameters(op: &Operation) -> String {
    let schema = (op.schema)();

    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return "none".to_owned();
    };
    if properties.is_empty() {
        return "none".to_owned();
    }

    properties
        .keys()
        .map(|name| {
            if required.iter().any(|r| r == name) {
                format!("{name} (required)")
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Normalizes the two shapes a client sends that are not the object the schema
/// asks for.
///
/// An omitted `params` arrives as JSON null, which no parameter struct accepts
/// even when every one of its fields is optional. A client that treats an
/// untyped schema as free-form sends the object serialized as a string. Both
/// carry the caller's real intent, so they are repaired rather than rejected.
fn normalize_params(params: Value) -> Value {
    match params {
        Value::Null => Value::Object(serde_json::Map::new()),
        Value::String(ref text) => serde_json::from_str(text).unwrap_or(params),
        other => other,
    }
}

/// Deserializes a call's `params` into the resource's own parameter type.
///
/// A failure answers with the accepted parameter set rather than a bare serde
/// message, so a caller can correct itself in one round trip instead of
/// guessing at the shape.
pub(crate) fn decode<T: DeserializeOwned + JsonSchema>(
    verb: &str,
    resource: &str,
    params: Value,
) -> Result<Parameters<T>, String> {
    serde_json::from_value(normalize_params(params)).map(Parameters).map_err(|error| {
        let accepted = find_operation(verb, resource)
            .map(accepted_parameters)
            .unwrap_or_else(|| "unknown".to_owned());

        format!(
            "invalid params for {verb} resource `{resource}`: {error}. Accepted parameters: {accepted}.              Call `help` with this verb and resource for the full schema."
        )
    })
}

/// The answer to a resource name this verb does not have.
pub(crate) fn unknown_resource(verb: &str, resource: &str) -> String {
    format!(
        "unknown resource `{resource}` for `{verb}`. Accepted resources: {}.          Call `help` with no arguments to see every verb.",
        resource_names(verb)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every capability the pre-consolidation catalog advertised as its own
    /// tool. Nothing may leave this list: consolidating the surface was allowed
    /// to change how a capability is addressed, never whether it is reachable.
    const LEGACY_TOOL_NAMES: &[&str] = &[
        "ping",
        "search",
        "get_document",
        "read_document_lines",
        "search_document_content",
        "list_tasks",
        "get_task",
        "list_documents",
        "list_folders",
        "list_boards",
        "list_columns",
        "list_tags",
        "list_used_labels",
        "list_members",
        "list_workspaces",
        "get_agent_identity",
        "list_projects",
        "list_saved_searches",
        "list_task_views",
        "get_task_references",
        "get_task_graph",
        "get_task_backlinks",
        "get_document_backlinks",
        "list_checklist",
        "list_activity",
        "list_comments",
        "list_comment_feed",
        "upload_task_comment_attachment",
        "list_task_comment_attachments",
        "get_task_comment_attachment",
        "delete_task_comment_attachment",
        "list_workspace_activity",
        "list_document_history",
        "get_document_revision",
        "list_attachments",
        "list_task_attachments",
        "get_task_attachment",
        "create_task",
        "update_task",
        "move_task",
        "delete_task",
        "add_task_assignee",
        "remove_task_assignee",
        "create_document",
        "update_document_metadata",
        "update_document_content",
        "edit_document_lines",
        "delete_document",
        "move_document",
        "move_documents_batch",
        "copy_document",
        "create_folder",
        "rename_folder",
        "move_folder",
        "copy_folder",
        "delete_folder",
        "create_board",
        "update_board",
        "delete_board",
        "create_column",
        "update_column",
        "delete_column",
        "create_tag",
        "update_tag",
        "delete_tag",
        "add_task_reference",
        "add_task_references_batch",
        "remove_task_reference",
        "add_checklist_item",
        "update_checklist_item",
        "delete_checklist_item",
        "add_comment",
        "update_comment",
        "delete_comment",
        "list_document_comments",
        "list_document_comment_feed",
        "upload_document_comment_attachment",
        "list_document_comment_attachments",
        "get_document_comment_attachment",
        "delete_document_comment_attachment",
        "add_document_comment",
        "update_document_comment",
        "delete_document_comment",
        "promote_checklist_item",
        "create_subtask",
        "set_task_parent",
        "promote_subtask",
        "create_project",
        "update_project",
        "delete_project",
        "list_status_templates",
        "create_status_template",
        "update_status_template",
        "delete_status_template",
        "list_platform_status_templates",
        "create_platform_status_template",
        "update_platform_status_template",
        "delete_platform_status_template",
        "create_saved_search",
        "rename_saved_search",
        "delete_saved_search",
        "create_task_view",
        "update_task_view",
        "delete_task_view",
        "get_workspace_audit",
        "get_platform_audit",
        "list_webhooks",
        "get_webhook",
        "list_webhook_deliveries",
        "create_webhook",
        "update_webhook",
        "delete_webhook",
    ];

    #[test]
    fn every_capability_of_the_old_catalog_is_still_reachable() {
        for name in LEGACY_TOOL_NAMES {
            assert!(
                find_by_legacy_name(name).is_some(),
                "capability `{name}` is no longer reachable through any verb"
            );
        }
    }

    #[test]
    fn the_catalog_adds_no_capability_the_old_one_did_not_have() {
        for op in OPERATIONS {
            assert!(
                LEGACY_TOOL_NAMES.contains(&op.legacy_name),
                "`{}/{}` maps to `{}`, which was never an advertised tool",
                op.verb,
                op.resource,
                op.legacy_name
            );
        }

        assert_eq!(OPERATIONS.len(), LEGACY_TOOL_NAMES.len());
    }

    #[test]
    fn no_capability_is_reachable_through_two_resources() {
        let mut seen = std::collections::BTreeSet::new();
        for op in OPERATIONS {
            assert!(
                seen.insert(op.legacy_name),
                "`{}` is reachable through more than one resource",
                op.legacy_name
            );
        }
    }

    #[test]
    fn every_operation_belongs_to_an_advertised_verb() {
        for op in OPERATIONS {
            assert!(
                VERBS.contains(&op.verb),
                "operation `{}` has verb `{}`, which is not advertised",
                op.resource,
                op.verb
            );
        }
    }

    #[test]
    fn every_verb_has_at_least_one_resource() {
        for verb in VERBS {
            assert!(
                operations_for(verb).next().is_some(),
                "verb `{verb}` advertises no resources"
            );
        }
    }

    #[test]
    fn no_verb_declares_the_same_resource_twice() {
        for verb in VERBS {
            let mut seen = std::collections::BTreeSet::new();
            for op in operations_for(verb) {
                assert!(
                    seen.insert(op.resource),
                    "verb `{verb}` declares resource `{}` twice",
                    op.resource
                );
            }
        }
    }

    #[test]
    fn every_operation_carries_a_summary() {
        for op in OPERATIONS {
            assert!(
                !op.summary.trim().is_empty(),
                "operation `{}/{}` has no summary",
                op.verb,
                op.resource
            );
        }
    }

    #[test]
    fn accepted_parameters_names_required_fields() {
        let op = find_operation("get", "task").unwrap_or_else(|| unreachable!());
        let accepted = accepted_parameters(op);

        assert!(accepted.contains("workspace (required)"), "{accepted}");
        assert!(accepted.contains("readable_id (required)"), "{accepted}");
        assert!(accepted.contains("detail"), "{accepted}");
    }

    #[test]
    fn a_resource_with_no_parameters_says_so() {
        let op = find_operation("identity", "ping").unwrap_or_else(|| unreachable!());

        assert_eq!(accepted_parameters(op), "none");
    }

    #[test]
    fn an_unknown_resource_error_lists_the_real_ones() {
        let message = unknown_resource("get", "taskk");

        assert!(message.contains("unknown resource `taskk`"), "{message}");
        assert!(message.contains("task_graph"), "{message}");
        assert!(message.contains("help"), "{message}");
    }

    #[test]
    fn an_invalid_params_error_lists_the_accepted_ones() {
        let error = decode::<crate::GetTaskParams>(
            "get",
            "task",
            serde_json::json!({ "workspace": "atlas" }),
        )
        .err()
        .unwrap_or_default();

        assert!(error.contains("Accepted parameters"), "{error}");
        assert!(error.contains("readable_id (required)"), "{error}");
    }

    #[test]
    fn params_serialized_as_a_string_still_decode() {
        let Parameters(params) = decode::<crate::GetTaskParams>(
            "get",
            "task",
            serde_json::json!(r#"{"workspace":"atlas","readable_id":"ATL-42"}"#),
        )
        .unwrap_or_else(|error| unreachable!("{error}"));

        assert_eq!(params.workspace, "atlas");
        assert_eq!(params.readable_id, "ATL-42");
    }

    #[test]
    fn omitted_params_decode_as_an_empty_object() {
        assert!(
            decode::<crate::ListWorkspacesParams>("find", "workspaces", Value::Null).is_ok(),
            "an omitted `params` must reach a resource that takes none"
        );
    }

    #[test]
    fn a_string_that_is_not_json_keeps_its_own_error() {
        let error = decode::<crate::GetTaskParams>("get", "task", serde_json::json!("ATL-42"))
            .err()
            .unwrap_or_default();

        assert!(error.contains("invalid type: string"), "{error}");
    }
}
