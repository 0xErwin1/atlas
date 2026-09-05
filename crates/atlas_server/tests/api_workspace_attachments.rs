#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_acta::ports::identity::MembershipRepo;
use atlas_api::dtos::{
    CreateProjectRequest,
    boards_tasks::{
        CreateBoardRequest, CreateColumnRequest, CreateTaskRequest, TaskDto, UpdateTaskRequest,
    },
    documents::{
        CreateDocumentRequest, DocumentDto, RenameAttachmentRequest, UpdateContentRequest,
        WorkspaceAttachmentDto,
    },
};
use atlas_client::AtlasClient;

fn project_req(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: None,
        visibility_role: None,
    }
}

/// Creates a project, a board with one column, and returns a task on it.
async fn seed_task(client: &AtlasClient, ws: &str, project_slug: &str, prefix: &str) -> TaskDto {
    client
        .acta()
        .create_project(ws, project_req(project_slug, prefix))
        .await
        .expect("create project");

    let board = client
        .acta()
        .create_board(
            ws,
            project_slug,
            CreateBoardRequest {
                folder_id: None,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let column = client
        .acta()
        .create_column(
            ws,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create column");

    client
        .acta()
        .create_task(
            ws,
            board.id,
            CreateTaskRequest {
                references: vec![],
                column_id: column.id,
                title: "Task with attachment".to_string(),
                description: Some("see [[file:old.pdf]] for the details".to_string()),
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task")
}

async fn seed_note(
    client: &AtlasClient,
    ws: &str,
    project_slug: &str,
    content: &str,
) -> DocumentDto {
    client
        .acta()
        .create_document(
            ws,
            project_slug,
            CreateDocumentRequest {
                title: "Runbook".to_string(),
                folder_id: None,
                content: Some(content.to_string()),
            },
        )
        .await
        .expect("create document")
}

fn find_by_id(items: &[WorkspaceAttachmentDto], id: uuid::Uuid) -> &WorkspaceAttachmentDto {
    items
        .iter()
        .find(|item| item.id == id)
        .expect("attachment present in the workspace listing")
}

/// The workspace listing spans every place a file can be uploaded — a note, a
/// task, and a comment on either — and carries the metadata the files view
/// renders: uploader, upload time, size, and content type.
#[tokio::test]
async fn workspace_listing_spans_notes_tasks_and_comments() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "ws-attach-1").await;

    let task = seed_task(&client, &ws.slug, "files-proj", "FL").await;
    let note = seed_note(&client, &ws.slug, "files-proj", "# Runbook\n").await;

    let note_file = client
        .acta()
        .upload_attachment(
            &ws.slug,
            note.slug.as_deref().expect("note slug"),
            "policy.pdf",
            "application/pdf",
            b"note bytes".to_vec(),
        )
        .await
        .expect("upload note attachment");

    let task_file = client
        .acta()
        .upload_task_attachment(
            &ws.slug,
            &task.readable_id,
            "old.pdf",
            "application/pdf",
            b"task bytes".to_vec(),
        )
        .await
        .expect("upload task attachment");

    let comment = client
        .acta()
        .add_comment(
            &ws.slug,
            &task.readable_id,
            atlas_api::dtos::boards_tasks::CreateCommentRequest::published("look at this"),
        )
        .await
        .expect("create comment");

    let comment_file = client
        .acta()
        .upload_task_comment_attachment(
            &ws.slug,
            &task.readable_id,
            comment.id,
            "screenshot.png",
            "image/png",
            b"png bytes".to_vec(),
        )
        .await
        .expect("upload comment attachment");

    let page = client
        .acta()
        .list_workspace_attachments(&ws.slug, None, None)
        .await
        .expect("list workspace attachments");

    assert_eq!(page.items.len(), 3, "every uploaded file must be listed");

    let listed_note = find_by_id(&page.items, note_file.id);
    assert_eq!(listed_note.file_name, "policy.pdf");
    assert_eq!(listed_note.content_type, "application/pdf");
    assert_eq!(listed_note.size_bytes, 10);
    assert_eq!(listed_note.owner.kind, "document");
    assert_eq!(listed_note.owner.title, "Runbook");
    assert_eq!(listed_note.owner.document_slug, note.slug);
    assert_eq!(listed_note.owner.comment_id, None);
    assert_eq!(
        listed_note
            .actor
            .as_ref()
            .and_then(|a| a.display_name.clone()),
        Some(user.display_name.clone()),
        "the listing names who uploaded each file"
    );

    let listed_task = find_by_id(&page.items, task_file.id);
    assert_eq!(listed_task.owner.kind, "task");
    assert_eq!(
        listed_task.owner.task_readable_id.as_deref(),
        Some(task.readable_id.as_str())
    );

    let listed_comment = find_by_id(&page.items, comment_file.id);
    assert_eq!(
        listed_comment.owner.kind, "task",
        "a comment attachment resolves to the comment's parent"
    );
    assert_eq!(listed_comment.owner.comment_id, Some(comment.id));

    db.teardown().await;
}

/// Renaming a note attachment rewrites the `[[file:…]]` links that addressed it
/// by name, through the normal content save so the note keeps its history.
#[tokio::test]
async fn renaming_a_note_attachment_rewrites_its_file_links() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ws-attach-2").await;

    client
        .acta()
        .create_project(&ws.slug, project_req("notes-proj", "NP"))
        .await
        .expect("create project");

    let note = seed_note(
        &client,
        &ws.slug,
        "notes-proj",
        "Read [[file:old.pdf]] and also [[file:old.pdf|the old one]].\nUnrelated [[note:other]].",
    )
    .await;
    let slug = note.slug.clone().expect("note slug");

    let uploaded = client
        .acta()
        .upload_attachment(
            &ws.slug,
            &slug,
            "old.pdf",
            "application/pdf",
            b"bytes".to_vec(),
        )
        .await
        .expect("upload attachment");

    let renamed = client
        .acta()
        .rename_workspace_attachment(
            &ws.slug,
            uploaded.id,
            RenameAttachmentRequest {
                file_name: "new.pdf".to_string(),
            },
        )
        .await
        .expect("rename attachment");

    assert_eq!(renamed.file_name, "new.pdf");

    let after = client
        .acta()
        .get_document(&ws.slug, &slug)
        .await
        .expect("read note after rename");

    assert_eq!(
        after.content,
        "Read [[file:new.pdf]] and also [[file:new.pdf|the old one]].\nUnrelated [[note:other]]."
    );
    assert_ne!(
        after.head_revision_id, note.head_revision_id,
        "the rewrite is a normal content save, so it produces a revision"
    );

    db.teardown().await;
}

/// The same rewrite applies to a task description, which is the other body that
/// can carry a `file:` link.
#[tokio::test]
async fn renaming_a_task_attachment_rewrites_its_description_links() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ws-attach-3").await;

    let task = seed_task(&client, &ws.slug, "tasks-proj", "TP").await;

    let uploaded = client
        .acta()
        .upload_task_attachment(
            &ws.slug,
            &task.readable_id,
            "old.pdf",
            "application/pdf",
            b"bytes".to_vec(),
        )
        .await
        .expect("upload attachment");

    client
        .acta()
        .rename_workspace_attachment(
            &ws.slug,
            uploaded.id,
            RenameAttachmentRequest {
                file_name: "spec.pdf".to_string(),
            },
        )
        .await
        .expect("rename attachment");

    let after = client
        .acta()
        .get_task(&ws.slug, &task.readable_id)
        .await
        .expect("read task after rename");

    assert_eq!(
        after.description.as_str(),
        "see [[file:spec.pdf]] for the details"
    );

    db.teardown().await;
}

/// A body with no reference to the file is left byte-identical: renaming must
/// not manufacture a revision or touch unrelated wikilinks.
#[tokio::test]
async fn renaming_leaves_an_unreferenced_body_untouched() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ws-attach-4").await;

    client
        .acta()
        .create_project(&ws.slug, project_req("quiet-proj", "QP"))
        .await
        .expect("create project");

    let note = seed_note(&client, &ws.slug, "quiet-proj", "No file links here.").await;
    let slug = note.slug.clone().expect("note slug");

    let uploaded = client
        .acta()
        .upload_attachment(
            &ws.slug,
            &slug,
            "old.pdf",
            "application/pdf",
            b"bytes".to_vec(),
        )
        .await
        .expect("upload attachment");

    client
        .acta()
        .rename_workspace_attachment(
            &ws.slug,
            uploaded.id,
            RenameAttachmentRequest {
                file_name: "new.pdf".to_string(),
            },
        )
        .await
        .expect("rename attachment");

    let after = client
        .acta()
        .get_document(&ws.slug, &slug)
        .await
        .expect("read note after rename");

    assert_eq!(after.content, "No file links here.");
    assert_eq!(
        after.head_revision_id, note.head_revision_id,
        "an unchanged body must not produce a revision"
    );

    db.teardown().await;
}

