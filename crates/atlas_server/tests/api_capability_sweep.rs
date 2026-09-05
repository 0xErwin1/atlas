#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Black-box capability sweep for API key scopes.
//!
//! Two invariants, exercised through the real HTTP router (not the gate
//! function directly), so both the `Authorized<R, M, S>` extractor path and
//! the manual `enforce_api_key_scope` call sites are covered identically:
//!
//! 1. A key with an EMPTY scope set gets 403 with a scope-denial detail on
//!    every live registry route that declares `action: Some(_)`.
//! 2. A key holding all catalog capabilities never gets a scope-403 on any
//!    of those same entries (it may still get a 404 from an earlier
//!    destructive call in the same sweep touching a shared fixture — that is
//!    not a scope denial and is explicitly allowed here).
//!
//! Every path parameter beyond the extractor's own primary resource (e.g. a
//! checklist item id, a comment id, a column id nested under a board) is a
//! throwaway value: the `Authorized` extractor is always the first handler
//! parameter, so a 403 from the capability gate is returned before any
//! secondary `Path<...>` extractor or the JSON body is ever parsed. Only the
//! primary resource per entry (project slug, board id, task readable_id,
//! document ref, folder id) and the two manual attachment-gate routes' real
//! attachment id need to exist.

mod support;

use atlas_acta::actor::Actor;
use atlas_acta_postgres::repos::webhook_subscription::PgWebhookSubscriptionRepo;
use atlas_api::{
    dtos::{
        CreateProjectRequest, UpdateProjectRequest, UpdateWorkspaceRequest,
        boards_tasks::{
            AddAssigneeRequest, CreateBoardRequest, CreateChecklistItemRequest,
            CreateColumnRequest, CreateCommentRequest, CreateReferenceRequest,
            CreateSubtaskRequest, CreateTaskRequest, MoveBoardRequest, MoveTaskRequest,
            PromoteChecklistItemRequest, RenameTaskAttachmentRequest, SetTaskParentRequest,
            UpdateBoardRequest, UpdateChecklistItemRequest, UpdateColumnRequest,
            UpdateCommentRequest, UpdateTaskRequest, WorkspaceTaskQueryParams,
        },
        documents::{
            CreateDocumentRequest, DocumentCompactDto, DocumentContentEditRequest,
            DocumentLineEditRequest, MoveDocumentRequest, UpdateContentRequest,
            UpdateDocumentRequest,
        },
        folders::{CreateFolderRequest, MoveFolderRequest, RenameFolderRequest},
        saved_searches::{CreateSavedSearchRequest, RenameSavedSearchRequest},
        status_templates::{CreateStatusTemplateRequest, UpdateStatusTemplateRequest},
        task_views::{CreateTaskViewRequest, TaskViewFiltersDto, UpdateTaskViewRequest},
    },
    problem::ProblemDetails,
};
use atlas_client::{AtlasClient, ClientError};
use atlas_core::principal::UserId;
use atlas_core::registry::{HttpMethod, build};
use atlas_custos::capability::Capability;
use atlas_custos::entities::identity::ApiKeyType;
use atlas_custos_postgres::repos::identity::PgApiKeyRepo;
use atlas_server::{
    crypto::WebhookCrypto,
    persistence::repos::{ApiKeyRepo, NewApiKey},
    reg5::{StorageBackend, reg5_component_entries},
    router_audit::capability_from_action_id,
};
use support::{TestDb, TestServer, login_user_with_workspace};

struct Fixtures {
    ws_slug: String,
    project_slug: String,
    board_id: uuid::Uuid,
    task_readable_id: String,
    document_ref: String,
    document_head_revision_id: uuid::Uuid,
    folder_id: uuid::Uuid,
    doc_attachment_id: uuid::Uuid,
    task_comment_id: uuid::Uuid,
    task_comment_attachment_id: uuid::Uuid,
    document_comment_id: uuid::Uuid,
    document_comment_attachment_id: uuid::Uuid,
    webhook_id: uuid::Uuid,
}

