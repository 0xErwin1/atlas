#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Characterization coverage for the api-key revoke split (design D4).
//!
//! Revoking an api key must remove its task assignments in the same
//! transaction. Before the split, `PgApiKeyRepo::revoke_for_user_in` did both
//! writes itself. After the split, the Custos-side row update
//! (`revoke_for_user_in`, in `atlas_custos_postgres`) and the Acta-side
//! `task_assignees` cleanup (`PgTaskAssigneeRepo::unassign_api_key_in`, in
//! `atlas_server`) are two calls composed by the caller — see
//! `routes::api_keys::revoke_user_api_key` — but they must still commit or
//! roll back together exactly as they did as one function.
//!
//! `revoke_and_unassign` below reproduces that composition. These tests pin
//! its atomicity by driving both calls through an externally-owned
//! transaction and asserting on commit and on rollback. Real Postgres offers
//! no clean seam to inject a failure strictly between the two writes, so
//! rollback is used as the atomicity proof: if the two writes shared one
//! transaction, aborting it discards both; if they didn't, one would survive.
//! This is the permanent regression test for the split (design D4).

mod support;

use atlas_acta::entities::boards_tasks::AssigneeRef;
use atlas_acta::entities::boards_tasks::NewBoard;
use atlas_acta::entities::boards_tasks::NewTask;
use atlas_acta::entities::boards_tasks::NewTaskAssignee;
use atlas_acta::entities::boards_tasks::PositionBetween;
use atlas_acta::entities::workspace_core::NewProject;
use atlas_acta::ids::TaskId;
use atlas_acta::permissions::Visibility;
use atlas_acta::permissions::VisibilityRole;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::UserId;
use atlas_server::persistence::repos::{
    ApiKeyRepo, BoardRepo, NewApiKey, PgApiKeyRepo, PgBoardRepo, PgProjectRepo, PgTaskAssigneeRepo,
    PgTaskRepo, ProjectRepo, TaskAssigneeRepo, TaskRepo,
};
use sea_orm::{FromQueryResult, Statement, TransactionTrait};

/// Seeds a project/board/column/task and assigns a fresh, user-owned api key
/// to that task. Returns the key id and the task id.
async fn seed_task_with_api_key_assignee(
    db: &support::TestDb,
    ctx: &atlas_acta::actor::WorkspaceCtx,
    user_id: UserId,
) -> (ApiKeyId, TaskId) {
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        ctx,
        NewProject {
            name: "Revoke split project".into(),
            slug: "revoke-split".into(),
            task_prefix: "RS".into(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("seed project");

    let board = PgBoardRepo::new(db.conn().clone())
        .create_board(
            ctx,
            NewBoard {
                folder_id: None,
                project_id: project.id,
                name: "Main".into(),
            },
        )
        .await
        .expect("seed board");

    let col = PgBoardRepo::new(db.conn().clone())
        .add_column(
            ctx,
            board.id,
            "Backlog".into(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("seed column");

    let task = PgTaskRepo::new(db.conn().clone())
        .create(
            ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: col.id,
                title: "Task".into(),
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
        .expect("seed task");

    let key = db
        .api_key_repo()
        .create_for_user(
            user_id,
            NewApiKey {
                name: "assignee-key".into(),
                token_hash: "hash-revoke-split".into(),
                type_: atlas_custos::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_custos::capability::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("seed api key");

    PgTaskAssigneeRepo::new(db.conn().clone())
        .add(
            ctx,
            NewTaskAssignee {
                task_id: task.id,
                assignee: AssigneeRef::ApiKey(key.id),
            },
        )
        .await
        .expect("assign api key to task");

    (key.id, task.id)
}

/// Raw count of `task_assignees` rows still pointing at `key_id`, bypassing
/// `list_for_task`'s revoked-key filter so the row's physical presence is
/// what gets asserted, not its visibility.
async fn task_assignee_row_count(db: &support::TestDb, key_id: ApiKeyId) -> i64 {
    #[derive(FromQueryResult)]
    struct Row {
        count: i64,
    }

    Row::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT COUNT(*) AS count FROM acta.task_assignees WHERE assignee_api_key_id = $1",
        [key_id.0.into()],
    ))
    .one(db.conn())
    .await
    .expect("count task_assignees")
    .expect("count row")
    .count
}

/// Runs the same two calls `routes::api_keys::revoke_user_api_key` composes
/// in production: the Custos-side row update, then the Acta-side assignee
/// cleanup, both against `conn`. The caller decides whether to commit or roll
/// back `conn`.
async fn revoke_and_unassign(
    conn: &impl sea_orm::ConnectionTrait,
    user_id: UserId,
    key_id: ApiKeyId,
) {
    PgApiKeyRepo::revoke_for_user_in(conn, user_id, key_id)
        .await
        .expect("revoke_for_user_in");
    PgTaskAssigneeRepo::unassign_api_key_in(conn, key_id)
        .await
        .expect("unassign_api_key_in");
}

#[tokio::test]
async fn composed_revoke_commits_both_writes_atomically() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "revoke-commit-user").await;
    let ctx = support::ctx(&ws, &user);

    let (key_id, _task_id) = seed_task_with_api_key_assignee(&db, &ctx, user.id).await;
    assert_eq!(task_assignee_row_count(&db, key_id).await, 1);

    let txn = db.conn().begin().await.expect("begin txn");
    revoke_and_unassign(&txn, user.id, key_id).await;
    txn.commit().await.expect("commit txn");

    let key = db
        .api_key_repo()
        .get_by_id(key_id)
        .await
        .expect("get_by_id")
        .expect("key exists");
    assert!(key.revoked_at.is_some(), "key must be revoked after commit");
    assert_eq!(
        task_assignee_row_count(&db, key_id).await,
        0,
        "task assignment must be removed after commit"
    );
}

#[tokio::test]
async fn composed_revoke_rolls_back_both_writes_together() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "revoke-rollback-user").await;
    let ctx = support::ctx(&ws, &user);

    let (key_id, _task_id) = seed_task_with_api_key_assignee(&db, &ctx, user.id).await;
    assert_eq!(task_assignee_row_count(&db, key_id).await, 1);

    let txn = db.conn().begin().await.expect("begin txn");
    revoke_and_unassign(&txn, user.id, key_id).await;
    txn.rollback().await.expect("rollback txn");

    let key = db
        .api_key_repo()
        .get_by_id(key_id)
        .await
        .expect("get_by_id")
        .expect("key exists");
    assert!(
        key.revoked_at.is_none(),
        "key revoke must roll back with the assignment cleanup"
    );
    assert_eq!(
        task_assignee_row_count(&db, key_id).await,
        1,
        "task assignment must survive a rollback of the shared transaction"
    );
}
