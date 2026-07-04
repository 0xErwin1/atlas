#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::events::{DomainEvent, TaskCreatedPayload},
    ids::{BoardId, ColumnId, ProjectId, TaskId},
};
use atlas_server::live::{LiveEvent, LiveEventHub};
use atlas_server::persistence::repos::PgOutboxRepo;
use sea_orm::TransactionTrait;
use tokio::sync::watch;
use uuid::Uuid;

fn task_created_event() -> DomainEvent {
    DomainEvent::TaskCreated(TaskCreatedPayload {
        task_id: TaskId::new(),
        title: "Live task".into(),
        project_id: ProjectId::new(),
        board_id: BoardId::new(),
        column_id: ColumnId::new(),
    })
}

// ---------------------------------------------------------------------------
// A committed outbox insert fires NOTIFY, and the listener forwards it to a
// hub subscriber as a LiveEvent carrying the expected routing fields.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn listener_forwards_committed_outbox_event_to_subscriber() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "live-forward").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    let pool = db.conn().get_postgres_connection_pool().clone();
    let hub = LiveEventHub::new(16);
    let mut subscriber = hub.subscribe();

    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = tokio::spawn(atlas_server::live::run_listener(pool, hub, shutdown_rx));

    // NOTIFY only reaches a session that has already issued LISTEN, and the
    // event fires on commit. Give the listener a moment to subscribe before the
    // committing insert so the notification is delivered rather than missed.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let event = task_created_event();
    let event_type = event.event_type();

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, event)
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    let received = tokio::time::timeout(Duration::from_secs(5), subscriber.recv())
        .await
        .expect("live event within timeout")
        .expect("broadcast recv");

    assert_eq!(
        received.workspace_id, ws.id.0,
        "forwarded event must carry the workspace id"
    );
    assert_eq!(
        received.event_type, event_type,
        "forwarded event must carry the event type"
    );
    assert!(
        !received.payload.is_empty(),
        "forwarded event must carry the raw envelope JSON"
    );

    handle.abort();
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Publishing with no active subscribers is a no-op, not an error.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn publish_without_subscribers_does_not_error() {
    let hub = LiveEventHub::new(8);

    hub.publish(LiveEvent {
        workspace_id: Uuid::now_v7(),
        project_id: None,
        board_id: None,
        event_type: "task.created".into(),
        payload: Arc::from("{}"),
    });
}