async fn seed_fixtures(
    owner: &AtlasClient,
    db: &TestDb,
    ws_slug: &str,
    ws_id: uuid::Uuid,
    user_id: UserId,
) -> Fixtures {
    let project = owner
        .acta()
        .create_project(
            ws_slug,
            CreateProjectRequest {
                name: "Capability Sweep Project".into(),
                slug: "cap-sweep-proj".into(),
                task_prefix: "CSW".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = owner
        .acta()
        .create_board(
            ws_slug,
            &project.slug,
            CreateBoardRequest {
                folder_id: None,
                name: "Sweep Board".into(),
            },
        )
        .await
        .expect("create board");

    let column = owner
        .acta()
        .create_column(
            ws_slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".into(),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = owner
        .acta()
        .create_task(
            ws_slug,
            board.id,
            CreateTaskRequest {
                references: vec![],
                column_id: column.id,
                title: "Sweep task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let document = owner
        .acta()
        .create_document(
            ws_slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Sweep doc".into(),
                folder_id: None,
                content: Some("hello".into()),
            },
        )
        .await
        .expect("create document");

    let attachment = owner
        .acta()
        .upload_attachment(
            ws_slug,
            &document.id.to_string(),
            "sweep.txt",
            "text/plain",
            b"hello".to_vec(),
        )
        .await
        .expect("upload document attachment");

    let task_comment = owner
        .acta()
        .add_comment(
            ws_slug,
            &task.readable_id,
            CreateCommentRequest::published("Sweep task comment"),
        )
        .await
        .expect("create task comment");

    let task_comment_attachment = owner
        .acta()
        .upload_task_comment_attachment(
            ws_slug,
            &task.readable_id,
            task_comment.id,
            "sweep-task-comment.txt",
            "text/plain",
            b"hello".to_vec(),
        )
        .await
        .expect("upload task comment attachment");

    let document_comment = owner
        .acta()
        .add_document_comment(
            ws_slug,
            &document.id.to_string(),
            CreateCommentRequest::published("Sweep document comment"),
        )
        .await
        .expect("create document comment");

    let document_comment_attachment = owner
        .acta()
        .upload_document_comment_attachment(
            ws_slug,
            &document.id.to_string(),
            document_comment.id,
            "sweep-document-comment.txt",
            "text/plain",
            b"hello".to_vec(),
        )
        .await
        .expect("upload document comment attachment");

    let folder = owner
        .acta()
        .create_folder(
            ws_slug,
            &project.slug,
            CreateFolderRequest {
                name: "Sweep Folder".into(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create folder");

    // Seed a real webhook so the get/update/delete/deliveries webhook cases
    // resolve a concrete `webhook_id` in the positive pass instead of 404-ing.
    // The stored secret is never decrypted by the read/list/delete handlers, so
    // a dummy crypto key is sufficient here.
    let crypto = WebhookCrypto::new(&[0x42u8; 32]);
    let (enc, nonce) = crypto
        .encrypt(b"test-hmac-secret-32-bytes-dummy!")
        .expect("encrypt webhook secret");
    let webhook = PgWebhookSubscriptionRepo::create(
        db.conn(),
        ws_id,
        "https://example.com/sweep-hook".to_string(),
        vec!["task.created".to_string()],
        "workspace".to_string(),
        None,
        enc,
        nonce,
        None,
        &Actor::User(atlas_acta::actor::UserAttributionId(user_id.0)),
    )
    .await
    .expect("create sweep webhook");

    Fixtures {
        ws_slug: ws_slug.to_string(),
        project_slug: project.slug,
        board_id: board.id,
        task_readable_id: task.readable_id,
        document_ref: document.id.to_string(),
        document_head_revision_id: document.head_revision_id,
        folder_id: folder.id,
        doc_attachment_id: attachment.id,
        task_comment_id: task_comment.id,
        task_comment_attachment_id: task_comment_attachment.id,
        document_comment_id: document_comment.id,
        document_comment_attachment_id: document_comment_attachment.id,
        webhook_id: webhook.id,
    }
}

/// Creates a global agent key (so it inherits the owner's Editor+ reach in
/// every resource without needing per-resource grants) with the given scope
/// set, and returns the plaintext bearer token.
async fn create_scoped_agent(
    db: &TestDb,
    owner_user_id: atlas_core::principal::UserId,
    name: &str,
    scopes: Vec<Capability>,
) -> String {
    let plain = format!("atlas_{name}_secret_{}", uuid::Uuid::now_v7().as_simple());
    let hash = atlas_server::auth::tokens::hash_token(&plain);

    let key = db
        .api_key_repo()
        .create_for_user(
            owner_user_id,
            NewApiKey {
                name: name.to_string(),
                token_hash: hash,
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes,
            },
        )
        .await
        .expect("create scoped api key");

    PgApiKeyRepo::set_global_for_user_in(db.conn(), owner_user_id, key.id, true)
        .await
        .expect("make key global");

    plain
}

#[derive(Debug, Clone, Copy)]
#[allow(clippy::enum_variant_names)]
enum Case {
    // ---- tasks (31) ----
    CreateTask,
    ListTasks,
    ListWorkspaceTasks,
    GetTask,
    UpdateTask,
    DeleteTask,
    MoveTask,
    ListAssignees,
    AddAssignee,
    RemoveAssignee,
    ListReferences,
    CreateReference,
    CreateReferencesBatch,
    DeleteReference,
    UploadTaskAttachment,
    ListTaskAttachments,
    DownloadTaskAttachment,
    RenameTaskAttachment,
    DeleteTaskAttachment,
    ListTaskBacklinks,
    GetTaskGraph,
    ListChecklist,
    CreateChecklistItem,
    UpdateChecklistItem,
    DeleteChecklistItem,
    PromoteChecklistItem,
    ListSubtasks,
    CreateSubtask,
    PromoteSubtask,
    SetTaskParent,
    ListActivity,
    ListTaskComments,
    AddTaskComment,
    UpdateTaskComment,
    DeleteTaskComment,
    CreateTaskCommentDraft,
    CancelTaskCommentDraft,
    UploadTaskCommentDraftAttachment,
    UploadTaskCommentAttachment,
    ListTaskCommentAttachments,
    DownloadTaskCommentAttachment,
    DeleteTaskCommentAttachment,
    // ---- docs (22) ----
    CreateDocument,
    ListDocuments,
    GetDocument,
    GetDocumentCompact,
    GetDocumentRange,
    SearchDocumentContent,
    UpdateDocument,
    EditDocumentContent,
    DeleteDocument,
    UpdateContent,
    ListDocumentHistory,
    GetRevisionContent,
    ListDocBacklinks,
    GetFrontmatter,
    UploadDocAttachment,
    ListDocAttachments,
    DownloadDocAttachment,
    DeleteDocAttachment,
    MoveDocument,
    CopyDocument,
    ListDocComments,
    AddDocComment,
    UpdateDocComment,
    DeleteDocComment,
    CreateDocumentCommentDraft,
    CancelDocumentCommentDraft,
    UploadDocumentCommentAttachment,
    UploadDocumentCommentDraftAttachment,
    ListDocumentCommentAttachments,
    DownloadDocumentCommentAttachment,
    DeleteDocumentCommentAttachment,
    DocumentHeartbeat,
    DocumentLeave,
    MoveDocumentsBatch,
    // ---- boards (13) ----
    CreateBoard,
    ListBoards,
    GetBoard,
    UpdateBoard,
    MoveBoard,
    ArchiveBoard,
    UnarchiveBoard,
    DeleteBoard,
    CreateColumn,
    ListColumns,
    UpdateColumn,
    DeleteColumn,
    ApplyStatusTemplates,
    BoardHeartbeat,
    BoardLeave,
    ListStatusTemplates,
    CreateStatusTemplate,
    UpdateStatusTemplate,
    DeleteStatusTemplate,
    // ---- folders (7) ----
    CreateFolder,
    ListFolders,
    GetFolder,
    RenameFolder,
    MoveFolder,
    CopyFolder,
    DeleteFolder,
    // ---- projects (5) ----
    CreateProject,
    ListProjects,
    GetProject,
    UpdateProject,
    DeleteProject,
    // ---- webhooks (6) ----
    CreateWebhook,
    ListWebhooks,
    GetWebhook,
    UpdateWebhook,
    DeleteWebhook,
    ListWebhookDeliveries,
    // ---- config: tags + property-definitions (8) ----
    ListTags,
    CreateTag,
    ListUsedLabels,
    UpdateTag,
    DeleteTag,
    ListPropertyDefinitions,
    CreatePropertyDefinition,
    DeletePropertyDefinition,
    // ---- config: workspace rename (1) ----
    RenameWorkspace,
    // ---- config: semantic reindex (2) ----
    SemanticReindexPlan,
    SemanticReindexStart,
    // ---- grants: read-only list reads (2) ----
    ListProjectGrants,
    ListWorkspaceGrants,
    // ---- saved_searches (4) ----
    ListSavedSearches,
    CreateSavedSearch,
    RenameSavedSearch,
    DeleteSavedSearch,
    // ---- task_views (5) ----
    ListTaskViews,
    CreateTaskView,
    GetTaskView,
    UpdateTaskView,
    DeleteTaskView,
}

impl Case {
    const ALL: &'static [Case] = &[
        Case::CreateTask,
        Case::ListTasks,
        Case::ListWorkspaceTasks,
        Case::GetTask,
        Case::UpdateTask,
        Case::DeleteTask,
        Case::MoveTask,
        Case::ListAssignees,
        Case::AddAssignee,
        Case::RemoveAssignee,
        Case::ListReferences,
        Case::CreateReference,
        Case::CreateReferencesBatch,
        Case::DeleteReference,
        Case::UploadTaskAttachment,
        Case::ListTaskAttachments,
        Case::DownloadTaskAttachment,
        Case::RenameTaskAttachment,
        Case::DeleteTaskAttachment,
        Case::ListTaskBacklinks,
        Case::GetTaskGraph,
        Case::ListChecklist,
        Case::CreateChecklistItem,
        Case::UpdateChecklistItem,
        Case::DeleteChecklistItem,
        Case::PromoteChecklistItem,
        Case::ListSubtasks,
        Case::CreateSubtask,
        Case::PromoteSubtask,
        Case::SetTaskParent,
        Case::ListActivity,
        Case::ListTaskComments,
        Case::AddTaskComment,
        Case::UpdateTaskComment,
        Case::DeleteTaskComment,
        Case::CreateTaskCommentDraft,
        Case::CancelTaskCommentDraft,
        Case::UploadTaskCommentDraftAttachment,
        Case::UploadTaskCommentAttachment,
        Case::ListTaskCommentAttachments,
        Case::DownloadTaskCommentAttachment,
        Case::DeleteTaskCommentAttachment,
        Case::CreateDocument,
        Case::ListDocuments,
        Case::GetDocument,
        Case::GetDocumentCompact,
        Case::GetDocumentRange,
        Case::SearchDocumentContent,
        Case::UpdateDocument,
        Case::EditDocumentContent,
        Case::DeleteDocument,
        Case::UpdateContent,
        Case::ListDocumentHistory,
        Case::GetRevisionContent,
        Case::ListDocBacklinks,
        Case::GetFrontmatter,
        Case::UploadDocAttachment,
        Case::ListDocAttachments,
        Case::DownloadDocAttachment,
        Case::DeleteDocAttachment,
        Case::MoveDocument,
        Case::CopyDocument,
        Case::ListDocComments,
        Case::AddDocComment,
        Case::UpdateDocComment,
        Case::DeleteDocComment,
        Case::CreateDocumentCommentDraft,
        Case::CancelDocumentCommentDraft,
        Case::UploadDocumentCommentAttachment,
        Case::UploadDocumentCommentDraftAttachment,
        Case::ListDocumentCommentAttachments,
        Case::DownloadDocumentCommentAttachment,
        Case::DeleteDocumentCommentAttachment,
        Case::DocumentHeartbeat,
        Case::DocumentLeave,
        Case::MoveDocumentsBatch,
        Case::CreateBoard,
        Case::ListBoards,
        Case::GetBoard,
        Case::UpdateBoard,
        Case::MoveBoard,
        Case::ArchiveBoard,
        Case::UnarchiveBoard,
        Case::DeleteBoard,
        Case::CreateColumn,
        Case::ListColumns,
        Case::UpdateColumn,
        Case::DeleteColumn,
        Case::ApplyStatusTemplates,
        Case::BoardHeartbeat,
        Case::BoardLeave,
        Case::ListStatusTemplates,
        Case::CreateStatusTemplate,
        Case::UpdateStatusTemplate,
        Case::DeleteStatusTemplate,
        Case::CreateFolder,
        Case::ListFolders,
        Case::GetFolder,
        Case::RenameFolder,
        Case::MoveFolder,
        Case::CopyFolder,
        Case::DeleteFolder,
        Case::CreateProject,
        Case::ListProjects,
        Case::GetProject,
        Case::UpdateProject,
        Case::DeleteProject,
        Case::CreateWebhook,
        Case::ListWebhooks,
        Case::GetWebhook,
        Case::UpdateWebhook,
        Case::DeleteWebhook,
        Case::ListWebhookDeliveries,
        Case::ListTags,
        Case::CreateTag,
        Case::ListUsedLabels,
        Case::UpdateTag,
        Case::DeleteTag,
        Case::ListPropertyDefinitions,
        Case::CreatePropertyDefinition,
        Case::DeletePropertyDefinition,
        Case::RenameWorkspace,
        Case::SemanticReindexPlan,
        Case::SemanticReindexStart,
        Case::ListProjectGrants,
        Case::ListWorkspaceGrants,
        Case::ListSavedSearches,
        Case::CreateSavedSearch,
        Case::RenameSavedSearch,
        Case::DeleteSavedSearch,
        Case::ListTaskViews,
        Case::CreateTaskView,
        Case::GetTaskView,
        Case::UpdateTaskView,
        Case::DeleteTaskView,
    ];

    /// `(method, capability)` as declared for this case's route — cross-checked
    /// against the live REG-5 registry in
    /// `capability_sweep_covers_every_registry_entry` so this list can never
    /// silently drift from it.
    fn registry_key(self) -> (&'static str, &'static str) {
        match self {
            Case::CreateTask => ("POST", "tasks:create"),
            Case::ListTasks => ("GET", "tasks:read"),
            Case::ListWorkspaceTasks => ("GET", "tasks:read"),
            Case::GetTask => ("GET", "tasks:read"),
            Case::UpdateTask => ("PATCH", "tasks:update"),
            Case::DeleteTask => ("DELETE", "tasks:delete"),
            Case::MoveTask => ("POST", "tasks:update"),
            Case::ListAssignees => ("GET", "tasks:read"),
            Case::AddAssignee => ("POST", "tasks:update"),
            Case::RemoveAssignee => ("DELETE", "tasks:update"),
            Case::ListReferences => ("GET", "tasks:read"),
            Case::CreateReference => ("POST", "tasks:update"),
            Case::CreateReferencesBatch => ("POST", "tasks:update"),
            Case::DeleteReference => ("DELETE", "tasks:update"),
            Case::UploadTaskAttachment => ("POST", "tasks:update"),
            Case::ListTaskAttachments => ("GET", "tasks:read"),
            Case::DownloadTaskAttachment => ("GET", "tasks:read"),
            Case::RenameTaskAttachment => ("PATCH", "tasks:update"),
            Case::DeleteTaskAttachment => ("DELETE", "tasks:update"),
            Case::ListTaskBacklinks => ("GET", "tasks:read"),
            Case::GetTaskGraph => ("GET", "tasks:read"),
            Case::ListChecklist => ("GET", "tasks:read"),
            Case::CreateChecklistItem => ("POST", "tasks:update"),
            Case::UpdateChecklistItem => ("PATCH", "tasks:update"),
            Case::DeleteChecklistItem => ("DELETE", "tasks:update"),
            Case::PromoteChecklistItem => ("POST", "tasks:create"),
            Case::ListSubtasks => ("GET", "tasks:read"),
            Case::CreateSubtask => ("POST", "tasks:create"),
            Case::PromoteSubtask => ("POST", "tasks:update"),
            Case::SetTaskParent => ("POST", "tasks:update"),
            Case::ListActivity => ("GET", "tasks:read"),
            Case::ListTaskComments => ("GET", "tasks:read"),
            Case::AddTaskComment => ("POST", "tasks:update"),
            Case::UpdateTaskComment => ("PATCH", "tasks:update"),
            Case::DeleteTaskComment => ("DELETE", "tasks:update"),
            Case::CreateTaskCommentDraft => ("POST", "tasks:update"),
            Case::CancelTaskCommentDraft => ("DELETE", "tasks:update"),
            Case::UploadTaskCommentDraftAttachment => ("POST", "tasks:update"),
            Case::UploadTaskCommentAttachment => ("POST", "tasks:update"),
            Case::ListTaskCommentAttachments => ("GET", "tasks:read"),
            Case::DownloadTaskCommentAttachment => ("GET", "tasks:read"),
            Case::DeleteTaskCommentAttachment => ("DELETE", "tasks:update"),

            Case::CreateDocument => ("POST", "docs:create"),
            Case::ListDocuments => ("GET", "docs:read"),
            Case::GetDocument => ("GET", "docs:read"),
            Case::GetDocumentCompact => ("GET", "docs:read"),
            Case::GetDocumentRange => ("GET", "docs:read"),
            Case::SearchDocumentContent => ("POST", "docs:read"),
            Case::UpdateDocument => ("PATCH", "docs:update"),
            Case::EditDocumentContent => ("PATCH", "docs:update"),
            Case::DeleteDocument => ("DELETE", "docs:delete"),
            Case::UpdateContent => ("PUT", "docs:update"),
            Case::ListDocumentHistory => ("GET", "docs:read"),
            Case::GetRevisionContent => ("GET", "docs:read"),
            Case::ListDocBacklinks => ("GET", "docs:read"),
            Case::GetFrontmatter => ("GET", "docs:read"),
            Case::UploadDocAttachment => ("POST", "docs:update"),
            Case::ListDocAttachments => ("GET", "docs:read"),
            Case::DownloadDocAttachment => ("GET", "docs:read"),
            Case::DeleteDocAttachment => ("DELETE", "docs:update"),
            Case::MoveDocument => ("PATCH", "docs:update"),
            Case::CopyDocument => ("POST", "docs:create"),
            Case::ListDocComments => ("GET", "docs:read"),
            Case::AddDocComment => ("POST", "docs:update"),
            Case::UpdateDocComment => ("PATCH", "docs:update"),
            Case::DeleteDocComment => ("DELETE", "docs:update"),
            Case::CreateDocumentCommentDraft => ("POST", "docs:update"),
            Case::CancelDocumentCommentDraft => ("DELETE", "docs:update"),
            Case::UploadDocumentCommentAttachment => ("POST", "docs:update"),
            Case::UploadDocumentCommentDraftAttachment => ("POST", "docs:update"),
            Case::ListDocumentCommentAttachments => ("GET", "docs:read"),
            Case::DownloadDocumentCommentAttachment => ("GET", "docs:read"),
            Case::DeleteDocumentCommentAttachment => ("DELETE", "docs:update"),
            Case::DocumentHeartbeat => ("POST", "docs:read"),
            Case::DocumentLeave => ("DELETE", "docs:read"),
            Case::MoveDocumentsBatch => ("POST", "docs:update"),

            Case::CreateBoard => ("POST", "boards:create"),
            Case::ListBoards => ("GET", "boards:read"),
            Case::GetBoard => ("GET", "boards:read"),
            Case::UpdateBoard => ("PATCH", "boards:update"),
            Case::MoveBoard => ("PATCH", "boards:update"),
            Case::ArchiveBoard => ("POST", "boards:update"),
            Case::UnarchiveBoard => ("POST", "boards:update"),
            Case::DeleteBoard => ("DELETE", "boards:delete"),
            Case::CreateColumn => ("POST", "boards:update"),
            Case::ListColumns => ("GET", "boards:read"),
            Case::UpdateColumn => ("PATCH", "boards:update"),
            Case::DeleteColumn => ("DELETE", "boards:update"),
            Case::ApplyStatusTemplates => ("POST", "boards:update"),
            Case::BoardHeartbeat => ("POST", "boards:read"),
            Case::BoardLeave => ("DELETE", "boards:read"),
            Case::ListStatusTemplates => ("GET", "boards:read"),
            Case::CreateStatusTemplate => ("POST", "boards:create"),
            Case::UpdateStatusTemplate => ("PATCH", "boards:update"),
            Case::DeleteStatusTemplate => ("DELETE", "boards:delete"),

            Case::CreateFolder => ("POST", "folders:create"),
            Case::ListFolders => ("GET", "folders:read"),
            Case::GetFolder => ("GET", "folders:read"),
            Case::RenameFolder => ("PATCH", "folders:update"),
            Case::MoveFolder => ("PATCH", "folders:update"),
            Case::CopyFolder => ("POST", "folders:create"),
            Case::DeleteFolder => ("DELETE", "folders:delete"),

            Case::CreateProject => ("POST", "projects:create"),
            Case::ListProjects => ("GET", "projects:read"),
            Case::GetProject => ("GET", "projects:read"),
            Case::UpdateProject => ("PATCH", "projects:update"),
            Case::DeleteProject => ("DELETE", "projects:delete"),

            Case::CreateWebhook => ("POST", "webhooks:create"),
            Case::ListWebhooks => ("GET", "webhooks:read"),
            Case::GetWebhook => ("GET", "webhooks:read"),
            Case::UpdateWebhook => ("PATCH", "webhooks:update"),
            Case::DeleteWebhook => ("DELETE", "webhooks:delete"),
            Case::ListWebhookDeliveries => ("GET", "webhooks:read"),

            Case::ListTags => ("GET", "config:read"),
            Case::CreateTag => ("POST", "config:create"),
            Case::ListUsedLabels => ("GET", "config:read"),
            Case::UpdateTag => ("PATCH", "config:update"),
            Case::DeleteTag => ("DELETE", "config:delete"),
            Case::ListPropertyDefinitions => ("GET", "config:read"),
            Case::CreatePropertyDefinition => ("POST", "config:create"),
            Case::DeletePropertyDefinition => ("DELETE", "config:delete"),

            Case::RenameWorkspace => ("PATCH", "config:update"),
            Case::SemanticReindexPlan => ("GET", "config:read"),
            Case::SemanticReindexStart => ("POST", "config:update"),

            Case::ListProjectGrants => ("GET", "grants:read"),
            Case::ListWorkspaceGrants => ("GET", "grants:read"),

            Case::ListSavedSearches => ("GET", "saved_searches:read"),
            Case::CreateSavedSearch => ("POST", "saved_searches:create"),
            Case::RenameSavedSearch => ("PATCH", "saved_searches:update"),
            Case::DeleteSavedSearch => ("DELETE", "saved_searches:delete"),

            Case::ListTaskViews => ("GET", "task_views:read"),
            Case::CreateTaskView => ("POST", "task_views:create"),
            Case::GetTaskView => ("GET", "task_views:read"),
            Case::UpdateTaskView => ("PATCH", "task_views:update"),
            Case::DeleteTaskView => ("DELETE", "task_views:delete"),
        }
    }
}

/// Executes one sweep case against `client`, mapping every success payload to
/// `()` so callers only need to inspect the pass/fail shape.
#[allow(clippy::too_many_lines)]
async fn invoke(
    case: Case,
    client: &AtlasClient,
    http: &reqwest::Client,
    base_url: &str,
    token: &str,
    fx: &Fixtures,
) -> Result<(), ClientError> {
    let ws = fx.ws_slug.as_str();
    let nil = uuid::Uuid::nil();

    match case {
        Case::CreateTask => client
            .acta()
            .create_task(
                ws,
                fx.board_id,
                CreateTaskRequest {
                    references: vec![],
                    column_id: nil,
                    title: "x".into(),
                    description: None,
                    properties: None,
                    before: None,
                    after: None,
                },
            )
            .await
            .map(|_| ()),
        Case::ListTasks => client
            .acta()
            .list_tasks(ws, fx.board_id, None, None)
            .await
            .map(|_| ()),
        Case::ListWorkspaceTasks => client
            .acta()
            .list_workspace_tasks(ws, &WorkspaceTaskQueryParams::default())
            .await
            .map(|_| ()),
        Case::GetTask => client
            .acta()
            .get_task(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::UpdateTask => client
            .acta()
            .update_task(ws, &fx.task_readable_id, UpdateTaskRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteTask => client.acta().delete_task(ws, &fx.task_readable_id).await,
        Case::MoveTask => client
            .acta()
            .move_task(
                ws,
                &fx.task_readable_id,
                MoveTaskRequest {
                    column_id: nil,
                    before: None,
                    after: None,
                },
            )
            .await
            .map(|_| ()),
        Case::ListAssignees => client
            .acta()
            .list_assignees(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::AddAssignee => client
            .acta()
            .add_assignee(
                ws,
                &fx.task_readable_id,
                AddAssigneeRequest {
                    assignee_type: "user".into(),
                    assignee_id: nil,
                },
            )
            .await
            .map(|_| ()),
        Case::RemoveAssignee => {
            client
                .acta()
                .remove_assignee(ws, &fx.task_readable_id, &format!("user:{nil}"))
                .await
        }
        Case::ListReferences => client
            .acta()
            .list_references(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::CreateReference => client
            .acta()
            .create_reference(
                ws,
                &fx.task_readable_id,
                CreateReferenceRequest {
                    kind: "relates".into(),
                    target_task_readable_id: Some("CSW-nonexistent".into()),
                    target_document_id: None,
                },
            )
            .await
            .map(|_| ()),
        Case::CreateReferencesBatch => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/tasks/{}/references/batch",
                        fx.task_readable_id
                    ),
                ),
            )
            .await
        }
        Case::DeleteReference => {
            client
                .acta()
                .delete_reference(ws, &fx.task_readable_id, nil)
                .await
        }
        Case::UploadTaskAttachment => client
            .acta()
            .upload_task_attachment(
                ws,
                &fx.task_readable_id,
                "f.txt",
                "text/plain",
                vec![1, 2, 3],
            )
            .await
            .map(|_| ()),
        Case::ListTaskAttachments => client
            .acta()
            .list_task_attachments(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::DownloadTaskAttachment => client
            .acta()
            .download_task_attachment(ws, &fx.task_readable_id, nil)
            .await
            .map(|_| ()),
        Case::RenameTaskAttachment => client
            .acta()
            .rename_task_attachment(
                ws,
                &fx.task_readable_id,
                nil,
                RenameTaskAttachmentRequest {
                    file_name: "renamed.txt".to_string(),
                },
            )
            .await
            .map(|_| ()),
        Case::DeleteTaskAttachment => {
            client
                .acta()
                .delete_task_attachment(ws, &fx.task_readable_id, nil)
                .await
        }
        Case::ListTaskBacklinks => client
            .acta()
            .list_task_backlinks(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::GetTaskGraph => client
            .acta()
            .get_task_graph(ws, &fx.task_readable_id, None)
            .await
            .map(|_| ()),
        Case::ListChecklist => client
            .acta()
            .list_checklist(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::CreateChecklistItem => client
            .acta()
            .create_checklist_item(
                ws,
                &fx.task_readable_id,
                CreateChecklistItemRequest {
                    title: "x".into(),
                    before: None,
                    after: None,
                },
            )
            .await
            .map(|_| ()),
        Case::UpdateChecklistItem => client
            .acta()
            .update_checklist_item(
                ws,
                &fx.task_readable_id,
                nil,
                UpdateChecklistItemRequest::default(),
            )
            .await
            .map(|_| ()),
        Case::DeleteChecklistItem => {
            client
                .acta()
                .delete_checklist_item(ws, &fx.task_readable_id, nil)
                .await
        }
        Case::PromoteChecklistItem => client
            .acta()
            .promote_checklist_item(
                ws,
                &fx.task_readable_id,
                nil,
                PromoteChecklistItemRequest {
                    board_id: fx.board_id,
                    column_id: nil,
                },
            )
            .await
            .map(|_| ()),
        Case::ListSubtasks => client
            .acta()
            .list_subtasks(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::CreateSubtask => client
            .acta()
            .create_subtask(ws, &fx.task_readable_id, CreateSubtaskRequest::titled("x"))
            .await
            .map(|_| ()),
        Case::PromoteSubtask => client
            .acta()
            .promote_subtask(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::SetTaskParent => client
            .acta()
            .set_task_parent(
                ws,
                &fx.task_readable_id,
                SetTaskParentRequest {
                    parent_readable_id: fx.task_readable_id.clone(),
                },
            )
            .await
            .map(|_| ()),
        Case::ListActivity => client
            .acta()
            .list_activity(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::ListTaskComments => client
            .acta()
            .list_comments(ws, &fx.task_readable_id, None, None)
            .await
            .map(|_| ()),
        Case::AddTaskComment => client
            .acta()
            .add_comment(
                ws,
                &fx.task_readable_id,
                CreateCommentRequest::published("x"),
            )
            .await
            .map(|_| ()),
        Case::UpdateTaskComment => client
            .acta()
            .update_comment(
                ws,
                &fx.task_readable_id,
                nil,
                UpdateCommentRequest { body: "x".into() },
            )
            .await
            .map(|_| ()),
        Case::DeleteTaskComment => {
            client
                .acta()
                .delete_comment(ws, &fx.task_readable_id, nil)
                .await
        }
        Case::CreateTaskCommentDraft => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/tasks/{}/comment-drafts",
                        fx.task_readable_id
                    ),
                ),
            )
            .await
        }
        Case::CancelTaskCommentDraft => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/tasks/{}/comment-drafts/{nil}",
                        fx.task_readable_id
                    ),
                ),
            )
            .await
        }
        Case::UploadTaskCommentDraftAttachment => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/tasks/{}/comment-drafts/{nil}/attachments",
                        fx.task_readable_id
                    ),
                ),
            )
            .await
        }
        Case::UploadTaskCommentAttachment => client
            .acta()
            .upload_task_comment_attachment(
                ws,
                &fx.task_readable_id,
                fx.task_comment_id,
                "f.txt",
                "text/plain",
                vec![1, 2, 3],
            )
            .await
            .map(|_| ()),
        Case::ListTaskCommentAttachments => client
            .acta()
            .list_task_comment_attachments(ws, &fx.task_readable_id, fx.task_comment_id)
            .await
            .map(|_| ()),
        Case::DownloadTaskCommentAttachment => client
            .acta()
            .download_task_comment_attachment(
                ws,
                &fx.task_readable_id,
                fx.task_comment_id,
                fx.task_comment_attachment_id,
            )
            .await
            .map(|_| ()),
        Case::DeleteTaskCommentAttachment => {
            client
                .acta()
                .delete_task_comment_attachment(
                    ws,
                    &fx.task_readable_id,
                    fx.task_comment_id,
                    fx.task_comment_attachment_id,
                )
                .await
        }

        Case::CreateDocument => client
            .acta()
            .create_document(
                ws,
                &fx.project_slug,
                CreateDocumentRequest {
                    title: "x".into(),
                    folder_id: None,
                    content: None,
                },
            )
            .await
            .map(|_| ()),
        Case::ListDocuments => client
            .acta()
            .list_documents(ws, &fx.project_slug, None, None)
            .await
            .map(|_| ()),
        Case::GetDocument => client
            .acta()
            .get_document(ws, &fx.document_ref)
            .await
            .map(|_| ()),
        Case::GetDocumentCompact => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/documents/{}/compact", fx.document_ref),
                ),
            )
            .await
        }
        Case::GetDocumentRange => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/documents/{}/content/range",
                        fx.document_ref
                    ),
                ),
            )
            .await
        }
        Case::SearchDocumentContent => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/documents/{}/content/search",
                        fx.document_ref
                    ),
                ),
            )
            .await
        }
        Case::UpdateDocument => client
            .acta()
            .update_document(ws, &fx.document_ref, UpdateDocumentRequest::default())
            .await
            .map(|_| ()),
        Case::EditDocumentContent => {
            raw_call(
                http,
                base_url,
                token,
                "PATCH",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/documents/{}/content/range",
                        fx.document_ref
                    ),
                ),
            )
            .await
        }
        Case::DeleteDocument => client.acta().delete_document(ws, &fx.document_ref).await,
        Case::UpdateContent => client
            .acta()
            .update_content(
                ws,
                &fx.document_ref,
                UpdateContentRequest {
                    content: "x".into(),
                    base_revision_id: nil,
                },
            )
            .await
            .map(|_| ()),
        Case::ListDocumentHistory => client
            .acta()
            .list_document_history(ws, &fx.document_ref, None, None)
            .await
            .map(|_| ()),
        Case::GetRevisionContent => client
            .acta()
            .get_revision_content(ws, &fx.document_ref, 1)
            .await
            .map(|_| ()),
        Case::ListDocBacklinks => client
            .acta()
            .list_backlinks(ws, &fx.document_ref, None, None)
            .await
            .map(|_| ()),
        Case::GetFrontmatter => client
            .acta()
            .get_frontmatter(ws, &fx.document_ref)
            .await
            .map(|_| ()),
        Case::UploadDocAttachment => client
            .acta()
            .upload_attachment(ws, &fx.document_ref, "f.txt", "text/plain", vec![1, 2, 3])
            .await
            .map(|_| ()),
        Case::ListDocAttachments => client
            .acta()
            .list_attachments(ws, &fx.document_ref, None, None)
            .await
            .map(|_| ()),
        Case::DownloadDocAttachment => client
            .acta()
            .download_attachment(ws, fx.doc_attachment_id)
            .await
            .map(|_| ()),
        Case::DeleteDocAttachment => {
            client
                .acta()
                .delete_attachment(ws, fx.doc_attachment_id)
                .await
        }
        Case::MoveDocument => client
            .acta()
            .move_document(ws, &fx.document_ref, MoveDocumentRequest::default())
            .await
            .map(|_| ()),
        Case::CopyDocument => client
            .acta()
            .copy_document(ws, &fx.document_ref, None)
            .await
            .map(|_| ()),
        Case::ListDocComments => client
            .acta()
            .list_document_comments(ws, &fx.document_ref, None, None)
            .await
            .map(|_| ()),
        Case::AddDocComment => client
            .acta()
            .add_document_comment(ws, &fx.document_ref, CreateCommentRequest::published("x"))
            .await
            .map(|_| ()),
        Case::UpdateDocComment => client
            .acta()
            .update_document_comment(
                ws,
                &fx.document_ref,
                nil,
                UpdateCommentRequest { body: "x".into() },
            )
            .await
            .map(|_| ()),
        Case::DeleteDocComment => {
            client
                .acta()
                .delete_document_comment(ws, &fx.document_ref, nil)
                .await
        }
        Case::CreateDocumentCommentDraft => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/documents/{}/comment-drafts",
                        fx.document_ref
                    ),
                ),
            )
            .await
        }
        Case::CancelDocumentCommentDraft => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/documents/{}/comment-drafts/{nil}",
                        fx.document_ref
                    ),
                ),
            )
            .await
        }
        Case::UploadDocumentCommentAttachment => client
            .acta()
            .upload_document_comment_attachment(
                ws,
                &fx.document_ref,
                fx.document_comment_id,
                "f.txt",
                "text/plain",
                vec![1, 2, 3],
            )
            .await
            .map(|_| ()),
        Case::UploadDocumentCommentDraftAttachment => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{ws}/documents/{}/comment-drafts/{nil}/attachments",
                        fx.document_ref
                    ),
                ),
            )
            .await
        }
        Case::ListDocumentCommentAttachments => client
            .acta()
            .list_document_comment_attachments(ws, &fx.document_ref, fx.document_comment_id)
            .await
            .map(|_| ()),
        Case::DownloadDocumentCommentAttachment => client
            .acta()
            .download_document_comment_attachment(
                ws,
                &fx.document_ref,
                fx.document_comment_id,
                fx.document_comment_attachment_id,
            )
            .await
            .map(|_| ()),
        Case::DeleteDocumentCommentAttachment => {
            client
                .acta()
                .delete_document_comment_attachment(
                    ws,
                    &fx.document_ref,
                    fx.document_comment_id,
                    fx.document_comment_attachment_id,
                )
                .await
        }
        Case::DocumentHeartbeat => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/documents/{}/presence", fx.document_ref),
                ),
            )
            .await
        }
        Case::DocumentLeave => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/documents/{}/presence", fx.document_ref),
                ),
            )
            .await
        }
        Case::MoveDocumentsBatch => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/documents/moves/batch"),
                ),
            )
            .await
        }

        Case::CreateBoard => client
            .acta()
            .create_board(
                ws,
                &fx.project_slug,
                CreateBoardRequest {
                    folder_id: None,
                    name: "x".into(),
                },
            )
            .await
            .map(|_| ()),
        Case::ListBoards => client
            .acta()
            .list_boards(ws, &fx.project_slug, None, None)
            .await
            .map(|_| ()),
        Case::GetBoard => client.acta().get_board(ws, fx.board_id).await.map(|_| ()),
        Case::UpdateBoard => client
            .acta()
            .update_board(ws, fx.board_id, UpdateBoardRequest::default())
            .await
            .map(|_| ()),
        Case::MoveBoard => client
            .acta()
            .move_board(ws, fx.board_id, MoveBoardRequest::default())
            .await
            .map(|_| ()),
        // Unarchive runs first so the board is left writable for every later
        // case in the sweep, whatever order they run in.
        Case::UnarchiveBoard => client
            .acta()
            .unarchive_board(ws, fx.board_id)
            .await
            .map(|_| ()),
        Case::ArchiveBoard => {
            let archived = client
                .acta()
                .archive_board(ws, fx.board_id)
                .await
                .map(|_| ());
            let _ = client.acta().unarchive_board(ws, fx.board_id).await;
            archived
        }
        Case::DeleteBoard => client.acta().delete_board(ws, fx.board_id).await,
        Case::CreateColumn => client
            .acta()
            .create_column(
                ws,
                fx.board_id,
                CreateColumnRequest {
                    name: "x".into(),
                    color: None,
                    before: None,
                    after: None,
                },
            )
            .await
            .map(|_| ()),
        Case::ListColumns => client
            .acta()
            .list_columns(ws, fx.board_id)
            .await
            .map(|_| ()),
        Case::UpdateColumn => client
            .acta()
            .update_column(ws, fx.board_id, nil, UpdateColumnRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteColumn => client.acta().delete_column(ws, fx.board_id, nil).await,
        Case::ApplyStatusTemplates => client
            .acta()
            .apply_status_templates(ws, fx.board_id)
            .await
            .map(|_| ()),
        Case::BoardHeartbeat => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/boards/{}/presence", fx.board_id),
                ),
            )
            .await
        }
        Case::BoardLeave => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/boards/{}/presence", fx.board_id),
                ),
            )
            .await
        }
        Case::ListStatusTemplates => client.acta().list_status_templates(ws).await.map(|_| ()),
        Case::CreateStatusTemplate => client
            .acta()
            .create_status_template(
                ws,
                CreateStatusTemplateRequest {
                    name: "x".into(),
                    color: None,
                    before: None,
                    after: None,
                },
            )
            .await
            .map(|_| ()),
        Case::UpdateStatusTemplate => client
            .acta()
            .update_status_template(ws, nil, UpdateStatusTemplateRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteStatusTemplate => client.acta().delete_status_template(ws, nil).await,

        Case::CreateFolder => client
            .acta()
            .create_folder(
                ws,
                &fx.project_slug,
                CreateFolderRequest {
                    name: "x".into(),
                    parent_folder_id: None,
                },
            )
            .await
            .map(|_| ()),
        Case::ListFolders => client
            .acta()
            .list_folders(ws, &fx.project_slug, None, None)
            .await
            .map(|_| ()),
        Case::GetFolder => client.acta().get_folder(ws, fx.folder_id).await.map(|_| ()),
        Case::RenameFolder => client
            .acta()
            .rename_folder(ws, fx.folder_id, RenameFolderRequest { name: "y".into() })
            .await
            .map(|_| ()),
        Case::MoveFolder => client
            .acta()
            .move_folder(
                ws,
                fx.folder_id,
                MoveFolderRequest {
                    parent_folder_id: None,
                },
            )
            .await
            .map(|_| ()),
        Case::CopyFolder => client
            .acta()
            .copy_folder(ws, fx.folder_id, None)
            .await
            .map(|_| ()),
        Case::DeleteFolder => client.acta().delete_folder(ws, fx.folder_id).await,

        Case::CreateProject => client
            .acta()
            .create_project(
                ws,
                CreateProjectRequest {
                    name: "x".into(),
                    slug: format!("proj-{}", uuid::Uuid::now_v7().as_simple()),
                    task_prefix: "XPX".into(),
                    visibility: None,
                    visibility_role: None,
                },
            )
            .await
            .map(|_| ()),
        Case::ListProjects => client
            .acta()
            .list_projects(ws, None, None)
            .await
            .map(|_| ()),
        Case::GetProject => client
            .acta()
            .get_project(ws, &fx.project_slug)
            .await
            .map(|_| ()),
        Case::UpdateProject => client
            .acta()
            .update_project(ws, &fx.project_slug, UpdateProjectRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteProject => client.acta().delete_project(ws, &fx.project_slug).await,

        // Webhooks have no generated `atlas_client` methods (added in a later
        // batch), so they go through `raw_call`. The capability gate runs inside
        // the `Authorized` extractor (the first handler param) before the JSON
        // body is read, so the wrong/zero-scope passes are denied even though
        // `raw_call` sends no body on POST/PATCH.
        Case::CreateWebhook => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/webhooks")),
            )
            .await
        }
        Case::ListWebhooks => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/webhooks")),
            )
            .await
        }
        Case::GetWebhook => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/webhooks/{}", fx.webhook_id),
                ),
            )
            .await
        }
        Case::UpdateWebhook => {
            raw_call(
                http,
                base_url,
                token,
                "PATCH",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/webhooks/{}", fx.webhook_id),
                ),
            )
            .await
        }
        Case::DeleteWebhook => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/webhooks/{}", fx.webhook_id),
                ),
            )
            .await
        }
        // `webhook_id` here is a SECONDARY path param: the capability gate in the
        // extractor short-circuits a wrong-scope call before the handler body
        // ever looks the delivery up, so the seeded id only matters for the
        // positive pass (where the handler resolves the real webhook).
        Case::ListWebhookDeliveries => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/webhooks/{}/deliveries", fx.webhook_id),
                ),
            )
            .await
        }

        // Config family (tags + property-definitions). Like webhooks, these have
        // no generated `atlas_client` methods, so they go through `raw_call`. The
        // capability gate runs inside the `Authorized` extractor (the first
        // handler param) before any secondary `Path<...>` or JSON body is read,
        // so the wrong/zero-scope passes are denied even though `raw_call` sends
        // no body and the tag/property ids are throwaway nils.
        Case::SemanticReindexPlan => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/semantic-search/reindex"),
                ),
            )
            .await
        }
        Case::SemanticReindexStart => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/semantic-search/reindex"),
                ),
            )
            .await
        }
        Case::ListTags => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/tags")),
            )
            .await
        }
        Case::CreateTag => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/tags")),
            )
            .await
        }
        Case::ListUsedLabels => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/tags/used")),
            )
            .await
        }
        Case::UpdateTag => {
            raw_call(
                http,
                base_url,
                token,
                "PATCH",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/tags/{nil}")),
            )
            .await
        }
        Case::DeleteTag => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/tags/{nil}")),
            )
            .await
        }
        Case::ListPropertyDefinitions => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/property-definitions")),
            )
            .await
        }
        Case::CreatePropertyDefinition => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &support::path::api_path("acta", &format!("/workspaces/{ws}/property-definitions")),
            )
            .await
        }
        Case::DeletePropertyDefinition => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &support::path::api_path(
                    "acta",
                    &format!("/workspaces/{ws}/property-definitions/{nil}"),
                ),
            )
            .await
        }

        // Workspace rename keeps the WorkspaceMember extractor and gates
        // `config:update` manually inside the handler (after the body parses), so
        // the sweep uses the generated `atlas_client` method with a valid body;
        // the workspace slug is the primary resource, so no secondary id is needed.
        Case::RenameWorkspace => client
            .acta()
            .update_workspace(
                ws,
                UpdateWorkspaceRequest {
                    name: "sweep-renamed".into(),
                },
            )
            .await
            .map(|_| ()),

        // Grant list reads reuse the seeded project slug so the positive pass
        // resolves the real project (ProjectRes) and returns 200; the gate runs
        // in the `Authorized` extractor, so zero/wrong-scope is denied first.
        Case::ListProjectGrants => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path(
                    "custos",
                    &format!("/workspaces/{ws}/projects/{}/grants", fx.project_slug),
                ),
            )
            .await
        }
        Case::ListWorkspaceGrants => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &support::path::api_path("custos", &format!("/workspaces/{ws}/grants")),
            )
            .await
        }

        // Saved searches are manually gated inside each handler (WorkspaceMember
        // is kept, not switched to Authorized), so the scope check runs after the
        // JSON body is parsed. These go through the generated `atlas_client`
        // methods with dummy-valid bodies so the body deserializes and the gate
        // is what denies a wrong/zero-scope caller; ids are throwaway nils.
        Case::ListSavedSearches => client.acta().list_saved_searches(ws).await.map(|_| ()),
        Case::CreateSavedSearch => client
            .acta()
            .create_saved_search(
                ws,
                CreateSavedSearchRequest {
                    name: "sweep".into(),
                    query: "status:open".into(),
                },
            )
            .await
            .map(|_| ()),
        Case::RenameSavedSearch => client
            .acta()
            .rename_saved_search(
                ws,
                nil,
                RenameSavedSearchRequest {
                    name: "sweep-renamed".into(),
                },
            )
            .await
            .map(|_| ()),
        Case::DeleteSavedSearch => client.acta().delete_saved_search(ws, nil).await,

        // Task views mirror saved searches: WorkspaceMember is kept and the scope
        // is enforced manually inside each handler, so the sweep uses the
        // generated `atlas_client` methods with dummy-valid bodies (an empty
        // filter set is a valid "all tasks" view); ids are throwaway nils.
        Case::ListTaskViews => client.acta().list_task_views(ws).await.map(|_| ()),
        Case::CreateTaskView => client
            .acta()
            .create_task_view(
                ws,
                CreateTaskViewRequest {
                    name: "sweep".into(),
                    filters: TaskViewFiltersDto::default(),
                },
            )
            .await
            .map(|_| ()),
        Case::GetTaskView => client.acta().get_task_view(ws, nil).await.map(|_| ()),
        Case::UpdateTaskView => client
            .acta()
            .update_task_view(
                ws,
                nil,
                UpdateTaskViewRequest {
                    name: "sweep-renamed".into(),
                    filters: TaskViewFiltersDto::default(),
                },
            )
            .await
            .map(|_| ()),
        Case::DeleteTaskView => client.acta().delete_task_view(ws, nil).await,
    }
}