/// Two live siblings sharing a name would make every `[[file:…]]` link on that
/// owner ambiguous, so the rename is refused before anything is written.
#[tokio::test]
async fn renaming_onto_a_sibling_name_is_refused_and_changes_nothing() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ws-attach-5").await;

    client
        .acta()
        .create_project(&ws.slug, project_req("dup-proj", "DP"))
        .await
        .expect("create project");

    let note = seed_note(&client, &ws.slug, "dup-proj", "[[file:old.pdf]]").await;
    let slug = note.slug.clone().expect("note slug");

    let first = client
        .acta()
        .upload_attachment(
            &ws.slug,
            &slug,
            "old.pdf",
            "application/pdf",
            b"first".to_vec(),
        )
        .await
        .expect("upload first");

    client
        .acta()
        .upload_attachment(
            &ws.slug,
            &slug,
            "taken.pdf",
            "application/pdf",
            b"second".to_vec(),
        )
        .await
        .expect("upload second");

    let error = client
        .acta()
        .rename_workspace_attachment(
            &ws.slug,
            first.id,
            RenameAttachmentRequest {
                file_name: "taken.pdf".to_string(),
            },
        )
        .await
        .expect_err("rename onto a taken name must fail");

    match error {
        atlas_client::ClientError::Api(problem) => assert_eq!(problem.status, 422),
        other => panic!("expected an API problem, got {other:?}"),
    }

    let after = client
        .acta()
        .get_document(&ws.slug, &slug)
        .await
        .expect("read note after refused rename");
    assert_eq!(after.content, "[[file:old.pdf]]");

    let page = client
        .acta()
        .list_workspace_attachments(&ws.slug, None, None)
        .await
        .expect("list workspace attachments");
    assert_eq!(find_by_id(&page.items, first.id).file_name, "old.pdf");

    db.teardown().await;
}

