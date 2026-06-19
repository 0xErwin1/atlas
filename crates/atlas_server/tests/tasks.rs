#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::entities::boards_tasks::{NewBoard, NewTask, PositionBetween};
use atlas_domain::entities::workspace_core::NewProject;
use atlas_domain::permissions::{Visibility, VisibilityRole};
use atlas_server::persistence::repos::{
    BoardRepo, PgBoardRepo, PgProjectRepo, PgTaskRepo, ProjectRepo, TaskRepo,
};
use sea_orm::TransactionTrait;

fn make_board_repo(db: &support::TestDb) -> PgBoardRepo {
    PgBoardRepo::new(db.conn().clone())
}

fn make_task_repo(db: &support::TestDb) -> PgTaskRepo {
    PgTaskRepo::new(db.conn().clone())
}

async fn seed_project(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    slug: &str,
    prefix: &str,
) -> atlas_domain::entities::workspace_core::Project {
    let repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    repo.create(
        ctx,
        NewProject {
            name: format!("Project {slug}"),
            slug: slug.into(),
            task_prefix: prefix.into(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("seed project")
}

async fn seed_board(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    project_id: atlas_domain::ids::ProjectId,
    name: &str,
) -> atlas_domain::entities::boards_tasks::Board {
    let repo = make_board_repo(db);
    repo.create_board(
        ctx,
        NewBoard {
            project_id,
            name: name.into(),
        },
    )
    .await
    .expect("seed board")
}

async fn seed_column(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    board_id: atlas_domain::ids::BoardId,
    name: &str,
    position: PositionBetween,
) -> atlas_domain::entities::boards_tasks::BoardColumn {
    let repo = make_board_repo(db);
    repo.add_column(ctx, board_id, name.into(), position)
        .await
        .expect("seed column")
}

#[tokio::test]
async fn readable_id_is_allocated_monotonically() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "task-mono-user").await;
    let ctx = support::ctx(&ws, &user);

    let project = seed_project(&db, &ctx, "task-mono", "TM").await;
    let board = seed_board(&db, &ctx, project.id, "Main").await;
    let col = seed_column(
        &db,
        &ctx,
        board.id,
        "Backlog",
        PositionBetween {
            before: None,
            after: None,
        },
    )
    .await;

    let task_repo = make_task_repo(&db);

    let mut ids = Vec::new();
    for i in 1_u32..=5 {
        let task = task_repo
            .create(
                &ctx,
                NewTask {
                    project_id: project.id,
                    board_id: board.id,
                    column_id: col.id,
                    title: format!("Task {i}"),
                    description: String::new(),
                    priority: None,
                    due_date: None,
                    estimate: None,
                    labels: vec![],
                    properties: None,
                    position: PositionBetween {
                        before: None,
                        after: None,
                    },
                },
            )
            .await
            .expect("create task");
        ids.push(task.readable_id.clone());
    }

    for (i, id) in ids.iter().enumerate() {
        let expected = format!("TM-{}", i + 1);
        assert_eq!(id, &expected, "readable_id must be monotonically allocated");
    }

    db.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn readable_id_no_collision_under_concurrency() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "task-concurrent-user").await;
    let ctx = support::ctx(&ws, &user);

    let project = seed_project(&db, &ctx, "task-conc", "TC").await;
    let board = seed_board(&db, &ctx, project.id, "Main").await;
    let col = seed_column(
        &db,
        &ctx,
        board.id,
        "Backlog",
        PositionBetween {
            before: None,
            after: None,
        },
    )
    .await;

    const N: u32 = 10;
    let task_repo = std::sync::Arc::new(make_task_repo(&db));

    let handles: Vec<_> = (1..=N)
        .map(|i| {
            let repo = task_repo.clone();
            let ctx = ctx.clone();
            let project_id = project.id;
            let board_id = board.id;
            let col_id = col.id;
            tokio::spawn(async move {
                repo.create(
                    &ctx,
                    NewTask {
                        project_id,
                        board_id,
                        column_id: col_id,
                        title: format!("Concurrent Task {i}"),
                        description: String::new(),
                        priority: None,
                        due_date: None,
                        estimate: None,
                        labels: vec![],
                        properties: None,
                        position: PositionBetween {
                            before: None,
                            after: None,
                        },
                    },
                )
                .await
                .expect("create concurrent task")
            })
        })
        .collect();

    let mut readable_ids: Vec<String> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.expect("join").readable_id)
        .collect();

    readable_ids.sort();
    readable_ids.dedup();

    assert_eq!(
        readable_ids.len(),
        N as usize,
        "all {N} readable_ids must be unique"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_position_key_ordering() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "task-pos-user").await;
    let ctx = support::ctx(&ws, &user);

    let project = seed_project(&db, &ctx, "task-pos", "TP").await;
    let board = seed_board(&db, &ctx, project.id, "Main").await;
    let col = seed_column(
        &db,
        &ctx,
        board.id,
        "Backlog",
        PositionBetween {
            before: None,
            after: None,
        },
    )
    .await;

    let task_repo = make_task_repo(&db);

    let task_a = task_repo
        .create(
            &ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: col.id,
                title: "Task A".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("create task A");

    let task_b = task_repo
        .create(
            &ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: col.id,
                title: "Task B".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: Some(task_a.position_key.clone()),
                    after: None,
                },
            },
        )
        .await
        .expect("create task B after A");

    assert!(
        task_b.position_key > task_a.position_key,
        "task B (after A) must have a larger position_key"
    );

    db.teardown().await;
}