/// Fires a raw HTTP call for the handful of routes (board/document presence)
/// that have no generated `atlas_client` method, mirroring the client's own
/// error-decoding shape so callers can use one assertion path for every case.
///
/// `v2-e3-s4` PR5 (D1, R2), collapsed to one mount by `v2-e3-s7` (D1/U2):
/// every call site passes its path already fully mounted, built by
/// `support::path::api_path` at the route's own owning component's
/// `/api/v2/<component>` namespace — this helper no longer re-derives a
/// mount from a bare relative path, since exactly one mount exists.
async fn raw_call(
    http: &reqwest::Client,
    base_url: &str,
    token: &str,
    method: &str,
    path: &str,
) -> Result<(), ClientError> {
    let url = format!("{base_url}{path}");
    // No `_` fallback: every call site names its method explicitly, so an
    // unrecognized value is a bug in the caller, not a route this helper
    // should silently retry as GET (the defect this PR's other raw-request
    // migration in `api_401_sweep.rs` fixes for the same reason).
    let builder = match method {
        "GET" => http.get(&url),
        "POST" => http.post(&url),
        "PUT" => http.put(&url),
        "PATCH" => http.patch(&url),
        "DELETE" => http.delete(&url),
        other => panic!("raw_call: unsupported method `{other}`"),
    };
    let builder = if path.ends_with("/content/search") {
        builder.json(&serde_json::json!({"query": "x"}))
    } else {
        builder
    };
    let response = builder
        .bearer_auth(token)
        .header("x-atlas-csrf", "1")
        .send()
        .await
        .map_err(ClientError::Transport)?;

    if response.status().is_success() {
        return Ok(());
    }

    let problem: ProblemDetails = response
        .json()
        .await
        .unwrap_or_else(|_| ProblemDetails::new("urn:atlas:error:unknown", "Unknown", 0));
    Err(ClientError::Api(problem))
}

