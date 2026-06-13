#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::{
    entities::boards_tasks::NewBoard,
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{BoardRepo, PgBoardRepo, PgProjectRepo, ProjectRepo};

async fn make_project(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    slug: &str,
    prefix: &str,
) -> atlas_domain::entities::workspace_core::Project {
    PgProjectRepo { conn: db.conn().clone() }
        .create(
            ctx,
            NewProject {
                name: format!("Project {slug}"),
                slug: slug.into(),
                task_prefix: prefix.into(),
                visibility: Visibility::Workspace(VisibilityRole::Viewer),
            },
        )
        .await
        .expect("seed project")
}

#[tokio::test]
async fn list_boards_is_scoped_to_project() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "board-scope-user").await;
    let ctx = support::ctx(&ws, &user);

    let proj_a = make_project(&db, &ctx, "board-proj-a", "BA").await;
    let proj_b = make_project(&db, &ctx, "board-proj-b", "BB").await;

    let repo = PgBoardRepo::new(db.conn().clone());

    repo.create_board(&ctx, NewBoard { project_id: proj_a.id, name: "Board A1".into() })
        .await
        .expect("create board A1");
    repo.create_board(&ctx, NewBoard { project_id: proj_a.id, name: "Board A2".into() })
        .await
        .expect("create board A2");
    repo.create_board(&ctx, NewBoard { project_id: proj_b.id, name: "Board B1".into() })
        .await
        .expect("create board B1");

    let boards_a = repo.list_boards(&ctx, proj_a.id).await.expect("list boards A");
    let boards_b = repo.list_boards(&ctx, proj_b.id).await.expect("list boards B");

    assert_eq!(boards_a.len(), 2, "project A must have 2 boards");
    assert_eq!(boards_b.len(), 1, "project B must have 1 board");
    assert!(
        boards_a.iter().all(|b| b.project_id == proj_a.id),
        "all boards from list_boards(proj_a) must belong to proj_a"
    );
    assert!(
        boards_b.iter().all(|b| b.project_id == proj_b.id),
        "all boards from list_boards(proj_b) must belong to proj_b"
    );

    db.teardown().await;
}