/// Deleting through the workspace route soft-deletes the row, so the file drops
/// out of the listing while the note it hung on is untouched.
#[tokio::test]
async fn deleting_an_attachment_drops_it_from_the_listing() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ws-attach-6").await;

    let task = seed_task(&client, &ws.slug, "del-proj", "DL").await;

    let uploaded = client
        .acta()
        .upload_task_attachment(
            &ws.slug,
            &task.readable_id,
            "old.pdf",
            "application/pdf",
            b"bytes".to_vec(),
        )
        .await
        .expect("upload attachment");

    let downloaded = client
        .acta()
        .download_attachment(&ws.slug, uploaded.id)
        .await
        .expect("a task attachment downloads through the workspace route");
    assert_eq!(downloaded, b"bytes".to_vec());

    client
        .acta()
        .delete_attachment(&ws.slug, uploaded.id)
        .await
        .expect("delete attachment");

    let page = client
        .acta()
        .list_workspace_attachments(&ws.slug, None, None)
        .await
        .expect("list workspace attachments");
    assert!(
        page.items.is_empty(),
        "a deleted attachment must leave the listing"
    );

    let after = client
        .acta()
        .get_task(&ws.slug, &task.readable_id)
        .await
        .expect("read task after delete");
    assert_eq!(
        after.description.as_str(),
        "see [[file:old.pdf]] for the details",
        "deleting a file does not rewrite the body that referenced it"
    );

    db.teardown().await;
}