fn is_scope_denial(err: &ClientError) -> bool {
    matches!(
        err,
        ClientError::Api(p) if p.status == 403
            && p.detail.as_deref().unwrap_or("").contains("lacks required scope")
    )
}

fn http_method_str(method: HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Head => "HEAD",
        HttpMethod::Options => "OPTIONS",
    }
}

#[tokio::test]
async fn capability_sweep_covers_every_registry_entry() {
    let mut from_cases: Vec<(&'static str, &'static str)> =
        Case::ALL.iter().map(|c| c.registry_key()).collect();
    from_cases.sort_unstable();

    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let mut from_registry: Vec<(&'static str, &'static str)> = registry
        .entries()
        .iter()
        .flat_map(|entry| entry.api.routes.iter())
        .filter_map(|route| {
            route.action.as_ref().map(|action| {
                (
                    http_method_str(route.method),
                    capability_from_action_id(action).as_str(),
                )
            })
        })
        .collect();
    from_registry.sort_unstable();

    assert_eq!(
        from_cases, from_registry,
        "the sweep's Case list must cover exactly the live registry's entries with action: Some(_)"
    );
}

/// T5.15/T5.16/T7.14 (`v2-e3-s4` PR5/PR7, D1/D10, R2), collapsed to one
/// mount by `v2-e3-s7` (D1/U2): this sweep drives every case through
/// `AtlasClient` or `raw_call`, never `route_matrix()` (see this file's own
/// module doc / D1's grounding). `AtlasClient`'s own typed methods stay
/// `/api/v2`-absolute (S6's job, not this one), and every `raw_call`-backed
/// case is built by `support::path::api_path`, joined with its OWN
/// component's prefix (`raw_call`'s own per-route lookup), never a flat or
/// another component's prefix.
#[tokio::test]
async fn zero_scope_key_gets_scope_403_on_every_capability_gated_route() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "cap-sweep-zero-owner").await;
    let fx = seed_fixtures(&owner, &db, &ws.slug, ws.id.0, owner_user.id).await;

    let token = create_scoped_agent(&db, owner_user.id, "cap-sweep-zero", vec![]).await;
    let agent = AtlasClient::new(server.base_url()).with_token(token.clone());
    let http = reqwest::Client::new();
    let mut examined_routes = 0usize;

    for case in Case::ALL {
        let result = invoke(*case, &agent, &http, server.base_url(), &token, &fx).await;
        assert!(
            matches!(&result, Err(e) if is_scope_denial(e)),
            "case {case:?}: expected a scope-403, got {result:?}"
        );
        examined_routes += 1;
    }

    assert!(
        examined_routes > 0,
        "zero_scope_key_gets_scope_403_on_every_capability_gated_route must examine at least \
         one capability-gated route, or its assertions pass vacuously"
    );

    db.teardown().await;
}

