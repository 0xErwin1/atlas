#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization test for `PgBoardRepo`/`PgTaskRepo`'s current query
//! shapes, ported from `atlas_server::persistence::repos::boards_tasks`
//! before that code moves into this crate (S4 PR7, T7.1). Must keep passing
//! unmodified once the move lands (T7.4).
//!
//! `create_board` reads `workspace_status_templates` (already Acta-owned,
//! moved PR3) to derive default columns and writes to `events_outbox` via
//! the now-co-located `PgOutboxRepo` — both same-crate after this PR, so this
//! test doubles as the proof that pulling `outbox.rs` forward into this PR
//! did not change either query shape. `PgTaskRepo` methods also read
//! `custos.api_keys` by raw SQL for assignee display (design D6); this test
//! does not assign a task to an API key, so that path is exercised only by
//! the pre-existing `atlas_server` integration suite, not duplicated here.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::boards_tasks::{NewBoard, NewTask, PositionBetween};
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::ports::boards_tasks::{BoardRepo, TaskRepo};
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta_postgres::repos::boards_tasks::{PgBoardRepo, PgTaskRepo};
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_core::principal::UserId;
use atlas_custos::entities::identity::NewUser;
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use uuid::Uuid;

async fn seed_workspace_and_project(db: &TestDb, slug: &str) -> (WorkspaceId, Uuid, UserId) {
    let workspace_repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };
    let workspace_id = WorkspaceId(Uuid::now_v7());
    workspace_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: slug.to_string(),
            slug: slug.to_string(),
        })
        .await
        .expect("workspace must be created");

    let user_repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let user = user_repo
        .create(NewUser {
            username: slug.to_string(),
            display_name: slug.to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed user must be created");

    let project_id = Uuid::now_v7();
    sea_orm::ConnectionTrait::execute_unprepared(
        db.conn(),
        &format!(
            "INSERT INTO acta.projects \
             (id, workspace_id, name, slug, task_prefix, next_task_number, visibility, \
              created_by_user_id, created_at, updated_at) \
             VALUES ('{project_id}', '{}', '{slug}', '{slug}', 'TSK', 1, 'workspace', \
                     '{}', now(), now())",
            workspace_id.0, user.id.0
        ),
    )
    .await
    .expect("seed project");

    (workspace_id, project_id, user.id)
}

#[tokio::test]
async fn board_and_task_repo_create_find_and_list_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let (workspace_id, project_id, user_id) =
        seed_workspace_and_project(&db, "board-task-repo").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));

    let board_repo = PgBoardRepo::new(db.conn().clone());
    let task_repo = PgTaskRepo {
        conn: db.conn().clone(),
    };

    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                project_id: atlas_acta::ids::ProjectId(project_id),
                folder_id: None,
                name: "Sprint board".to_string(),
            },
        )
        .await
        .expect("board must be created");
    assert_eq!(board.name, "Sprint board");

    let found_board = board_repo
        .find_board(&ctx, board.id)
        .await
        .expect("find_board must not error")
        .expect("board must exist");
    assert_eq!(found_board.id, board.id);

    let boards = board_repo
        .list_boards(&ctx, atlas_acta::ids::ProjectId(project_id))
        .await
        .expect("list_boards must not error");
    assert_eq!(boards.len(), 1);

    let column = board_repo
        .add_column(
            &ctx,
            board.id,
            "Todo".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("add_column must succeed");

    let task = task_repo
        .create(
            &ctx,
            NewTask {
                project_id: atlas_acta::ids::ProjectId(project_id),
                board_id: board.id,
                column_id: column.id,
                title: "Ship S4 PR7".to_string(),
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
        .expect("task must be created");
    assert_eq!(task.title, "Ship S4 PR7");

    let found_task = task_repo
        .find(&ctx, task.id)
        .await
        .expect("find must not error")
        .expect("task must exist");
    assert_eq!(found_task.id, task.id);

    let by_readable_id = task_repo
        .find_by_readable_id(&ctx, &found_task.readable_id)
        .await
        .expect("find_by_readable_id must not error")
        .expect("task must resolve by readable id");
    assert_eq!(by_readable_id.id, task.id);

    let listed = task_repo
        .list_by_board(&ctx, board.id)
        .await
        .expect("list_by_board must not error");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, task.id);

    let by_column = task_repo
        .list_by_column(&ctx, column.id)
        .await
        .expect("list_by_column must not error");
    assert_eq!(by_column.len(), 1);

    db.teardown().await.expect("teardown must succeed");
}
