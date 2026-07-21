#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use atlas_domain::{
    entities::task_views::TaskViewFilters,
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        documents::NewDocument,
        workspace_core::{NewFolder, NewProject},
    },
    permissions::{Principal, Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    BoardRepo, DocumentRepo, FolderRepo, ProjectRepo, TaskRepo,
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
