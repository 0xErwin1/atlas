#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::entities::boards_tasks::{NewBoard, NewTask, PositionBetween};
use atlas_domain::entities::workspace_core::NewProject;
use atlas_server::persistence::repos::{
    ApiKeyRepo, BoardRepo, PgBoardRepo, PgTaskRepo, ProjectRepo, TaskRepo,
};

#[tokio::test]
async fn api_key_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice").await;
    let (_ws_b, _user_b) = support::seed_workspace(&db, "bob").await;

    let ctx_a = support::ctx(&ws_a, &user_a);

    let repo = db.api_key_repo();

    let keys_a = repo.list(&ctx_a).await.expect("list keys for workspace A");
    assert!(
        keys_a.is_empty(),
        "workspace A must not see workspace B's api keys"
    );

    db.teardown().await;
}

#[tokio::test]
async fn project_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-proj").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-proj").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);
    let repo = db.project_repo();

    repo.create(
        &ctx_b,
        NewProject {
            name: "Bob's Project".into(),
            slug: "bobs-project".into(),
            task_prefix: "BP".into(),
        },
    )
    .await
    .expect("create project in ws_b");

    let projects_a = repo.list(&ctx_a).await.expect("list for ws_a");
    assert!(
        projects_a.is_empty(),
        "workspace A must not see workspace B's projects"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-task").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-task").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);

    let proj_repo = db.project_repo();
    let board_repo = PgBoardRepo::new(db.conn().clone());
    let task_repo = PgTaskRepo::new(db.conn().clone());

    let proj_b = proj_repo
        .create(
            &ctx_b,
            NewProject {
                name: "Bob Project".into(),
                slug: "bob-task-proj".into(),
                task_prefix: "BTKP".into(),
            },
        )
        .await
        .expect("create project in ws_b");

    let board_b = board_repo
        .create_board(
            &ctx_b,
            NewBoard {
                project_id: proj_b.id,
                name: "Main".into(),
            },
        )
        .await
        .expect("create board in ws_b");

    let col_b = board_repo
        .add_column(
            &ctx_b,
            board_b.id,
            "Backlog".into(),
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column in ws_b");

    task_repo
        .create(
            &ctx_b,
            NewTask {
                project_id: proj_b.id,
                board_id: board_b.id,
                column_id: col_b.id,
                title: "Bob's Task".into(),
                description: String::new(),
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("create task in ws_b");

    let tasks_a = task_repo
        .list_by_column(&ctx_a, col_b.id)
        .await
        .expect("list tasks from ws_a perspective");

    assert!(
        tasks_a.is_empty(),
        "workspace A must not see workspace B's tasks"
    );

    db.teardown().await;
}