/// See [`zero_scope_key_gets_scope_403_on_every_capability_gated_route`]'s
/// doc.
#[tokio::test]
async fn all_capabilities_scope_key_never_gets_scope_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "cap-sweep-all-owner").await;
    let fx = seed_fixtures(&owner, &db, &ws.slug, ws.id.0, owner_user.id).await;

    let token = create_scoped_agent(
        &db,
        owner_user.id,
        "cap-sweep-all",
        Capability::ALL.to_vec(),
    )
    .await;
    let agent = AtlasClient::new(server.base_url()).with_token(token.clone());
    let http = reqwest::Client::new();
    let mut examined_routes = 0usize;

    for case in Case::ALL {
        let result = invoke(*case, &agent, &http, server.base_url(), &token, &fx).await;
        if let Err(e) = &result {
            assert!(
                !is_scope_denial(e),
                "case {case:?}: unexpected scope-403 with all catalog capabilities: {e:?}"
            );
        }
        examined_routes += 1;
    }

    assert!(
        examined_routes > 0,
        "all_capabilities_scope_key_never_gets_scope_403 must examine at least one \
         capability-gated route, or its assertions pass vacuously"
    );

    db.teardown().await;
}

/// The first `Case` in `Case::ALL` that declares `cap` as its required
/// capability — the single representative route the per-capability matrix
/// exercises. Derived from `Case::registry_key` (not a hand-maintained second
/// table), so it can never drift from the sweep's own route mapping.
fn representative_case_for(cap: Capability) -> Case {
    Case::ALL
        .iter()
        .copied()
        .find(|c| c.registry_key().1 == cap.as_str())
        .expect("every catalog capability has at least one representative Case")
}

