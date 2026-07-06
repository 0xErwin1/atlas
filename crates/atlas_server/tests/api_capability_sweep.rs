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
//! the four manual `enforce_api_key_scope` call sites are covered identically:
//!
//! 1. A key with an EMPTY scope set gets 403 with a scope-denial detail on
//!    every `ROUTE_REGISTRY` entry that declares `capability: Some(_)`.
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

use atlas_api::{
    dtos::{
        CreateProjectRequest, UpdateProjectRequest,
        boards_tasks::{
            AddAssigneeRequest, CreateBoardRequest, CreateChecklistItemRequest,
            CreateColumnRequest, CreateCommentRequest, CreateReferenceRequest,
            CreateSubtaskRequest, CreateTaskRequest, MoveTaskRequest, PromoteChecklistItemRequest,
            UpdateBoardRequest, UpdateChecklistItemRequest, UpdateColumnRequest,
            UpdateCommentRequest, UpdateTaskRequest, WorkspaceTaskQueryParams,
        },
        documents::{
            CreateDocumentRequest, MoveDocumentRequest, UpdateContentRequest, UpdateDocumentRequest,
        },
        folders::{CreateFolderRequest, MoveFolderRequest, RenameFolderRequest},
        status_templates::{CreateStatusTemplateRequest, UpdateStatusTemplateRequest},
    },
    problem::ProblemDetails,
};
use atlas_client::{AtlasClient, ClientError};
use atlas_domain::{Actor, entities::identity::ApiKeyType, ids::UserId, permissions::Capability};
use atlas_server::{
    crypto::WebhookCrypto,
    persistence::repos::{ApiKeyRepo, NewApiKey, PgApiKeyRepo, PgWebhookSubscriptionRepo},
    routes::registry::ROUTE_REGISTRY,
};
use support::{TestDb, TestServer, login_user_with_workspace};

struct Fixtures {
    ws_slug: String,
    project_slug: String,
    board_id: uuid::Uuid,
    task_readable_id: String,
    document_ref: String,
    folder_id: uuid::Uuid,
    doc_attachment_id: uuid::Uuid,
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
        .create_board(
            ws_slug,
            &project.slug,
            CreateBoardRequest {
                name: "Sweep Board".into(),
            },
        )
        .await
        .expect("create board");

    let column = owner
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
        .create_task(
            ws_slug,
            board.id,
            CreateTaskRequest {
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
        .upload_attachment(
            ws_slug,
            &document.id.to_string(),
            "sweep.txt",
            "text/plain",
            b"hello".to_vec(),
        )
        .await
        .expect("upload document attachment");

    let folder = owner
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
        &Actor::User(user_id),
    )
    .await
    .expect("create sweep webhook");

    Fixtures {
        ws_slug: ws_slug.to_string(),
        project_slug: project.slug,
        board_id: board.id,
        task_readable_id: task.readable_id,
        document_ref: document.id.to_string(),
        folder_id: folder.id,
        doc_attachment_id: attachment.id,
        webhook_id: webhook.id,
    }
}

/// Creates a global agent key (so it inherits the owner's Editor+ reach in
/// every resource without needing per-resource grants) with the given scope
/// set, and returns the plaintext bearer token.
async fn create_scoped_agent(
    db: &TestDb,
    owner_user_id: atlas_domain::ids::UserId,
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
    DeleteReference,
    UploadTaskAttachment,
    ListTaskAttachments,
    DownloadTaskAttachment,
    DeleteTaskAttachment,
    ListTaskBacklinks,
    ListChecklist,
    CreateChecklistItem,
    UpdateChecklistItem,
    DeleteChecklistItem,
    PromoteChecklistItem,
    ListSubtasks,
    CreateSubtask,
    PromoteSubtask,
    ListActivity,
    ListTaskComments,
    AddTaskComment,
    UpdateTaskComment,
    DeleteTaskComment,
    // ---- docs (22) ----
    CreateDocument,
    ListDocuments,
    GetDocument,
    UpdateDocument,
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
    DocumentHeartbeat,
    DocumentLeave,
    // ---- boards (12) ----
    CreateBoard,
    ListBoards,
    GetBoard,
    UpdateBoard,
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
    // ---- grants: read-only list reads (2) ----
    ListProjectGrants,
    ListWorkspaceGrants,
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
        Case::DeleteReference,
        Case::UploadTaskAttachment,
        Case::ListTaskAttachments,
        Case::DownloadTaskAttachment,
        Case::DeleteTaskAttachment,
        Case::ListTaskBacklinks,
        Case::ListChecklist,
        Case::CreateChecklistItem,
        Case::UpdateChecklistItem,
        Case::DeleteChecklistItem,
        Case::PromoteChecklistItem,
        Case::ListSubtasks,
        Case::CreateSubtask,
        Case::PromoteSubtask,
        Case::ListActivity,
        Case::ListTaskComments,
        Case::AddTaskComment,
        Case::UpdateTaskComment,
        Case::DeleteTaskComment,
        Case::CreateDocument,
        Case::ListDocuments,
        Case::GetDocument,
        Case::UpdateDocument,
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
        Case::DocumentHeartbeat,
        Case::DocumentLeave,
        Case::CreateBoard,
        Case::ListBoards,
        Case::GetBoard,
        Case::UpdateBoard,
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
        Case::ListProjectGrants,
        Case::ListWorkspaceGrants,
    ];

