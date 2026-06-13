#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::{
    entities::boards_tasks::{NewBoard, NewTask, PositionBetween},
    entities::documents::{ExtractedLink, NewDocument},
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    BoardRepo, DocumentLinkRepo, DocumentRepo, PgBoardRepo, PgDocumentLinkRepo, PgDocumentRepo,
    PgProjectRepo, PgTaskRepo, ProjectRepo, TaskRepo,
};

async fn seed_project_board_task(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
) -> atlas_domain::entities::boards_tasks::Task {
    let project = PgProjectRepo { conn: db.conn().clone() }
        .create(
            ctx,
            NewProject {
                name: "DocLink Project".into(),
                slug: "dl-proj".into(),
                task_prefix: "DL".into(),
                visibility: Visibility::Workspace(VisibilityRole::Viewer),
            },
        )
        .await
        .expect("seed project");

    let board = PgBoardRepo::new(db.conn().clone())
        .create_board(ctx, NewBoard { project_id: project.id, name: "Main".into() })
        .await
        .expect("seed board");

    let col = PgBoardRepo::new(db.conn().clone())
        .add_column(ctx, board.id, "Backlog".into(), PositionBetween { before: None, after: None })
        .await
        .expect("seed column");

    PgTaskRepo::new(db.conn().clone())
        .create(
            ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: col.id,
                title: "Task with wikilinks".into(),
                description: "[[Target Doc]]".into(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween { before: None, after: None },
            },
        )
        .await
        .expect("seed task")
}

#[tokio::test]
async fn replace_for_task_source_stores_and_replaces_links() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "dlink-task-user").await;
    let ctx = support::ctx(&ws, &user);

    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 10);
    let link_repo = PgDocumentLinkRepo { conn: db.conn().clone() };

    let task = seed_project_board_task(&db, &ctx).await;

    let target_doc = doc_repo
        .create(
            &ctx,
            NewDocument {
                title: "Target Doc".into(),
                slug: None,
                content: "".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create target doc");

    // First write: one link.
    link_repo
        .replace_for_task_source(
            &ctx,
            task.id,
            vec![ExtractedLink {
                target_title: "Target Doc".into(),
                target_document_id: Some(target_doc.id),
            }],
        )
        .await
        .expect("replace_for_task_source (first write)");

    // Confirm backlinks from target_doc shows the task-sourced link.
    let backlinks = link_repo
        .backlinks(&ctx, target_doc.id)
        .await
        .expect("backlinks after first write");

    assert_eq!(backlinks.len(), 1, "one backlink after first write");
    assert!(
        backlinks[0].source_task_id.is_some(),
        "backlink must carry source_task_id"
    );
    assert!(
        backlinks[0].source_document_id.is_none(),
        "backlink must NOT carry source_document_id"
    );

    // Second write: replace with empty list (removing all links).
    link_repo
        .replace_for_task_source(&ctx, task.id, vec![])
        .await
        .expect("replace_for_task_source (clear)");

    let backlinks_after_clear = link_repo
        .backlinks(&ctx, target_doc.id)
        .await
        .expect("backlinks after clear");

    assert!(backlinks_after_clear.is_empty(), "links must be removed on second write");

    db.teardown().await;
}