/// Any capability guaranteed distinct from `cap`, used as the WRONG scope a key
/// holds when proving a route's gate rejects the wrong capability. The catalog
/// always contains at least one other capability.
fn a_different_capability(cap: Capability) -> Capability {
    Capability::ALL
        .into_iter()
        .find(|c| c.as_str() != cap.as_str())
        .expect("the catalog holds more than one capability")
}

/// Per-capability positive + wrong-scope matrix.
///
/// The zero-scope and all-capabilities sweeps above prove deny-by-default and
/// never-over-deny, but neither proves a route requires its OWN capability: a
/// route mis-annotated with the wrong marker would still pass both. For each
/// catalog capability this picks one representative route and asserts both
/// directions of correctness:
///
/// - **positive**: a key scoped to EXACTLY that one capability passes the gate
///   (result is not a scope-403; a downstream 404/400 on the throwaway ids is
///   still a pass, mirroring the all-capabilities sweep's convention);
/// - **wrong-scope**: a key scoped to exactly one DIFFERENT capability is
///   scope-denied on the same route.
///
/// Ordering matters, and not as the extractor's doc-comment first suggests: the
/// capability gate runs AFTER the primary `ResolvedResource` is looked up (the
/// resolve chain → role check → scope gate order), so it is only the SECONDARY
/// path/body params that the gate short-circuits. A wrong-scope call therefore
/// needs its primary fixture to still exist to reach the gate at all — a
/// positive `DELETE` that removes the shared fixture would otherwise turn a
/// later wrong-scope call into a resolve-time 404 instead of the expected 403.
/// To stay robust regardless of the catalog order, the two directions run in
/// separate passes:
///   1. all wrong-scope calls first — each is denied inside the extractor
///      before its handler body runs, so none of them mutate and every fixture
///      stays intact for the whole pass;
///   2. all positive calls second — these may delete fixtures, but a positive
///      only asserts "not a scope-403", which depends solely on the key's scope
///      set and is immune to a downstream 404.
#[tokio::test]
async fn each_capability_gate_admits_its_own_scope_and_rejects_a_wrong_one() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "cap-matrix-owner").await;
    let fx = seed_fixtures(&owner, &db, &ws.slug, ws.id.0, owner_user.id).await;
    let http = reqwest::Client::new();

    // Pass 1: a key holding a DIFFERENT capability is scope-denied on each
    // representative route. Denials fire before any handler body, so no fixture
    // is mutated during this pass.
    for cap in Capability::ALL {
        let case = representative_case_for(cap);
        let wrong = a_different_capability(cap);
        let slug = cap.as_str().replace(':', "-");

        let wrong_token = create_scoped_agent(
            &db,
            owner_user.id,
            &format!("cap-matrix-wrong-{slug}"),
            vec![wrong],
        )
        .await;
        let wrong_agent = AtlasClient::new(server.base_url()).with_token(wrong_token.clone());
        let wrong_result = invoke(
            case,
            &wrong_agent,
            &http,
            server.base_url(),
            &wrong_token,
            &fx,
        )
        .await;
        assert!(
            matches!(&wrong_result, Err(e) if is_scope_denial(e)),
            "capability {}: representative case {case:?} was NOT scope-denied to a key holding only \
             {} — the gate is not enforcing this capability: {wrong_result:?}",
            cap.as_str(),
            wrong.as_str()
        );
    }

    // Pass 2: a key holding EXACTLY the required capability passes the gate. May
    // mutate/delete fixtures; a positive only requires "not a scope-403".
    for cap in Capability::ALL {
        let case = representative_case_for(cap);
        let slug = cap.as_str().replace(':', "-");

        let ok_token = create_scoped_agent(
            &db,
            owner_user.id,
            &format!("cap-matrix-ok-{slug}"),
            vec![cap],
        )
        .await;
        let ok_agent = AtlasClient::new(server.base_url()).with_token(ok_token.clone());
        let ok_result = invoke(case, &ok_agent, &http, server.base_url(), &ok_token, &fx).await;
        if let Err(e) = &ok_result {
            assert!(
                !is_scope_denial(e),
                "capability {}: representative case {case:?} was scope-denied to a key that HOLDS it \
                 — route likely annotated with the wrong capability: {e:?}",
                cap.as_str()
            );
        }
    }

    db.teardown().await;
}

