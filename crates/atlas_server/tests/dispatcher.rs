#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::{Arc, Mutex};

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::events::{DomainEvent, TaskCreatedPayload},
    ids::{BoardId, ColumnId, ProjectId, TaskId},
};
use atlas_server::{
    config::DispatcherConfig,
    crypto::WebhookCrypto,
    dispatcher::WebhookDispatcher,
    persistence::{
        entities::events_outbox::event_outbox,
        repos::{PgOutboxRepo, PgWebhookSubscriptionRepo},
    },
};
use axum::{Router, extract::State, routing::post};
use sea_orm::{EntityTrait, TransactionTrait};
use tokio::sync::watch;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A crypto instance with a fixed key shared across a test.
fn test_crypto() -> Arc<WebhookCrypto> {
    Arc::new(WebhookCrypto::new(&[0x42u8; 32]))
}

/// Plaintext secret used in all dispatcher integration tests.
const TEST_SECRET: &[u8] = b"integration-test-webhook-secret!";

fn task_created_event() -> DomainEvent {
    DomainEvent::TaskCreated(TaskCreatedPayload {
        task_id: TaskId::new(),
        title: "Dispatcher test task".into(),
        project_id: ProjectId::new(),
        board_id: BoardId::new(),
        column_id: ColumnId::new(),
    })
}

/// Minimal DispatcherConfig for tests: 1 max attempt so failure → dead in one pass.
fn one_attempt_config() -> DispatcherConfig {
    DispatcherConfig {
        poll_interval_ms: 50,
        max_attempts: 1,
        delivery_timeout_ms: 5_000,
        max_concurrent: 4,
        batch_size: 10,
        lease_secs: 30,
    }
}

/// DispatcherConfig for happy-path tests (5 attempts, won't be needed).
fn default_test_config() -> DispatcherConfig {
    DispatcherConfig {
        poll_interval_ms: 50,
        max_attempts: 5,
        delivery_timeout_ms: 5_000,
        max_concurrent: 4,
        batch_size: 10,
        lease_secs: 30,
    }
}

// ---------------------------------------------------------------------------
// Mock receiver
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<ReceivedRequest>>>,
    response_code: Arc<Mutex<u16>>,
}

struct ReceivedRequest {
    body: String,
    signature: Option<String>,
}

impl MockState {
    fn new(response_code: u16) -> Self {
        Self {
            requests: Arc::new(Mutex::new(vec![])),
            response_code: Arc::new(Mutex::new(response_code)),
        }
    }
}

async fn mock_handler(
    State(mock): State<MockState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl axum::response::IntoResponse {
    let body_str = String::from_utf8_lossy(&body).to_string();
    let signature = headers
        .get("x-atlas-signature")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    mock.requests.lock().unwrap().push(ReceivedRequest {
        body: body_str,
        signature,
    });

    let code = *mock.response_code.lock().unwrap();
    axum::http::StatusCode::from_u16(code).unwrap_or(axum::http::StatusCode::OK)
}

/// Spawns a mock HTTP receiver on a random port. Returns (url, state, abort_handle).
async fn spawn_mock_receiver(response_code: u16) -> (String, MockState, tokio::task::AbortHandle) {
    let mock_state = MockState::new(response_code);
    let app = Router::new()
        .route("/hook", post(mock_handler))
        .with_state(mock_state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock receiver");
    let addr = listener.local_addr().expect("local addr");
    let url = format!("http://{addr}/hook");

    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("mock receiver serve");
    });

    (url, mock_state, handle.abort_handle())
}

