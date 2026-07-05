//! Integration tests for the presence background tasks (work unit P2): the
//! agent-activity consumer and the TTL sweeper's broadcast wiring.
//!
//! The agent-consumer tests drive the full live pipeline exactly as `main` wires
//! it: a Postgres `LISTEN` consumer (`run_listener`) feeds the in-process hub from
//! committed outbox rows, and `run_presence_agent_consumer` subscribes to that hub.
//! An api-key principal that mutates a board is then shown present on it, and a
//! second mutation within the TTL does not re-broadcast (it is a refresh).
//!
//! The sweeper test avoids waiting the real 45s TTL: it drives the exact sequence
//! a sweeper tick performs (`PresenceRegistry::sweep` followed by
//! `broadcast_presence`) against a live SSE subscriber, seeding a presence
//! entry and sweeping with a zero TTL so every entry is treated as expired.
//!
//! All SSE reads use `tokio::time::timeout`; the stream is infinite, so tests read
//! only the frames they need and then drop the response.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::time::{Duration, Instant};

use atlas_api::dtos::{
    CreateGrantRequest, CreateProjectRequest, GrantPrincipal,
    boards_tasks::{CreateBoardRequest, CreateColumnRequest, CreateTaskRequest},
    documents::ActorDto,
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{boards_tasks::NewBoard, workspace_core::NewProject},
    ids::{BoardId, ProjectId},
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    persistence::repos::{
        ApiKeyRepo, BoardRepo, NewApiKey, PgBoardRepo, PgProjectRepo, ProjectRepo,
    },
    presence::{PresenceResource, broadcast_presence},
    state::AppState,
};
use tokio::sync::watch;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn events_url(base: &str, ws_slug: &str) -> String {
    format!("{base}/v1/workspaces/{ws_slug}/events")
}

/// Reads the next meaningful SSE frame (skipping keep-alive comments), returning
/// `(event_name, data)`. Returns `None` on timeout or end-of-body — never drains
/// to EOF, so it is safe against the infinite live stream.
async fn next_sse_event(
    resp: &mut reqwest::Response,
    buf: &mut String,
    timeout: Duration,
) -> Option<(String, String)> {
    loop {
        if let Some(idx) = buf.find("\n\n") {
            let frame: String = buf.drain(..idx + 2).collect();

            let mut event_name = String::new();
            let mut data = String::new();
            for line in frame.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    event_name = value.trim().to_string();
                } else if let Some(value) = line.strip_prefix("data:") {
                    data = value.trim().to_string();
                }
            }

            if event_name.is_empty() && data.is_empty() {
                continue;
            }

            return Some((event_name, data));
        }

        match tokio::time::timeout(timeout, resp.chunk()).await {
            Ok(Ok(Some(chunk))) => buf.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(Ok(None)) => return None,
            Ok(Err(_)) => return None,
            Err(_) => return None,
        }
    }
}

/// Reads frames until a `presence.updated` satisfies `predicate` or the timeout
/// elapses. Non-presence frames (e.g. the `task.created` that triggered it) are
/// skipped.
async fn wait_for_presence<F>(
    resp: &mut reqwest::Response,
    buf: &mut String,
    predicate: F,
) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    loop {
        let (name, data) = next_sse_event(resp, buf, Duration::from_secs(5)).await?;
        if name == "presence.updated" && predicate(&data) {
            return Some(data);
        }
    }
}

/// Fails if any `presence.updated` frame arrives within `window`. Non-presence
/// frames (such as a `task.*` from the mutation) are allowed and skipped.
async fn assert_no_presence_within(
    resp: &mut reqwest::Response,
    buf: &mut String,
    window: Duration,
) {
    let deadline = Instant::now() + window;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match next_sse_event(resp, buf, remaining).await {
            Some((name, data)) => assert_ne!(
                name, "presence.updated",
                "an already-present agent must not re-broadcast presence; got: {data}"
            ),
            None => break,
        }
    }
}

async fn open_stream(base: &str, ws_slug: &str, token: &str) -> reqwest::Response {
    let resp = reqwest::Client::new()
        .get(events_url(base, ws_slug))
        .bearer_auth(token)
        .send()
        .await
        .expect("open SSE stream");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "authenticated SSE connect must be 200"
    );
    resp
}

