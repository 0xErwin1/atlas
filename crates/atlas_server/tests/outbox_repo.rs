#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::events::{DomainEvent, TaskCreatedPayload},
    ids::{BoardId, ColumnId, ProjectId, TaskId, WorkspaceId},
};
use atlas_server::persistence::{
    entities::events_outbox::event_outbox,
    repos::PgOutboxRepo,
};
use sea_orm::{EntityTrait, TransactionTrait};
use uuid::Uuid;

fn make_ctx(ws_id: WorkspaceId, user: &atlas_server::persistence::repos::User) -> WorkspaceCtx {
    WorkspaceCtx::new(ws_id, Actor::User(user.id))
}

fn task_created_event() -> DomainEvent {
    DomainEvent::TaskCreated(TaskCreatedPayload {
        task_id: TaskId::new(),
        title: "Test task".into(),
        project_id: ProjectId::new(),
        board_id: BoardId::new(),
        column_id: ColumnId::new(),
    })
}

// ---------------------------------------------------------------------------
// B2.2-1 — insert_in writes exactly one pending row
// ---------------------------------------------------------------------------

#[tokio::test]
async fn insert_in_writes_one_pending_row() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ob-insert").await;
    let ctx = make_ctx(ws.id, &user);

    let event = task_created_event();
    let event_type = event.event_type();

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, event)
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find all");

    assert_eq!(rows.len(), 1, "exactly one outbox row must be written");
    assert_eq!(rows[0].status, "pending");
    assert_eq!(rows[0].event_type, event_type);
    assert_eq!(rows[0].attempt_count, 0);
    assert_eq!(rows[0].workspace_id, ws.id.0);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B2.2-2 — rolled-back txn leaves zero rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rollback_leaves_no_rows() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ob-rollback").await;
    let ctx = make_ctx(ws.id, &user);

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
        .await
        .expect("insert_in");
    txn.rollback().await.expect("rollback");

    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find all");

    assert!(
        rows.is_empty(),
        "rolled-back transaction must not leave any outbox rows"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B2.2-3 — claim_batch marks rows as delivering and increments attempt_count
// ---------------------------------------------------------------------------

#[tokio::test]
async fn claim_batch_marks_delivering_and_increments_count() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ob-claim").await;
    let ctx = make_ctx(ws.id, &user);

    for _ in 0..3u8 {
        let txn = db.conn().begin().await.expect("begin");
        PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
            .await
            .expect("insert_in");
        txn.commit().await.expect("commit");
    }

    let claimed = PgOutboxRepo::claim_batch(db.conn(), 2, 30)
        .await
        .expect("claim_batch");

    assert_eq!(claimed.len(), 2, "claim_batch must return exactly the requested batch_size");
    for row in &claimed {
        assert_eq!(row.status, "delivering", "claimed rows must be 'delivering'");
        assert_eq!(row.attempt_count, 1, "claim_batch must increment attempt_count to 1");
        assert!(row.locked_until.is_some(), "claimed rows must have a locked_until timestamp");
    }

    let remaining = PgOutboxRepo::claim_batch(db.conn(), 5, 30)
        .await
        .expect("second claim_batch");
    assert_eq!(remaining.len(), 1, "second claim must return the remaining pending row");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B2.2-4 — recovery_sweep resets stale delivering rows to pending
// ---------------------------------------------------------------------------

#[tokio::test]
async fn recovery_sweep_resets_stale_delivering_rows() {
    use sea_orm::{ConnectionTrait, Statement};

    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ob-sweep").await;
    let ctx = make_ctx(ws.id, &user);

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    let claimed = PgOutboxRepo::claim_batch(db.conn(), 1, 60)
        .await
        .expect("claim_batch");
    assert_eq!(claimed.len(), 1);
    let row_id = claimed[0].id;

    // Force locked_until into the past so the row becomes stale.
    let stale_stmt = Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "UPDATE events_outbox SET locked_until = NOW() - INTERVAL '1 second' WHERE id = $1",
        [row_id.into()],
    );
    db.conn()
        .execute_raw(stale_stmt)
        .await
        .expect("set stale locked_until");

    let recovered = PgOutboxRepo::recovery_sweep(db.conn())
        .await
        .expect("recovery_sweep");
    assert_eq!(recovered, 1, "exactly one stale row must be recovered");

    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find all");

    assert_eq!(rows[0].status, "pending", "recovered row must be 'pending'");
    assert!(
        rows[0].locked_until.is_none(),
        "recovered row must have no locked_until"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B2.2-5 — finalize_event with subs_remaining=0 marks the row delivered
// ---------------------------------------------------------------------------

#[tokio::test]
async fn finalize_with_zero_subs_marks_delivered() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ob-finalize").await;
    let ctx = make_ctx(ws.id, &user);

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    let claimed = PgOutboxRepo::claim_batch(db.conn(), 1, 30)
        .await
        .expect("claim_batch");
    let row_id = claimed[0].id;

    PgOutboxRepo::finalize_event(db.conn(), row_id, 0, 5)
        .await
        .expect("finalize_event");

    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find all");

    assert_eq!(
        rows[0].status, "delivered",
        "zero subs_remaining must mark the row 'delivered'"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// insert_external_in — first insert returns true; row has correct shape
// ---------------------------------------------------------------------------

#[tokio::test]
async fn insert_external_in_returns_true_and_row_exists() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, _user) = support::seed_workspace(&db, "ob-ext-first").await;

    let delivery_id = Uuid::new_v4();
    let actor_key_id = Uuid::new_v4();

    let txn = db.conn().begin().await.expect("begin");
    let inserted = PgOutboxRepo::insert_external_in(
        &txn,
        delivery_id,
        ws.id.0,
        "external/github",
        "external.github.workflow_run",
        actor_key_id,
        serde_json::json!({"action": "completed", "conclusion": "failure"}),
    )
    .await
    .expect("insert_external_in");
    txn.commit().await.expect("commit");

    assert!(inserted, "first insert must return true");

    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find all");

    assert_eq!(rows.len(), 1, "exactly one outbox row must exist");
    assert_eq!(rows[0].id, delivery_id, "row id must equal delivery_id");
    assert_eq!(rows[0].source, "external/github");
    assert_eq!(rows[0].event_type, "external.github.workflow_run");
    assert_eq!(rows[0].aggregate_type, "external");
    assert_eq!(rows[0].aggregate_id, delivery_id, "aggregate_id must equal delivery_id");
    assert_eq!(rows[0].status, "pending");
    assert_eq!(rows[0].event_version, 1);
    assert!(rows[0].project_id.is_none());
    assert!(rows[0].board_id.is_none());

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// insert_external_in — duplicate delivery_id returns false; no second row
// ---------------------------------------------------------------------------

#[tokio::test]
async fn insert_external_in_duplicate_delivery_id_returns_false() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, _user) = support::seed_workspace(&db, "ob-ext-dup").await;

    let delivery_id = Uuid::new_v4();
    let actor_key_id = Uuid::new_v4();

    let txn = db.conn().begin().await.expect("begin");
    let first = PgOutboxRepo::insert_external_in(
        &txn,
        delivery_id,
        ws.id.0,
        "external/github",
        "external.github.workflow_run",
        actor_key_id,
        serde_json::json!({}),
    )
    .await
    .expect("first insert");
    txn.commit().await.expect("commit");

    assert!(first, "first insert must return true");

    let txn2 = db.conn().begin().await.expect("begin2");
    let second = PgOutboxRepo::insert_external_in(
        &txn2,
        delivery_id,
        ws.id.0,
        "external/github",
        "external.github.workflow_run",
        actor_key_id,
        serde_json::json!({}),
    )
    .await
    .expect("second insert");
    txn2.commit().await.expect("commit2");

    assert!(!second, "duplicate delivery_id must return false");

    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find all");

    assert_eq!(rows.len(), 1, "must not insert a second row on duplicate delivery_id");

    db.teardown().await;
}
