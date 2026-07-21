#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::task_views::TaskViewFilters,
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        comments::{CommentOwner, NewComment},
        documents::{AttachmentOwner, NewAttachment, NewDocument},
        workspace_core::{NewFolder, NewProject},
    },
    permissions::{Principal, Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    AttachmentRepo, BoardRepo, CommentRepo, DocumentRepo, FolderRepo, PgAttachmentRepo,
    PgCommentRepo, ProjectRepo, TaskRepo,
};
use sea_orm::{ConnectionTrait, Statement};

async fn seed_project_tree(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    suffix: &str,
) -> (
    atlas_domain::entities::workspace_core::Project,
    atlas_domain::entities::workspace_core::Folder,
    atlas_domain::entities::workspace_core::Folder,
    atlas_domain::entities::documents::Document,
    atlas_domain::entities::boards_tasks::Board,
    atlas_domain::entities::boards_tasks::Task,
) {
    let task_prefix = match suffix {
        "project" => "PPRJ",
        "folder" => "PFOL",
        _ => "PTEST",
    };

    let project = db
        .project_repo()
        .create(
            ctx,
            NewProject {
                name: format!("Project {suffix}"),
                slug: format!("project-{suffix}"),
                task_prefix: task_prefix.into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");

    let parent = db
        .folder_repo()
        .create(
            ctx,
            NewFolder {
                project_id: Some(project.id),
                parent_folder_id: None,
                name: "Parent".into(),
            },
        )
        .await
        .expect("create parent folder");

    let child = db
        .folder_repo()
        .create(
            ctx,
            NewFolder {
                project_id: Some(project.id),
                parent_folder_id: Some(parent.id),
                name: "Child".into(),
            },
        )
        .await
        .expect("create child folder");

    let document = db
        .doc_repo()
        .create(
            ctx,
            NewDocument {
                title: "Hidden document".into(),
                slug: Some(format!("hidden-document-{suffix}")),
                content: "content".into(),
                folder_id: Some(child.id),
                project_id: Some(project.id),
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    let board = db
        .board_repo()
        .create_board(
            ctx,
            NewBoard {
                project_id: project.id,
                folder_id: Some(child.id),
                name: "Hidden board".into(),
            },
        )
        .await
        .expect("create board");

    let column = db
        .board_repo()
        .add_column(
            ctx,
            board.id,
            "Todo".into(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = db
        .task_repo()
        .create(
            ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: column.id,
                title: "Hidden task".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: Vec::new(),
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("create task");

    (project, parent, child, document, board, task)
}

async fn assert_descendants_remain_live(
    db: &support::TestDb,
    folder_id: uuid::Uuid,
    document_id: uuid::Uuid,
    board_id: uuid::Uuid,
    task_id: uuid::Uuid,
) {
    let row = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT \
                (SELECT deleted_at IS NULL FROM folders WHERE id = $1) AS folder_live, \
                (SELECT deleted_at IS NULL FROM documents WHERE id = $2) AS document_live, \
                (SELECT deleted_at IS NULL FROM boards WHERE id = $3) AS board_live, \
                (SELECT deleted_at IS NULL FROM tasks WHERE id = $4) AS task_live",
            [
                folder_id.into(),
                document_id.into(),
                board_id.into(),
                task_id.into(),
            ],
        ))
        .await
        .expect("query descendant tombstones")
        .expect("descendant rows exist");

    assert!(row.try_get::<bool>("", "folder_live").expect("folder_live"));
    assert!(
        row.try_get::<bool>("", "document_live")
            .expect("document_live")
    );
    assert!(row.try_get::<bool>("", "board_live").expect("board_live"));
    assert!(row.try_get::<bool>("", "task_live").expect("task_live"));
}

#[tokio::test]
async fn direct_list_visibility() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "visibility-ancestors").await;
    let ctx = support::ctx(&ws, &user);

    let (project, parent, child, document, board, task) =
        seed_project_tree(&db, &ctx, "project").await;

    db.project_repo()
        .soft_delete(&ctx, project.id)
        .await
        .expect("delete project");

    assert!(
        db.folder_repo()
            .find(&ctx, child.id)
            .await
            .expect("find folder")
            .is_none()
    );
    assert!(
        db.doc_repo()
            .get(&ctx, document.id)
            .await
            .expect("get document")
            .is_none()
    );
    assert!(
        db.board_repo()
            .find_board(&ctx, board.id)
            .await
            .expect("find board")
            .is_none()
    );
    assert!(
        db.task_repo()
            .find(&ctx, task.id)
            .await
            .expect("find task")
            .is_none()
    );
    assert!(
        db.folder_repo()
            .list_children(&ctx, Some(parent.id))
            .await
            .expect("list folders")
            .is_empty()
    );
    assert!(
        db.doc_repo()
            .list_in_folder(&ctx, child.id)
            .await
            .expect("list documents")
            .is_empty()
    );
    assert!(
        db.doc_repo()
            .list_visible(&ctx, &Principal::User(user.id), Some(project.id), None, 50)
            .await
            .expect("list visible documents")
            .is_empty()
    );
    assert!(
        db.board_repo()
            .list_boards(&ctx, project.id)
            .await
            .expect("list boards")
            .is_empty()
    );
    assert!(
        db.task_repo()
            .list_by_board(&ctx, board.id)
            .await
            .expect("list tasks")
            .is_empty()
    );
    assert!(
        db.task_repo()
            .list_by_workspace_filtered(&ctx, &TaskViewFilters::default(), None, 50)
            .await
            .expect("list workspace tasks")
            .is_empty()
    );
    assert_descendants_remain_live(&db, child.id.0, document.id.0, board.id.0, task.id.0).await;

    let (project, parent, child, document, board, task) =
        seed_project_tree(&db, &ctx, "folder").await;

    db.folder_repo()
        .soft_delete(&ctx, parent.id)
        .await
        .expect("delete folder");

    assert!(
        db.project_repo()
            .find(&ctx, project.id)
            .await
            .expect("find project")
            .is_some()
    );
    assert!(
        db.folder_repo()
            .find(&ctx, child.id)
            .await
            .expect("find folder")
            .is_none()
    );
    assert!(
        db.doc_repo()
            .get(&ctx, document.id)
            .await
            .expect("get document")
            .is_none()
    );
    assert!(
        db.board_repo()
            .find_board(&ctx, board.id)
            .await
            .expect("find board")
            .is_none()
    );
    assert!(
        db.task_repo()
            .find(&ctx, task.id)
            .await
            .expect("find task")
            .is_none()
    );
    assert!(
        db.folder_repo()
            .list_all(&ctx)
            .await
            .expect("list folders")
            .iter()
            .all(|folder| folder.id != child.id)
    );
    assert!(
        db.doc_repo()
            .list_in_folder(&ctx, child.id)
            .await
            .expect("list documents")
            .is_empty()
    );
    assert!(
        db.doc_repo()
            .list_visible(&ctx, &Principal::User(user.id), Some(project.id), None, 50)
            .await
            .expect("list visible documents")
            .is_empty()
    );
    assert!(
        db.board_repo()
            .list_boards_in_folder(&ctx, child.id)
            .await
            .expect("list boards")
            .is_empty()
    );
    assert!(
        db.task_repo()
            .list_by_column(&ctx, task.column_id)
            .await
            .expect("list tasks")
            .is_empty()
    );
    assert!(
        db.task_repo()
            .list_by_workspace_filtered(&ctx, &TaskViewFilters::default(), None, 50)
            .await
            .expect("list workspace tasks")
            .is_empty()
    );
    assert_descendants_remain_live(&db, child.id.0, document.id.0, board.id.0, task.id.0).await;

    db.teardown().await;
}

#[tokio::test]
async fn owner_chain_visibility() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "owner-chain-visibility").await;
    let ctx = support::ctx(&ws, &user);

    let (project, _parent, _child, document, _board, task) =
        seed_project_tree(&db, &ctx, "project").await;

    let comment_repo = PgCommentRepo::new(db.conn().clone());
    let task_comment = comment_repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "Task comment below deleted project".into(),
            },
        )
        .await
        .expect("create task comment");
    let document_comment = comment_repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Document(document.id),
                body: "Document comment below deleted project".into(),
            },
        )
        .await
        .expect("create document comment");

    let attachment_repo = PgAttachmentRepo {
        conn: db.conn().clone(),
    };
    let document_attachment = attachment_repo
        .record(
            &ctx,
            NewAttachment {
                document_id: Some(document.id),
                task_id: None,
                comment_id: None,
                file_name: "document.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "document-digest".into(),
            },
        )
        .await
        .expect("record document attachment");
    let task_attachment = attachment_repo
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: Some(task.id),
                comment_id: None,
                file_name: "task.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "task-digest".into(),
            },
        )
        .await
        .expect("record task attachment");
    let comment_attachment = attachment_repo
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: None,
                comment_id: Some(task_comment.id),
                file_name: "comment.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "comment-digest".into(),
            },
        )
        .await
        .expect("record comment attachment");

    db.project_repo()
        .soft_delete(&ctx, project.id)
        .await
        .expect("delete project");

    assert!(
        comment_repo
            .get_for_owner(&ctx, CommentOwner::Task(task.id), task_comment.id)
            .await
            .is_err()
    );
    assert!(
        comment_repo
            .get_for_owner(
                &ctx,
                CommentOwner::Document(document.id),
                document_comment.id,
            )
            .await
            .is_err()
    );
    assert!(
        comment_repo
            .list_for_owner(&ctx, CommentOwner::Task(task.id), None, 50)
            .await
            .expect("list task comments")
            .is_empty()
    );
    assert!(
        comment_repo
            .list_for_owner(&ctx, CommentOwner::Document(document.id), None, 50)
            .await
            .expect("list document comments")
            .is_empty()
    );
    assert!(
        attachment_repo
            .find(&ctx, document_attachment.id)
            .await
            .expect("find document attachment")
            .is_none()
    );
    assert!(
        attachment_repo
            .find(&ctx, task_attachment.id)
            .await
            .expect("find task attachment")
            .is_none()
    );
    assert!(
        attachment_repo
            .find(&ctx, comment_attachment.id)
            .await
            .expect("find comment attachment")
            .is_none()
    );
    assert!(
        attachment_repo
            .list_for_owner(&ctx, AttachmentOwner::Document(document.id))
            .await
            .expect("list document attachments")
            .is_empty()
    );
    assert!(
        attachment_repo
            .list_for_owner(&ctx, AttachmentOwner::Task(task.id))
            .await
            .expect("list task attachments")
            .is_empty()
    );
    assert!(
        attachment_repo
            .list_for_owner(&ctx, AttachmentOwner::Comment(task_comment.id))
            .await
            .expect("list comment attachments")
            .is_empty()
    );
    assert!(db.doc_repo().history(&ctx, document.id).await.is_err());
    assert!(
        db.doc_repo()
            .content_at(&ctx, document.id, 1)
            .await
            .is_err()
    );

    db.teardown().await;
}