/// The listing is permission-filtered: a plain member never sees a file hanging
/// off a private project they hold no grant on.
#[tokio::test]
async fn the_listing_hides_files_in_projects_the_principal_cannot_see() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "ws-attach-7").await;

    owner
        .acta()
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Private".to_string(),
                slug: "private-proj".to_string(),
                task_prefix: "PV".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create private project");

    let note = seed_note(&owner, &ws.slug, "private-proj", "secret").await;
    let hidden = owner
        .acta()
        .upload_attachment(
            &ws.slug,
            note.slug.as_deref().expect("note slug"),
            "secret.pdf",
            "application/pdf",
            b"bytes".to_vec(),
        )
        .await
        .expect("upload attachment");

    let (member, member_user) = support::login_user(&server, &db, "ws-attach-7-member").await;
    db.membership_repo()
        .add(
            &support::ctx(&ws, &member_user),
            member_user.id,
            atlas_acta::entities::identity::MemberRole::Member,
        )
        .await
        .expect("add member");

    let page = member
        .acta()
        .list_workspace_attachments(&ws.slug, None, None)
        .await
        .expect("member lists workspace attachments");

    assert!(
        !page.items.iter().any(|item| item.id == hidden.id),
        "a private project's files stay out of a plain member's listing"
    );

    db.teardown().await;
}

/// A rename request that only changes surrounding whitespace is a no-op the
/// route accepts without touching the body.
#[tokio::test]
async fn renaming_to_the_same_name_is_accepted_without_a_rewrite() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ws-attach-8").await;

    let task = seed_task(&client, &ws.slug, "noop-proj", "NO").await;

    let uploaded = client
        .acta()
        .upload_task_attachment(
            &ws.slug,
            &task.readable_id,
            "old.pdf",
            "application/pdf",
            b"bytes".to_vec(),
        )
        .await
        .expect("upload attachment");

    let renamed = client
        .acta()
        .rename_workspace_attachment(
            &ws.slug,
            uploaded.id,
            RenameAttachmentRequest {
                file_name: "  old.pdf  ".to_string(),
            },
        )
        .await
        .expect("rename to the same name");

    assert_eq!(renamed.file_name, "old.pdf");

    client
        .acta()
        .update_task(
            &ws.slug,
            &task.readable_id,
            UpdateTaskRequest {
                title: Some("Still here".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("task stays writable");

    db.teardown().await;
}

/// Content updates keep working after a rename, which proves the rewrite left
/// the note on a consistent CAS head.
#[tokio::test]
async fn a_note_stays_writable_after_its_attachment_is_renamed() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ws-attach-9").await;

    client
        .acta()
        .create_project(&ws.slug, project_req("cas-proj", "CA"))
        .await
        .expect("create project");

    let note = seed_note(&client, &ws.slug, "cas-proj", "[[file:old.pdf]]").await;
    let slug = note.slug.clone().expect("note slug");

    let uploaded = client
        .acta()
        .upload_attachment(
            &ws.slug,
            &slug,
            "old.pdf",
            "application/pdf",
            b"bytes".to_vec(),
        )
        .await
        .expect("upload attachment");

    client
        .acta()
        .rename_workspace_attachment(
            &ws.slug,
            uploaded.id,
            RenameAttachmentRequest {
                file_name: "new.pdf".to_string(),
            },
        )
        .await
        .expect("rename attachment");

    let head = client
        .acta()
        .get_document(&ws.slug, &slug)
        .await
        .expect("read note after rename");

    client
        .acta()
        .update_content(
            &ws.slug,
            &slug,
            UpdateContentRequest {
                content: "[[file:new.pdf]] plus more".to_string(),
                base_revision_id: head.head_revision_id,
            },
        )
        .await
        .expect("note accepts a normal CAS save after the rewrite");

    db.teardown().await;
}