#[tokio::test]
async fn move_to_single_update_succeeds_and_changes_column() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "move-task-user").await;
    let ctx = support::ctx(&ws, &user);

    let project = seed_project(&db, &ctx, "move-task", "MT").await;
    let board = seed_board(&db, &ctx, project.id, "Main").await;

    let col_a = seed_column(
        &db,
        &ctx,
        board.id,
        "Todo",
        PositionBetween {
            before: None,
            after: None,
        },
    )
    .await;
    let col_b = seed_column(
        &db,
        &ctx,
        board.id,
        "Done",
        PositionBetween {
            before: Some(col_a.position_key.clone()),
            after: None,
        },
    )
    .await;

    let task_repo = make_task_repo(&db);

    let task = task_repo
        .create(
            &ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: col_a.id,
                title: "T".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("create task");

    let moved = task_repo
        .move_to(
            &ctx,
            task.id,
            col_b.id,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("move task");

    assert_eq!(
        moved.column_id, col_b.id,
        "task must be in col_b after move"
    );
    assert_eq!(moved.id, task.id, "id must not change");

    db.teardown().await;
}

#[tokio::test]
async fn create_in_readable_id_counter_is_monotonic_in_outer_txn() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "create-in-user").await;
    let ctx = support::ctx(&ws, &user);

    let project = seed_project(&db, &ctx, "create-in", "CI").await;
    let board = seed_board(&db, &ctx, project.id, "Main").await;
    let col = seed_column(
        &db,
        &ctx,
        board.id,
        "Backlog",
        PositionBetween {
            before: None,
            after: None,
        },
    )
    .await;

    let txn = db.conn().begin().await.expect("begin outer txn");

    let task = PgTaskRepo::create_in(
        &txn,
        &ctx,
        NewTask {
            project_id: project.id,
            board_id: board.id,
            column_id: col.id,
            title: "Inner".into(),
            description: String::new(),
            priority: None,
            due_date: None,
            estimate: None,
            labels: vec![],
            properties: None,
            position: PositionBetween {
                before: None,
                after: None,
            },
        },
        None,
    )
    .await
    .expect("create_in");

    txn.commit().await.expect("commit outer txn");

    assert_eq!(
        task.readable_id, "CI-1",
        "first task in outer txn must be CI-1"
    );

    db.teardown().await;
}