#[tokio::test]
async fn owner_chain_visibility_hides_folder_descendants() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "owner-chain-folder-visibility").await;
    let ctx = support::ctx(&ws, &user);

    let (_project, parent, _child, document, _board, task) =
        seed_project_tree(&db, &ctx, "folder").await;

    let comment_repo = PgCommentRepo::new(db.conn().clone());
    let task_comment = comment_repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "Task comment below deleted folder".into(),
            },
        )
        .await
        .expect("create task comment");
    let attachment_repo = PgAttachmentRepo {
        conn: db.conn().clone(),
    };
    let comment_attachment = attachment_repo
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: None,
                comment_id: Some(task_comment.id),
                file_name: "comment.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "folder-comment-digest".into(),
            },
        )
        .await
        .expect("record comment attachment");

    db.folder_repo()
        .soft_delete(&ctx, parent.id)
        .await
        .expect("delete parent folder");

    assert!(
        comment_repo
            .get_for_owner(&ctx, CommentOwner::Task(task.id), task_comment.id)
            .await
            .is_err()
    );
    assert!(
        attachment_repo
            .find(&ctx, comment_attachment.id)
            .await
            .expect("find comment attachment")
            .is_none()
    );
    assert!(db.doc_repo().history(&ctx, document.id).await.is_err());
    assert!(
        db.doc_repo()
            .content_at(&ctx, document.id, 1)
            .await
            .is_err()
    );

    db.teardown().await;
}