    /// `(method, capability)` as declared for this case's route — cross-checked
    /// against `ROUTE_REGISTRY` in `capability_sweep_covers_every_registry_entry`
    /// so this list can never silently drift from the registry.
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
            Case::DeleteReference => ("DELETE", "tasks:update"),
            Case::UploadTaskAttachment => ("POST", "tasks:update"),
            Case::ListTaskAttachments => ("GET", "tasks:read"),
            Case::DownloadTaskAttachment => ("GET", "tasks:read"),
            Case::DeleteTaskAttachment => ("DELETE", "tasks:update"),
            Case::ListTaskBacklinks => ("GET", "tasks:read"),
            Case::ListChecklist => ("GET", "tasks:read"),
            Case::CreateChecklistItem => ("POST", "tasks:update"),
            Case::UpdateChecklistItem => ("PATCH", "tasks:update"),
            Case::DeleteChecklistItem => ("DELETE", "tasks:update"),
            Case::PromoteChecklistItem => ("POST", "tasks:create"),
            Case::ListSubtasks => ("GET", "tasks:read"),
            Case::CreateSubtask => ("POST", "tasks:create"),
            Case::PromoteSubtask => ("POST", "tasks:update"),
            Case::ListActivity => ("GET", "tasks:read"),
            Case::ListTaskComments => ("GET", "tasks:read"),
            Case::AddTaskComment => ("POST", "tasks:update"),
            Case::UpdateTaskComment => ("PATCH", "tasks:update"),
            Case::DeleteTaskComment => ("DELETE", "tasks:update"),

            Case::CreateDocument => ("POST", "docs:create"),
            Case::ListDocuments => ("GET", "docs:read"),
            Case::GetDocument => ("GET", "docs:read"),
            Case::UpdateDocument => ("PATCH", "docs:update"),
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
            Case::DocumentHeartbeat => ("POST", "docs:read"),
            Case::DocumentLeave => ("DELETE", "docs:read"),

            Case::CreateBoard => ("POST", "boards:create"),
            Case::ListBoards => ("GET", "boards:read"),
            Case::GetBoard => ("GET", "boards:read"),
            Case::UpdateBoard => ("PATCH", "boards:update"),
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