async fn seed_project_and_board(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    slug: &str,
    prefix: &str,
    visibility: Visibility,
) -> (ProjectId, BoardId) {
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        ctx,
        NewProject {
            name: format!("Project {slug}"),
            slug: slug.to_string(),
            task_prefix: prefix.to_string(),
            visibility,
        },
    )
    .await
    .expect("seed project");

    let board = PgBoardRepo::new(db.conn().clone())
        .create_board(
            ctx,
            NewBoard {
                project_id: project.id,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("seed board");

    (project.id, board.id)
}

/// Spawns the full live pipeline (Postgres `LISTEN` consumer + agent consumer)
/// against `state`'s hub, mirroring `main`'s wiring, and returns the shutdown
/// sender plus the running task handles.
async fn spawn_live_pipeline(
    db: &support::TestDb,
    state: &AppState,
) -> (watch::Sender<bool>, Vec<tokio::task::JoinHandle<()>>) {
    let pool = db.conn().get_postgres_connection_pool().clone();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let listener = tokio::spawn(atlas_server::live::run_listener(
        pool,
        state.live.clone(),
        shutdown_rx.clone(),
    ));
    let agent = tokio::spawn(atlas_server::presence::run_presence_agent_consumer(
        state.clone(),
        shutdown_rx,
    ));

    (shutdown_tx, vec![listener, agent])
}

async fn stop_pipeline(
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<tokio::task::JoinHandle<()>>,
) {
    let _ = shutdown_tx.send(true);
    for handle in handles {
        handle.abort();
    }
}

/// Creates an agent (api-key) principal in `ws` owned by `creator`, granted
/// workspace-level editor access, and returns its bearer token plus id.
async fn create_granted_agent(
    db: &support::TestDb,
    owner_client: &atlas_client::AtlasClient,
    ws_slug: &str,
    ws_id: atlas_domain::ids::WorkspaceId,
    creator: atlas_domain::ids::UserId,
    name: &str,
) -> (uuid::Uuid, String) {
    let plain = format!("atlas_{name}_secret");
    let hash = atlas_server::auth::tokens::hash_token(&plain);

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(creator));
    let key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: name.to_string(),
                token_hash: hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key");

    owner_client
        .create_workspace_grant(
            ws_slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key.id.0,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace editor to agent");

    (key.id.0, plain)
}

/// Seeds a project, board, and one column via the owner client, returning the
/// board id and the column id a task can be created in.
async fn seed_board_via_client(
    owner_client: &atlas_client::AtlasClient,
    ws_slug: &str,
    project_slug: &str,
    task_prefix: &str,
) -> (uuid::Uuid, uuid::Uuid) {
    let project = owner_client
        .create_project(
            ws_slug,
            CreateProjectRequest {
                name: format!("Project {project_slug}"),
                slug: project_slug.to_string(),
                task_prefix: task_prefix.to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = owner_client
        .create_board(
            ws_slug,
            &project.slug,
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let column = owner_client
        .create_column(
            ws_slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create column");

    (board.id, column.id)
}

async fn agent_create_task(
    base: &str,
    ws_slug: &str,
    board_id: uuid::Uuid,
    column_id: uuid::Uuid,
    token: &str,
    title: &str,
) {
    atlas_client::AtlasClient::new(base)
        .with_token(token.to_string())
        .create_task(
            ws_slug,
            board_id,
            CreateTaskRequest {
                column_id,
                title: title.to_string(),
                description: None,
                before: None,
                after: None,
                properties: None,
            },
        )
        .await
        .expect("agent creates task");
}

// ---------------------------------------------------------------------------
// 1. Agent consumer end to end: an api-key mutation marks the agent present.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn agent_mutation_broadcasts_agent_presence() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");

    let (shutdown_tx, handles) = spawn_live_pipeline(&db, &state).await;
    let server = support::TestServer::spawn_with_state(state).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "p2-agent-e2e").await;
    let owner_token = owner_client.token().expect("owner token").to_string();

    let (board_id, column_id) =
        seed_board_via_client(&owner_client, &ws.slug, "p2-agent-e2e-proj", "P2A").await;

    let (agent_id, agent_token) = create_granted_agent(
        &db,
        &owner_client,
        &ws.slug,
        ws.id,
        owner.id,
        "p2-agent-e2e-key",
    )
    .await;

    // Let the LISTEN consumer subscribe (NOTIFY only reaches an already-listening
    // session) and the agent consumer register on the hub before mutating.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut buf = String::new();

    agent_create_task(
        server.base_url(),
        &ws.slug,
        board_id,
        column_id,
        &agent_token,
        "Agent task",
    )
    .await;

    let agent_id_str = agent_id.to_string();
    let data = wait_for_presence(&mut resp, &mut buf, |d| d.contains(&agent_id_str))
        .await
        .expect("agent activity must broadcast a presence.updated carrying the agent");

    assert!(
        data.contains("api_key"),
        "the present principal is the api-key agent: {data}"
    );
    assert!(
        data.contains("p2-agent-e2e-key"),
        "the presence frame carries the agent's resolved name: {data}"
    );

    drop(resp);
    stop_pipeline(shutdown_tx, handles).await;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 2. No self-loop / no duplicate storm: a second mutation within the TTL does
//    not re-broadcast presence.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn second_agent_mutation_does_not_rebroadcast() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");

    let (shutdown_tx, handles) = spawn_live_pipeline(&db, &state).await;
    let server = support::TestServer::spawn_with_state(state).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "p2-agent-refresh").await;
    let owner_token = owner_client.token().expect("owner token").to_string();

    let (board_id, column_id) =
        seed_board_via_client(&owner_client, &ws.slug, "p2-agent-refresh-proj", "P2R").await;

    let (agent_id, agent_token) = create_granted_agent(
        &db,
        &owner_client,
        &ws.slug,
        ws.id,
        owner.id,
        "p2-agent-refresh-key",
    )
    .await;

    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut buf = String::new();

    // First mutation establishes the agent's presence (a change → one broadcast).
    agent_create_task(
        server.base_url(),
        &ws.slug,
        board_id,
        column_id,
        &agent_token,
        "First agent task",
    )
    .await;

    let agent_id_str = agent_id.to_string();
    wait_for_presence(&mut resp, &mut buf, |d| d.contains(&agent_id_str))
        .await
        .expect("first agent activity must broadcast presence");

    // Second mutation within the TTL is a refresh, not a change: no new presence
    // frame (the task.created frame is fine).
    agent_create_task(
        server.base_url(),
        &ws.slug,
        board_id,
        column_id,
        &agent_token,
        "Second agent task",
    )
    .await;

    assert_no_presence_within(&mut resp, &mut buf, Duration::from_secs(2)).await;

    drop(resp);
    stop_pipeline(shutdown_tx, handles).await;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 3. Sweeper wiring: an expired entry produces an empty-actors presence.updated.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sweep_then_broadcast_delivers_empty_actors() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");

    // The server shares the same hub and registry via the cloned state.
    let server = support::TestServer::spawn_with_state(state.clone()).await;

    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "p2-sweeper").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let token = client.token().expect("token").to_string();

    let (project_id, board_id) = seed_project_and_board(
        &db,
        &ctx,
        "p2-sweeper-proj",
        "P2S",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    // Seed a presence entry, then open the stream so the seeding broadcast (which
    // this helper does not perform) is never in play — the stream only observes the
    // sweep's empty-actors frame.
    let agent = ActorDto {
        r#type: "api_key".into(),
        id: uuid::Uuid::now_v7(),
        display_name: Some("Agent".into()),
        key_type: Some("agent".into()),
        account_status: None,
    };
    let resource = PresenceResource::Board(board_id.0);
    assert!(
        state.presence.heartbeat(ws.id.0, resource, agent),
        "seeding the entry is a change"
    );

    let mut resp = open_stream(server.base_url(), &ws.slug, &token).await;
    let mut buf = String::new();

    // Exactly what one sweeper tick does: sweep expired entries, then broadcast the
    // affected boards. A zero TTL treats every entry as expired without waiting the
    // real 45s window; `project = None` matches the sweeper, exercising the SSE
    // filter's board→project resolution.
    let changed = state.presence.sweep(Duration::from_secs(0));
    assert_eq!(
        changed,
        vec![(ws.id.0, resource)],
        "the swept board is reported changed"
    );
    for (workspace, resource) in changed {
        broadcast_presence(&state, workspace, resource, None);
    }

    let data = wait_for_presence(&mut resp, &mut buf, |_| true)
        .await
        .expect("the sweep must broadcast a presence.updated for the emptied board");

    assert!(
        data.contains("\"actors\":[]"),
        "the swept board's presence set is empty: {data}"
    );

    let _ = project_id;
    drop(resp);
    db.teardown().await;
}