#[tokio::test]
async fn owner_chain_download_visibility() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "owner-chain-download").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let (project, _parent, _child, document, _board, task) =
        seed_project_tree(&db, &ctx, "project").await;

    let comment_repo = PgCommentRepo::new(db.conn().clone());
    let comment = comment_repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "Comment attachment below deleted project".into(),
            },
        )
        .await
        .expect("create task comment");
    let attachment_repo = PgAttachmentRepo {
        conn: db.conn().clone(),
    };
    let document_attachment = attachment_repo
        .record(
            &ctx,
            NewAttachment {
                document_id: Some(document.id),
                task_id: None,
                comment_id: None,
                file_name: "document.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "download-document-digest".into(),
            },
        )
        .await
        .expect("record document attachment");
    let task_attachment = attachment_repo
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: Some(task.id),
                comment_id: None,
                file_name: "task.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "download-task-digest".into(),
            },
        )
        .await
        .expect("record task attachment");
    let comment_attachment = attachment_repo
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: None,
                comment_id: Some(comment.id),
                file_name: "comment.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "download-comment-digest".into(),
            },
        )
        .await
        .expect("record comment attachment");

    db.project_repo()
        .soft_delete(&ctx, project.id)
        .await
        .expect("delete project");

    for url in [
        format!(
            "{}/api/workspaces/{}/attachments/{}",
            server.base_url(),
            ws.slug,
            document_attachment.id.0
        ),
        format!(
            "{}/api/workspaces/{}/tasks/{}/attachments/{}/content",
            server.base_url(),
            ws.slug,
            task.readable_id,
            task_attachment.id.0
        ),
        format!(
            "{}/api/workspaces/{}/tasks/{}/comments/{}/attachments/{}/content",
            server.base_url(),
            ws.slug,
            task.readable_id,
            comment.id.0,
            comment_attachment.id.0
        ),
    ] {
        let response = client
            .http_client()
            .get(url)
            .bearer_auth(client.token().expect("authenticated token"))
            .send()
            .await
            .expect("download concealed attachment");
        assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    }

    db.teardown().await;
}