#[tokio::test]
async fn partial_document_edit_requires_docs_update_capability() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "cap-edit-content-owner").await;
    let fx = seed_fixtures(&owner, &db, &ws.slug, ws.id.0, owner_user.id).await;
    let request = DocumentContentEditRequest {
        base_revision_id: fx.document_head_revision_id,
        edit: DocumentLineEditRequest::Insert {
            position: 2,
            content: "updated".into(),
        },
    };
    let http = reqwest::Client::new();

    let update_token = create_scoped_agent(
        &db,
        owner_user.id,
        "cap-edit-content-update",
        vec![
            Capability::ALL
                .into_iter()
                .find(|capability| capability.as_str() == "docs:update")
                .expect("docs:update capability"),
        ],
    )
    .await;
    let update_response = http
        .patch(support::path::api_url(
            server.base_url(),
            "acta",
            &format!(
                "/workspaces/{}/documents/{}/content/range",
                fx.ws_slug, fx.document_ref
            ),
        ))
        .bearer_auth(&update_token)
        .json(&request)
        .send()
        .await
        .expect("docs:update partial edit request");
    assert_eq!(update_response.status(), reqwest::StatusCode::OK);
    let updated: DocumentCompactDto = update_response.json().await.expect("partial edit response");
    assert_ne!(updated.head_revision_id, fx.document_head_revision_id);

    let wrong_token = create_scoped_agent(
        &db,
        owner_user.id,
        "cap-edit-content-wrong",
        vec![
            Capability::ALL
                .into_iter()
                .find(|capability| capability.as_str() == "docs:read")
                .expect("docs:read capability"),
        ],
    )
    .await;
    let wrong_response = http
        .patch(support::path::api_url(
            server.base_url(),
            "acta",
            &format!(
                "/workspaces/{}/documents/{}/content/range",
                fx.ws_slug, fx.document_ref
            ),
        ))
        .bearer_auth(&wrong_token)
        .json(&request)
        .send()
        .await
        .expect("wrong-scope partial edit request");
    assert_eq!(wrong_response.status(), reqwest::StatusCode::FORBIDDEN);
    let problem: ProblemDetails = wrong_response.json().await.expect("scope denial problem");
    assert!(
        problem
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("lacks required scope: docs:update"))
    );

    db.teardown().await;
}