            Case::ListProjectGrants => ("GET", "grants:read"),
            Case::ListWorkspaceGrants => ("GET", "grants:read"),
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
            .create_task(
                ws,
                fx.board_id,
                CreateTaskRequest {
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
            .list_tasks(ws, fx.board_id, None, None)
            .await
            .map(|_| ()),
        Case::ListWorkspaceTasks => client
            .list_workspace_tasks(ws, &WorkspaceTaskQueryParams::default())
            .await
            .map(|_| ()),
        Case::GetTask => client.get_task(ws, &fx.task_readable_id).await.map(|_| ()),
        Case::UpdateTask => client
            .update_task(ws, &fx.task_readable_id, UpdateTaskRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteTask => client.delete_task(ws, &fx.task_readable_id).await,
        Case::MoveTask => client
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
            .list_assignees(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::AddAssignee => client
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
                .remove_assignee(ws, &fx.task_readable_id, &format!("user:{nil}"))
                .await
        }
        Case::ListReferences => client
            .list_references(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::CreateReference => client
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
        Case::DeleteReference => client.delete_reference(ws, &fx.task_readable_id, nil).await,
        Case::UploadTaskAttachment => client
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
            .list_task_attachments(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::DownloadTaskAttachment => client
            .download_task_attachment(ws, &fx.task_readable_id, nil)
            .await
            .map(|_| ()),
        Case::DeleteTaskAttachment => {
            client
                .delete_task_attachment(ws, &fx.task_readable_id, nil)
                .await
        }
        Case::ListTaskBacklinks => client
            .list_task_backlinks(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::ListChecklist => client
            .list_checklist(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::CreateChecklistItem => client
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
                .delete_checklist_item(ws, &fx.task_readable_id, nil)
                .await
        }
        Case::PromoteChecklistItem => client
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
            .list_subtasks(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::CreateSubtask => client
            .create_subtask(
                ws,
                &fx.task_readable_id,
                CreateSubtaskRequest { title: "x".into() },
            )
            .await
            .map(|_| ()),
        Case::PromoteSubtask => client
            .promote_subtask(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::ListActivity => client
            .list_activity(ws, &fx.task_readable_id)
            .await
            .map(|_| ()),
        Case::ListTaskComments => client
            .list_comments(ws, &fx.task_readable_id, None, None)
            .await
            .map(|_| ()),
        Case::AddTaskComment => client
            .add_comment(
                ws,
                &fx.task_readable_id,
                CreateCommentRequest { body: "x".into() },
            )
            .await
            .map(|_| ()),
        Case::UpdateTaskComment => client
            .update_comment(
                ws,
                &fx.task_readable_id,
                nil,
                UpdateCommentRequest { body: "x".into() },
            )
            .await
            .map(|_| ()),
        Case::DeleteTaskComment => client.delete_comment(ws, &fx.task_readable_id, nil).await,

        Case::CreateDocument => client
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
            .list_documents(ws, &fx.project_slug, None, None)
            .await
            .map(|_| ()),
        Case::GetDocument => client.get_document(ws, &fx.document_ref).await.map(|_| ()),
        Case::UpdateDocument => client
            .update_document(ws, &fx.document_ref, UpdateDocumentRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteDocument => client.delete_document(ws, &fx.document_ref).await,
        Case::UpdateContent => client
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
            .list_document_history(ws, &fx.document_ref, None, None)
            .await
            .map(|_| ()),
        Case::GetRevisionContent => client
            .get_revision_content(ws, &fx.document_ref, 1)
            .await
            .map(|_| ()),
        Case::ListDocBacklinks => client
            .list_backlinks(ws, &fx.document_ref, None, None)
            .await
            .map(|_| ()),
        Case::GetFrontmatter => client
            .get_frontmatter(ws, &fx.document_ref)
            .await
            .map(|_| ()),
        Case::UploadDocAttachment => client
            .upload_attachment(ws, &fx.document_ref, "f.txt", "text/plain", vec![1, 2, 3])
            .await
            .map(|_| ()),
        Case::ListDocAttachments => client
            .list_attachments(ws, &fx.document_ref, None, None)
            .await
            .map(|_| ()),
        Case::DownloadDocAttachment => client
            .download_attachment(ws, fx.doc_attachment_id)
            .await
            .map(|_| ()),
        Case::DeleteDocAttachment => client.delete_attachment(ws, fx.doc_attachment_id).await,
        Case::MoveDocument => client
            .move_document(ws, &fx.document_ref, MoveDocumentRequest::default())
            .await
            .map(|_| ()),
        Case::CopyDocument => client
            .copy_document(ws, &fx.document_ref, None)
            .await
            .map(|_| ()),
        Case::ListDocComments => client
            .list_document_comments(ws, &fx.document_ref, None, None)
            .await
            .map(|_| ()),
        Case::AddDocComment => client
            .add_document_comment(
                ws,
                &fx.document_ref,
                CreateCommentRequest { body: "x".into() },
            )
            .await
            .map(|_| ()),
        Case::UpdateDocComment => client
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
                .delete_document_comment(ws, &fx.document_ref, nil)
                .await
        }
        Case::DocumentHeartbeat => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &format!("/v1/workspaces/{ws}/documents/{}/presence", fx.document_ref),
            )
            .await
        }
        Case::DocumentLeave => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &format!("/v1/workspaces/{ws}/documents/{}/presence", fx.document_ref),
            )
            .await
        }

        Case::CreateBoard => client
            .create_board(
                ws,
                &fx.project_slug,
                CreateBoardRequest { name: "x".into() },
            )
            .await
            .map(|_| ()),
        Case::ListBoards => client
            .list_boards(ws, &fx.project_slug, None, None)
            .await
            .map(|_| ()),
        Case::GetBoard => client.get_board(ws, fx.board_id).await.map(|_| ()),
        Case::UpdateBoard => client
            .update_board(ws, fx.board_id, UpdateBoardRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteBoard => client.delete_board(ws, fx.board_id).await,
        Case::CreateColumn => client
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
        Case::ListColumns => client.list_columns(ws, fx.board_id).await.map(|_| ()),
        Case::UpdateColumn => client
            .update_column(ws, fx.board_id, nil, UpdateColumnRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteColumn => client.delete_column(ws, fx.board_id, nil).await,
        Case::ApplyStatusTemplates => client
            .apply_status_templates(ws, fx.board_id)
            .await
            .map(|_| ()),
        Case::BoardHeartbeat => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &format!("/v1/workspaces/{ws}/boards/{}/presence", fx.board_id),
            )
            .await
        }
        Case::BoardLeave => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &format!("/v1/workspaces/{ws}/boards/{}/presence", fx.board_id),
            )
            .await
        }
        Case::ListStatusTemplates => client.list_status_templates(ws).await.map(|_| ()),
        Case::CreateStatusTemplate => client
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
            .update_status_template(ws, nil, UpdateStatusTemplateRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteStatusTemplate => client.delete_status_template(ws, nil).await,

        Case::CreateFolder => client
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
            .list_folders(ws, &fx.project_slug, None, None)
            .await
            .map(|_| ()),
        Case::GetFolder => client.get_folder(ws, fx.folder_id).await.map(|_| ()),
        Case::RenameFolder => client
            .rename_folder(ws, fx.folder_id, RenameFolderRequest { name: "y".into() })
            .await
            .map(|_| ()),
        Case::MoveFolder => client
            .move_folder(
                ws,
                fx.folder_id,
                MoveFolderRequest {
                    parent_folder_id: None,
                },
            )
            .await
            .map(|_| ()),
        Case::CopyFolder => client.copy_folder(ws, fx.folder_id, None).await.map(|_| ()),
        Case::DeleteFolder => client.delete_folder(ws, fx.folder_id).await,

        Case::CreateProject => client
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
        Case::ListProjects => client.list_projects(ws, None, None).await.map(|_| ()),
        Case::GetProject => client.get_project(ws, &fx.project_slug).await.map(|_| ()),
        Case::UpdateProject => client
            .update_project(ws, &fx.project_slug, UpdateProjectRequest::default())
            .await
            .map(|_| ()),
        Case::DeleteProject => client.delete_project(ws, &fx.project_slug).await,

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
                &format!("/v1/workspaces/{ws}/webhooks"),
            )
            .await
        }
        Case::ListWebhooks => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &format!("/v1/workspaces/{ws}/webhooks"),
            )
            .await
        }
        Case::GetWebhook => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &format!("/v1/workspaces/{ws}/webhooks/{}", fx.webhook_id),
            )
            .await
        }
        Case::UpdateWebhook => {
            raw_call(
                http,
                base_url,
                token,
                "PATCH",
                &format!("/v1/workspaces/{ws}/webhooks/{}", fx.webhook_id),
            )
            .await
        }
        Case::DeleteWebhook => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &format!("/v1/workspaces/{ws}/webhooks/{}", fx.webhook_id),
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
                &format!("/v1/workspaces/{ws}/webhooks/{}/deliveries", fx.webhook_id),
            )
            .await
        }

        // Config family (tags + property-definitions). Like webhooks, these have
        // no generated `atlas_client` methods, so they go through `raw_call`. The
        // capability gate runs inside the `Authorized` extractor (the first
        // handler param) before any secondary `Path<...>` or JSON body is read,
        // so the wrong/zero-scope passes are denied even though `raw_call` sends
        // no body and the tag/property ids are throwaway nils.
        Case::ListTags => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &format!("/v1/workspaces/{ws}/tags"),
            )
            .await
        }
        Case::CreateTag => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &format!("/v1/workspaces/{ws}/tags"),
            )
            .await
        }
        Case::ListUsedLabels => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &format!("/v1/workspaces/{ws}/tags/used"),
            )
            .await
        }
        Case::UpdateTag => {
            raw_call(
                http,
                base_url,
                token,
                "PATCH",
                &format!("/v1/workspaces/{ws}/tags/{nil}"),
            )
            .await
        }
        Case::DeleteTag => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &format!("/v1/workspaces/{ws}/tags/{nil}"),
            )
            .await
        }
        Case::ListPropertyDefinitions => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &format!("/v1/workspaces/{ws}/property-definitions"),
            )
            .await
        }
        Case::CreatePropertyDefinition => {
            raw_call(
                http,
                base_url,
                token,
                "POST",
                &format!("/v1/workspaces/{ws}/property-definitions"),
            )
            .await
        }
        Case::DeletePropertyDefinition => {
            raw_call(
                http,
                base_url,
                token,
                "DELETE",
                &format!("/v1/workspaces/{ws}/property-definitions/{nil}"),
            )
            .await
        }

        // Grant list reads reuse the seeded project slug so the positive pass
        // resolves the real project (ProjectRes) and returns 200; the gate runs
        // in the `Authorized` extractor, so zero/wrong-scope is denied first.
        Case::ListProjectGrants => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &format!("/v1/workspaces/{ws}/projects/{}/grants", fx.project_slug),
            )
            .await
        }
        Case::ListWorkspaceGrants => {
            raw_call(
                http,
                base_url,
                token,
                "GET",
                &format!("/v1/workspaces/{ws}/grants"),
            )
            .await
        }
    }
}

