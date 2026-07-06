#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! E2E test: HTTP API → outbox → dispatcher → mock receiver (B4.6).

mod support;

use std::sync::{Arc, Mutex};

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::{NewBoard, PositionBetween},
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    config::DispatcherConfig,
    dispatcher::{WebhookDispatcher, compute_signature},
    persistence::repos::{BoardRepo, PgBoardRepo, PgProjectRepo, ProjectRepo},
    state::AppState,
};
use axum::{Router, extract::State, routing::post};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Mini mock receiver (same pattern as dispatcher.rs tests)
// ---------------------------------------------------------------------------

type ReceivedRequests = Arc<Mutex<Vec<(String, Option<String>)>>>;

#[derive(Clone, Default)]
struct MockState {
    requests: ReceivedRequests,
}

async fn mock_handler(
    State(mock): State<MockState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    let body_str = String::from_utf8_lossy(&body).to_string();
    let signature = headers
        .get("x-atlas-signature")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    mock.requests.lock().unwrap().push((body_str, signature));
    axum::http::StatusCode::OK
}

async fn spawn_mock_receiver() -> (String, MockState, tokio::task::AbortHandle) {
    let mock_state = MockState::default();
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
// B4.6: full-stack E2E — create webhook via API, create task via API,
//        run dispatcher, assert mock receiver received the signed POST
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_webhook_dispatched_on_task_creation() {
    let db = support::TestDb::create().await.expect("TestDb");

    // Build state manually so we can share the crypto instance with the dispatcher.
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let crypto = Arc::clone(&state.webhook_crypto);
    let server = support::TestServer::spawn_with_state(state).await;

    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "e2e-wh-admin").await;
    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let (mock_url, mock_state, _abort) = spawn_mock_receiver().await;

    let create_resp = reqwest::Client::new()
        .post(format!("{base_url}/v1/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": mock_url,
            "event_types": ["task.created"],
            "scope_type": "workspace"
        }))
        .send()
        .await
        .expect("create webhook POST");

    assert_eq!(create_resp.status(), 201, "webhook creation must succeed");
    let created: Value = create_resp.json().await.unwrap();
    let plaintext_secret = created["secret"].as_str().unwrap().to_string();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    let project_repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    let project = project_repo
        .create(
            &ctx,
            NewProject {
                name: "E2E Project".into(),
                slug: "e2e-proj".into(),
                task_prefix: "E2E".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");

    let board_repo = PgBoardRepo::new(db.conn().clone());
    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                project_id: project.id,
                name: "E2E Board".into(),
            },
        )
        .await
        .expect("create board");

    let column = board_repo
        .add_column(
            &ctx,
            board.id,
            "Backlog".into(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("add column");

    let board_id = board.id;
    let task_resp = reqwest::Client::new()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/boards/{board_id}/tasks"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "column_id": column.id,
            "title": "E2E webhook test task"
        }))
        .send()
        .await
        .expect("create task POST");

    assert_eq!(task_resp.status(), 201, "task creation must succeed");

    let dispatcher = WebhookDispatcher::new(
        db.conn().clone(),
        Arc::clone(&crypto),
        DispatcherConfig {
            poll_interval_ms: 50,
            max_attempts: 3,
            delivery_timeout_ms: 5_000,
            max_concurrent: 4,
            batch_size: 10,
            lease_secs: 30,
        },
        true,
    );

    dispatcher.poll_and_dispatch().await.expect("dispatch");

    let (req_count, body_text, signature_header) = {
        let reqs = mock_state.requests.lock().unwrap();
        let count = reqs.len();
        let body = reqs.first().map(|(b, _)| b.clone()).unwrap_or_default();
        let sig = reqs.first().and_then(|(_, s)| s.clone());
        (count, body, sig)
    };

    assert_eq!(req_count, 1, "mock must receive exactly one POST");

    let sig = signature_header.expect("x-atlas-signature header must be present");
    assert!(
        sig.starts_with("sha256="),
        "signature must start with sha256=: {sig}"
    );

    let expected_sig = compute_signature(plaintext_secret.as_bytes(), body_text.as_bytes())
        .expect("compute expected signature");
    assert_eq!(
        sig, expected_sig,
        "received HMAC signature must match plaintext secret from create response"
    );

    let payload: Value = serde_json::from_str(&body_text).expect("payload must be valid JSON");
    assert_eq!(
        payload["event_type"], "task.created",
        "event_type must be task.created"
    );

    db.teardown().await;
}