#[tokio::test]
async fn document_move_batch_requires_docs_update_capability() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "cap-document-move-batch-owner").await;
    let fx = seed_fixtures(&owner, &db, &ws.slug, ws.id.0, owner_user.id).await;
    let http = reqwest::Client::new();
    let body = serde_json::json!({
        "moves": [{
            "source_document": fx.document_ref,
            "folder_id": null
        }]
    });

    let update_token = create_scoped_agent(
        &db,
        owner_user.id,
        "cap-document-move-batch-update",
        vec![
            Capability::ALL
                .into_iter()
                .find(|capability| capability.as_str() == "docs:update")
                .expect("docs:update capability"),
        ],
    )
    .await;
    let update_response = http
        .post(support::path::api_url(
            server.base_url(),
            "acta",
            &format!("/workspaces/{}/documents/moves/batch", fx.ws_slug),
        ))
        .bearer_auth(&update_token)
        .json(&body)
        .send()
        .await
        .expect("docs:update document move batch request");
    assert_eq!(update_response.status(), reqwest::StatusCode::OK);
    let outcomes: serde_json::Value = update_response
        .json()
        .await
        .expect("decode docs:update batch result");
    assert_eq!(outcomes[0]["outcome"], "success");
    assert_eq!(outcomes[0]["document"]["id"], fx.document_ref);

    let wrong_token = create_scoped_agent(
        &db,
        owner_user.id,
        "cap-document-move-batch-wrong",
        vec![
            Capability::ALL
                .into_iter()
                .find(|capability| capability.as_str() == "docs:read")
                .expect("docs:read capability"),
        ],
    )
    .await;
    let wrong_response = http
        .post(support::path::api_url(
            server.base_url(),
            "acta",
            &format!("/workspaces/{}/documents/moves/batch", fx.ws_slug),
        ))
        .bearer_auth(&wrong_token)
        .json(&body)
        .send()
        .await
        .expect("wrong-scope document move batch request");
    assert_eq!(wrong_response.status(), reqwest::StatusCode::FORBIDDEN);
    let problem: ProblemDetails = wrong_response
        .json()
        .await
        .expect("wrong-scope batch problem");
    assert!(
        problem
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("lacks required scope: docs:update"))
    );

    db.teardown().await;
}

#[tokio::test]
async fn reference_batch_requires_tasks_update_capability() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "cap-reference-batch-owner").await;
    let fx = seed_fixtures(&owner, &db, &ws.slug, ws.id.0, owner_user.id).await;
    let http = reqwest::Client::new();
    let body = serde_json::json!({
        "references": [{
            "kind": "docs",
            "target_task_readable_id": null,
            "target_document_id": fx.document_ref,
        }]
    });

    let update_token = create_scoped_agent(
        &db,
        owner_user.id,
        "cap-reference-batch-update",
        vec![
            Capability::ALL
                .into_iter()
                .find(|capability| capability.as_str() == "tasks:update")
                .expect("tasks:update capability"),
        ],
    )
    .await;
    let update_response = http
        .post(support::path::api_url(
            server.base_url(),
            "acta",
            &format!(
                "/workspaces/{}/tasks/{}/references/batch",
                fx.ws_slug, fx.task_readable_id
            ),
        ))
        .bearer_auth(&update_token)
        .json(&body)
        .send()
        .await
        .expect("tasks:update batch request");
    assert_eq!(update_response.status(), reqwest::StatusCode::OK);
    let result: Vec<atlas_api::dtos::boards_tasks::CreateReferenceBatchResultDto> = update_response
        .json()
        .await
        .expect("decode tasks:update batch result");
    assert!(matches!(
        result.as_slice(),
        [atlas_api::dtos::boards_tasks::CreateReferenceBatchResultDto::Success { index: 0, .. }]
    ));

    let wrong_token = create_scoped_agent(
        &db,
        owner_user.id,
        "cap-reference-batch-wrong",
        vec![
            Capability::ALL
                .into_iter()
                .find(|capability| capability.as_str() == "tasks:read")
                .expect("tasks:read capability"),
        ],
    )
    .await;
    let wrong_response = http
        .post(support::path::api_url(
            server.base_url(),
            "acta",
            &format!(
                "/workspaces/{}/tasks/{}/references/batch",
                fx.ws_slug, fx.task_readable_id
            ),
        ))
        .bearer_auth(&wrong_token)
        .json(&body)
        .send()
        .await
        .expect("wrong-scope batch request");
    assert_eq!(wrong_response.status(), reqwest::StatusCode::FORBIDDEN);
    let problem: ProblemDetails = wrong_response
        .json()
        .await
        .expect("wrong-scope batch problem");
    assert!(
        problem
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("lacks required scope: tasks:update"))
    );

    db.teardown().await;
}