/// Fires a raw HTTP call for the handful of routes (board/document presence)
/// that have no generated `atlas_client` method, mirroring the client's own
/// error-decoding shape so callers can use one assertion path for every case.
async fn raw_call(
    http: &reqwest::Client,
    base_url: &str,
    token: &str,
    method: &str,
    path: &str,
) -> Result<(), ClientError> {
    let url = format!("{base_url}{path}");
    let builder = match method {
        "POST" => http.post(&url),
        "PATCH" => http.patch(&url),
        "DELETE" => http.delete(&url),
        _ => http.get(&url),
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

#[tokio::test]
async fn capability_sweep_covers_every_registry_entry() {
    let mut from_cases: Vec<(&'static str, &'static str)> =
        Case::ALL.iter().map(|c| c.registry_key()).collect();
    from_cases.sort_unstable();

    let mut from_registry: Vec<(&'static str, &'static str)> = ROUTE_REGISTRY
        .iter()
        .filter_map(|e| e.capability.map(|cap| (e.method, cap)))
        .collect();
    from_registry.sort_unstable();

    assert_eq!(
        from_cases, from_registry,
        "the sweep's Case list must cover exactly the ROUTE_REGISTRY entries with capability: Some(_)"
    );
}

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

    for case in Case::ALL {
        let result = invoke(*case, &agent, &http, server.base_url(), &token, &fx).await;
        assert!(
            matches!(&result, Err(e) if is_scope_denial(e)),
            "case {case:?}: expected a scope-403, got {result:?}"
        );
    }

    db.teardown().await;
}

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

    for case in Case::ALL {
        let result = invoke(*case, &agent, &http, server.base_url(), &token, &fx).await;
        if let Err(e) = &result {
            assert!(
                !is_scope_denial(e),
                "case {case:?}: unexpected scope-403 with all catalog capabilities: {e:?}"
            );
        }
    }

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
