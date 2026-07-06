#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! E2E test: GitHub webhook delivery → external event → automation rule → task
//! created → signed outgoing webhook dispatched (B5.1).

mod support;

use std::sync::{Arc, Mutex};

use hmac::{Hmac, Mac};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};
use sha2::Sha256;
use uuid::Uuid;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::boards_tasks::{NewBoard, PositionBetween},
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    config::DispatcherConfig,
    dispatcher::{WebhookDispatcher, compute_signature},
    persistence::{
        entities::boards_tasks::{task, task_activity},
        repos::{BoardRepo, PgAutomationRuleRepo, PgBoardRepo, PgProjectRepo, ProjectRepo},
    },
    state::AppState,
};
use axum::{Router, extract::State, routing::post};
use serde_json::Value;

type HmacSha256 = Hmac<Sha256>;

fn github_sig(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256={hex}")
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

// ---------------------------------------------------------------------------
// Mini mock receiver (same pattern as e2e_webhooks.rs)
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
// B5.1: Full loop — GitHub delivery → rule → task → signed outgoing webhook
// ---------------------------------------------------------------------------

#[tokio::test]
async fn e2e_github_workflow_run_fires_automation_and_dispatches_webhook() {
    let db = support::TestDb::create().await.expect("TestDb");

    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let crypto = Arc::clone(&state.webhook_crypto);
    let server = support::TestServer::spawn_with_state(state).await;

    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "e2e-auto-admin").await;
    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    // Seed project, board, and column so the automation rule has valid targets.
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewProject {
            name: "E2E Auto Project".into(),
            slug: "e2e-auto-proj".into(),
            task_prefix: "EAP".into(),
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
                name: "CI Board".into(),
            },
        )
        .await
        .expect("create board");

    let column = board_repo
        .add_column(
            &ctx,
            board.id,
            "Failures".into(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("add column");

    // Spawn mock HTTP receiver for outgoing task.created webhook.
    let (mock_url, mock_state, _abort) = spawn_mock_receiver().await;

    // Create integration config via HTTP API — the plaintext secret is returned once.
    let config_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .expect("create config POST");
    assert_eq!(
        config_resp.status(),
        201,
        "integration config creation must succeed"
    );
    let config_body: Value = config_resp.json().await.unwrap();
    let integration_secret = config_body["secret"].as_str().unwrap().to_string();
    let integration_api_key_id =
        Uuid::parse_str(config_body["integration_api_key_id"].as_str().unwrap()).unwrap();

    // Create outgoing webhook subscription scoped to task.created events.
    let wh_resp = http()
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
    assert_eq!(
        wh_resp.status(),
        201,
        "webhook subscription creation must succeed"
    );
    let wh_body: Value = wh_resp.json().await.unwrap();
    let wh_secret = wh_body["secret"].as_str().unwrap().to_string();

    // Create automation rule: on workflow_run with conclusion=failure → create task.
    let txn = db.conn().begin().await.expect("begin txn");
    PgAutomationRuleRepo::create(
        &txn,
        ws.id.0,
        "CI failure → create task".to_string(),
        "external.github.workflow_run".to_string(),
        Some(serde_json::json!({ "conclusion": "failure" })),
        None,
        "create_task".to_string(),
        serde_json::json!({
            "board_id": board.id.0,
            "column_id": column.id.0,
            "title_template": "CI failed: {{workflow_name}}"
        }),
        user.id.0,
    )
    .await
    .expect("create rule");
    txn.commit().await.expect("commit txn");

    // Build a realistic GitHub workflow_run delivery body.
    let body_json = serde_json::json!({
        "action": "completed",
        "workflow_run": {
            "id": 99001,
            "name": "Build and Test",
            "conclusion": "failure",
            "html_url": "https://github.com/owner/repo/actions/runs/99001",
            "head_branch": "main",
            "event": "push"
        },
        "repository": { "full_name": "owner/repo" }
    });
    let body_bytes = serde_json::to_vec(&body_json).unwrap();
    let delivery_id = Uuid::now_v7();
    let ingest_sig = github_sig(integration_secret.as_bytes(), &body_bytes);

    // POST the GitHub delivery to the ingestion endpoint.
    let ingest_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", &ingest_sig)
        .header("x-github-delivery", delivery_id.to_string())
        .header("x-github-event", "workflow_run")
        .header("content-type", "application/json")
        .body(body_bytes.clone())
        .send()
        .await
        .expect("ingest POST");
    assert_eq!(ingest_resp.status(), 200, "valid ingest must return 200");

    // Assert that exactly one task was created on the target board by the integration key.
    let tasks = task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ws.id.0))
        .filter(task::Column::BoardId.eq(board.id.0))
        .all(db.conn())
        .await
        .expect("query tasks");
    assert_eq!(
        tasks.len(),
        1,
        "one task must be created by the automation rule"
    );
    assert_eq!(tasks[0].title, "CI failed: Build and Test");
    assert!(
        tasks[0].created_by_api_key_id.is_some(),
        "task must be attributed to the integration api key"
    );
    assert_eq!(
        tasks[0].created_by_api_key_id.unwrap(),
        integration_api_key_id,
        "task attribution must match the integration config's provisioned api key"
    );
    assert_eq!(
        tasks[0].column_id, column.id.0,
        "task must land in the rule's target column"
    );

    let activities = task_activity::Entity::find()
        .filter(task_activity::Column::WorkspaceId.eq(ws.id.0))
        .filter(task_activity::Column::TaskId.eq(tasks[0].id))
        .all(db.conn())
        .await
        .expect("query task activity");
    assert_eq!(
        activities.len(),
        1,
        "automation-created task must have a task_activity entry"
    );
    assert_eq!(activities[0].kind, "created");
    assert_eq!(
        activities[0].created_by_api_key_id,
        Some(integration_api_key_id),
        "task_activity attribution must match the integration api key"
    );

    // Run the dispatcher — both outbox rows (external event + task.created) are
    // processed; the subscription only matches task.created, so the mock receives
    // exactly one signed POST.
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
    dispatcher
        .poll_and_dispatch()
        .await
        .expect("poll_and_dispatch");

    // Assert mock receiver got exactly one POST for task.created, with a valid signature.
    let (req_count, body_text, signature_header) = {
        let reqs = mock_state.requests.lock().unwrap();
        let count = reqs.len();
        let body = reqs.first().map(|(b, _)| b.clone()).unwrap_or_default();
        let sig = reqs.first().and_then(|(_, s)| s.clone());
        (count, body, sig)
    };

    assert_eq!(
        req_count, 1,
        "mock must receive exactly one POST (task.created)"
    );

    let sig_header = signature_header.expect("x-atlas-signature header must be present");
    assert!(
        sig_header.starts_with("sha256="),
        "signature must start with sha256=: {sig_header}"
    );

    let expected_sig = compute_signature(wh_secret.as_bytes(), body_text.as_bytes())
        .expect("compute expected signature");
    assert_eq!(
        sig_header, expected_sig,
        "x-atlas-signature must verify against the webhook subscription secret"
    );

    let payload: Value = serde_json::from_str(&body_text).expect("payload must be valid JSON");
    assert_eq!(
        payload["event_type"], "task.created",
        "delivered event must be task.created"
    );

    // Dedup: re-POST the same X-GitHub-Delivery GUID → 200, but no second task.
    let dup_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", &ingest_sig)
        .header("x-github-delivery", delivery_id.to_string())
        .header("x-github-event", "workflow_run")
        .header("content-type", "application/json")
        .body(body_bytes)
        .send()
        .await
        .expect("duplicate ingest POST");
    assert_eq!(dup_resp.status(), 200, "duplicate delivery must return 200");

    let tasks_after_dup = task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ws.id.0))
        .filter(task::Column::BoardId.eq(board.id.0))
        .all(db.conn())
        .await
        .expect("query tasks after dup");
    assert_eq!(
        tasks_after_dup.len(),
        1,
        "duplicate delivery must not create a second task"
    );

    db.teardown().await;
}
