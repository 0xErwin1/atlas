#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    entities::boards_tasks::{NewBoard, PositionBetween},
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    BoardRepo, PgBoardRepo, PgProjectRepo, ProjectRepo, resequence_column,
};
use sea_orm::TransactionTrait;

async fn make_project(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    slug: &str,
    prefix: &str,
) -> atlas_domain::entities::workspace_core::Project {
    PgProjectRepo {
        conn: db.conn().clone(),
    }
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

    repo.create_board(
        &ctx,
        NewBoard {
            project_id: proj_a.id,
            name: "Board A1".into(),
        },
    )
    .await
    .expect("create board A1");
    repo.create_board(
        &ctx,
        NewBoard {
            project_id: proj_a.id,
            name: "Board A2".into(),
        },
    )
    .await
    .expect("create board A2");
    repo.create_board(
        &ctx,
        NewBoard {
            project_id: proj_b.id,
            name: "Board B1".into(),
        },
    )
    .await
    .expect("create board B1");

    let boards_a = repo
        .list_boards(&ctx, proj_a.id)
        .await
        .expect("list boards A");
    let boards_b = repo
        .list_boards(&ctx, proj_b.id)
        .await
        .expect("list boards B");

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

#[tokio::test]
async fn resequence_column_reorders_existing_keys() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "resequence-user").await;
    let ctx = support::ctx(&ws, &user);

    let proj = make_project(&db, &ctx, "resequence-proj", "RS").await;
    let board_repo = PgBoardRepo::new(db.conn().clone());

    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                project_id: proj.id,
                name: "B".into(),
            },
        )
        .await
        .expect("create board");

    // Create three columns in order.
    let col_a = board_repo
        .add_column(
            &ctx,
            board.id,
            "A".into(),
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("col A");
    let col_b = board_repo
        .add_column(
            &ctx,
            board.id,
            "B".into(),
            PositionBetween {
                before: Some(col_a.position_key.clone()),
                after: None,
            },
        )
        .await
        .expect("col B");
    let col_c = board_repo
        .add_column(
            &ctx,
            board.id,
            "C".into(),
            PositionBetween {
                before: Some(col_b.position_key.clone()),
                after: None,
            },
        )
        .await
        .expect("col C");

    // Resequence the column.
    let txn = db.conn().begin().await.expect("begin txn");
    resequence_column(&txn, &ctx, board.id)
        .await
        .expect("resequence");
    txn.commit().await.expect("commit");

    // After resequencing, list_columns must still return A < B < C by position.
    let cols = board_repo
        .list_columns(&ctx, board.id)
        .await
        .expect("list columns");

    assert_eq!(cols.len(), 3);
    assert_eq!(cols[0].id, col_a.id, "A must be first");
    assert_eq!(cols[1].id, col_b.id, "B must be second");
    assert_eq!(cols[2].id, col_c.id, "C must be third");
    assert!(cols[0].position_key < cols[1].position_key, "A < B");
    assert!(cols[1].position_key < cols[2].position_key, "B < C");

    db.teardown().await;
}

#[tokio::test]
async fn add_column_returns_position_exhausted_when_anchors_are_equal() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "col-add-reseq-user").await;
    let ctx = support::ctx(&ws, &user);

    let proj = make_project(&db, &ctx, "col-add-reseq-proj", "CAR").await;
    let board_repo = PgBoardRepo::new(db.conn().clone());

    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                project_id: proj.id,
                name: "Exhaustion Board".into(),
            },
        )
        .await
        .expect("create board");

    let anchor = board_repo
        .add_column(
            &ctx,
            board.id,
            "Anchor".into(),
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("anchor");

    // Passing equal before and after exhausts the fractional space deterministically.
    // try_between(k, k) returns None, triggering the resequence+retry path.
    // The retry also produces None for equal keys, so PositionExhausted is returned.
    // The old code would silently call position::between and return Ok with a wrong
    // position instead.
    let key = anchor.position_key.clone();
    let result = board_repo
        .add_column(
            &ctx,
            board.id,
            "Should fail".into(),
            PositionBetween {
                before: Some(key.clone()),
                after: Some(key.clone()),
            },
        )
        .await;

    assert!(
        matches!(
            result,
            Err(atlas_domain::DomainError::PositionExhausted { .. })
        ),
        "expected PositionExhausted for equal anchors, got: {result:?}"
    );

    db.teardown().await;
}
