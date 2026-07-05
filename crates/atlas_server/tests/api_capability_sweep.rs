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
//! 2. A key holding all 20 catalog capabilities never gets a scope-403 on any
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
    },
    problem::ProblemDetails,
};
use atlas_client::{AtlasClient, ClientError};
use atlas_domain::{entities::identity::ApiKeyType, permissions::Capability};
use atlas_server::{
    persistence::repos::{ApiKeyRepo, NewApiKey, PgApiKeyRepo},
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
}

async fn seed_fixtures(owner: &AtlasClient, ws_slug: &str) -> Fixtures {
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

    Fixtures {
        ws_slug: ws_slug.to_string(),
        project_slug: project.slug,
        board_id: board.id,
        task_readable_id: task.readable_id,
        document_ref: document.id.to_string(),
        folder_id: folder.id,
        doc_attachment_id: attachment.id,
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
    let fx = seed_fixtures(&owner, &ws.slug).await;

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
async fn all_twenty_scope_key_never_gets_scope_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "cap-sweep-all-owner").await;
    let fx = seed_fixtures(&owner, &ws.slug).await;

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
                "case {case:?}: unexpected scope-403 with all 20 capabilities: {e:?}"
            );
        }
    }

    db.teardown().await;
}