// ---------------------------------------------------------------------------
// B3.7-1: happy path — signed delivery, delivery log, outbox delivered
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatcher_delivers_event_and_marks_delivered() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "disp-happy").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let crypto = test_crypto();

    // Spawn mock receiver (always 200)
    let (url, mock_state, _abort) = spawn_mock_receiver(200).await;

    // Create subscription with encrypted secret
    let (encrypted_secret, secret_nonce) = crypto.encrypt(TEST_SECRET).expect("encrypt");
    PgWebhookSubscriptionRepo::create(
        db.conn(),
        ws.id.0,
        url.clone(),
        vec!["task.created".to_string()],
        "workspace".to_string(),
        None,
        encrypted_secret,
        secret_nonce,
        None,
        &Actor::User(user.id),
    )
    .await
    .expect("create subscription");

    // Insert one outbox event
    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    // Run one dispatch pass
    let dispatcher = WebhookDispatcher::new(
        db.conn().clone(),
        Arc::clone(&crypto),
        default_test_config(),
        true,
    );
    dispatcher
        .poll_and_dispatch()
        .await
        .expect("poll_and_dispatch");

    // Mock must have received exactly one POST. Clone data before dropping the guard
    // so no MutexGuard is held across subsequent await points.
    let (req_count, received_sig, received_body) = {
        let reqs = mock_state.requests.lock().unwrap();
        let count = reqs.len();
        let sig = reqs[0].signature.clone();
        let body = reqs[0].body.clone();
        (count, sig, body)
    };
    assert_eq!(req_count, 1, "mock must receive exactly one POST");

    // Verify the signature is present and correct
    let received_sig = received_sig.expect("signature header must be present");
    let expected_sig =
        atlas_server::dispatcher::compute_signature(TEST_SECRET, received_body.as_bytes())
            .expect("compute expected sig");
    assert_eq!(
        received_sig, expected_sig,
        "received signature must match expected HMAC"
    );

    // Outbox row must be delivered
    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "delivered", "outbox must be 'delivered'");

    // Delivery log must have one success row
    let log_rows =
        atlas_server::persistence::entities::webhook_delivery::webhook_delivery_log::Entity::find()
            .all(db.conn())
            .await
            .expect("find log");
    assert_eq!(log_rows.len(), 1, "delivery log must have one row");
    assert_eq!(log_rows[0].outcome, "success");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B3.7-2: retry-then-dead — receiver always returns 500, max_attempts=1
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dispatcher_marks_dead_on_exhausted_attempts() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "disp-dead").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let crypto = test_crypto();

    // Mock always returns 500
    let (url, mock_state, _abort) = spawn_mock_receiver(500).await;

    let (encrypted_secret, secret_nonce) = crypto.encrypt(TEST_SECRET).expect("encrypt");
    PgWebhookSubscriptionRepo::create(
        db.conn(),
        ws.id.0,
        url,
        vec!["task.created".to_string()],
        "workspace".to_string(),
        None,
        encrypted_secret,
        secret_nonce,
        None,
        &Actor::User(user.id),
    )
    .await
    .expect("create subscription");

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    // max_attempts=1 → dead after first failed pass
    let dispatcher = WebhookDispatcher::new(
        db.conn().clone(),
        Arc::clone(&crypto),
        one_attempt_config(),
        true,
    );
    dispatcher
        .poll_and_dispatch()
        .await
        .expect("poll_and_dispatch");

    // Mock received one POST
    let req_count = { mock_state.requests.lock().unwrap().len() };
    assert_eq!(req_count, 1, "mock must receive exactly one POST");

    // Outbox must be dead
    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "dead", "exhausted row must be 'dead'");

    // Delivery log must have one failure row
    let log_rows =
        atlas_server::persistence::entities::webhook_delivery::webhook_delivery_log::Entity::find()
            .all(db.conn())
            .await
            .expect("find log");
    assert_eq!(log_rows.len(), 1);
    assert_eq!(log_rows[0].outcome, "failure");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B3.7-3: delivery log per attempt (multiple attempts, each records a row)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delivery_log_records_each_attempt() {
    use sea_orm::ConnectionTrait;

    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "disp-log").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let crypto = test_crypto();

    let (url, _mock_state, _abort) = spawn_mock_receiver(500).await;

    let (encrypted_secret, secret_nonce) = crypto.encrypt(TEST_SECRET).expect("encrypt");
    PgWebhookSubscriptionRepo::create(
        db.conn(),
        ws.id.0,
        url,
        vec!["task.created".to_string()],
        "workspace".to_string(),
        None,
        encrypted_secret,
        secret_nonce,
        None,
        &Actor::User(user.id),
    )
    .await
    .expect("create subscription");

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    // Use 3 max attempts, run 3 passes (manipulate next_attempt_at between passes
    // so the row is immediately reclaimable).
    let config = DispatcherConfig {
        poll_interval_ms: 50,
        max_attempts: 3,
        delivery_timeout_ms: 5_000,
        max_concurrent: 4,
        batch_size: 10,
        lease_secs: 30,
    };
    let dispatcher = WebhookDispatcher::new(db.conn().clone(), Arc::clone(&crypto), config, true);

    // Pass 1
    dispatcher.poll_and_dispatch().await.expect("pass 1");

    // Reset next_attempt_at to now so the row is immediately reclaimable
    db.conn()
        .execute_unprepared(
            "UPDATE events_outbox SET next_attempt_at = NOW() - INTERVAL '1 second'",
        )
        .await
        .expect("reset next_attempt_at");

    // Pass 2
    dispatcher.poll_and_dispatch().await.expect("pass 2");

    db.conn()
        .execute_unprepared(
            "UPDATE events_outbox SET next_attempt_at = NOW() - INTERVAL '1 second'",
        )
        .await
        .expect("reset next_attempt_at");

    // Pass 3
    dispatcher.poll_and_dispatch().await.expect("pass 3");

    // Expect 3 delivery log rows (one per attempt)
    let log_rows =
        atlas_server::persistence::entities::webhook_delivery::webhook_delivery_log::Entity::find()
            .all(db.conn())
            .await
            .expect("find log");
    assert_eq!(
        log_rows.len(),
        3,
        "must have one delivery log row per attempt"
    );

    // All failures
    for r in &log_rows {
        assert_eq!(r.outcome, "failure");
    }

    // Outbox must be dead after 3 attempts with max_attempts=3
    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find");
    assert_eq!(rows[0].status, "dead");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B3.7-4: zero matching subscriptions → row marked delivered immediately
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_matching_subscriptions_marks_delivered() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "disp-nosubs").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let crypto = test_crypto();

    // No subscriptions created

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    let dispatcher = WebhookDispatcher::new(
        db.conn().clone(),
        Arc::clone(&crypto),
        default_test_config(),
        true,
    );
    dispatcher
        .poll_and_dispatch()
        .await
        .expect("poll_and_dispatch");

    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find");
    assert_eq!(rows[0].status, "delivered", "no matching subs → delivered");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B3.7-5: graceful drain — run() exits cleanly after shutdown signal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_exits_cleanly() {
    let db = support::TestDb::create().await.expect("TestDb");
    let crypto = test_crypto();

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let config = DispatcherConfig {
        poll_interval_ms: 20,
        max_attempts: 5,
        delivery_timeout_ms: 5_000,
        max_concurrent: 4,
        batch_size: 10,
        lease_secs: 30,
    };

    let dispatcher = WebhookDispatcher::new(db.conn().clone(), Arc::clone(&crypto), config, true);
    let handle = tokio::spawn(dispatcher.run(shutdown_rx));

    // Give the dispatcher one or two cycles
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    // Signal shutdown
    shutdown_tx.send(true).expect("send shutdown");

    // Await with a timeout — if the dispatcher hangs, the test fails
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;

    assert!(
        result.is_ok(),
        "dispatcher must exit within the timeout on shutdown"
    );
    let join_result = result.unwrap();
    assert!(join_result.is_ok(), "dispatcher task must not panic");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B3.7-6: scope matching — subscription scoped to a different board is skipped
// ---------------------------------------------------------------------------

#[tokio::test]
async fn out_of_scope_subscription_is_not_delivered() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "disp-scope").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let crypto = test_crypto();

    let (url, mock_state, _abort) = spawn_mock_receiver(200).await;

    // Subscription scoped to a specific board that is NOT the event's board
    let other_board_id = Uuid::new_v4();
    let (encrypted_secret, secret_nonce) = crypto.encrypt(TEST_SECRET).expect("encrypt");
    PgWebhookSubscriptionRepo::create(
        db.conn(),
        ws.id.0,
        url,
        vec!["task.created".to_string()],
        "board".to_string(),
        Some(other_board_id),
        encrypted_secret,
        secret_nonce,
        None,
        &Actor::User(user.id),
    )
    .await
    .expect("create subscription");

    // Event is for a different board (None board_id → no board match)
    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, task_created_event())
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    let dispatcher = WebhookDispatcher::new(
        db.conn().clone(),
        Arc::clone(&crypto),
        default_test_config(),
        true,
    );
    dispatcher
        .poll_and_dispatch()
        .await
        .expect("poll_and_dispatch");

    // Mock must NOT have received any request
    let req_count = { mock_state.requests.lock().unwrap().len() };
    assert_eq!(
        req_count, 0,
        "out-of-scope subscription must not receive delivery"
    );

    // Event still delivered (zero matching subs)
    let rows = event_outbox::Entity::find()
        .all(db.conn())
        .await
        .expect("find");
    assert_eq!(rows[0].status, "delivered");

    db.teardown().await;
}
