//! Integration tests for `GET /v1/workspaces/{ws}/events` — the live-updates
//! Server-Sent Events endpoint (work unit 2).
//!
//! Covers the SSE transport plus the per-resource authorization filter:
//! - a connected principal receives forwarded events;
//! - unauthenticated requests are rejected before any stream is opened;
//! - events from another workspace never reach the stream (cross-tenant isolation);
//! - an event on a board the principal cannot view is filtered out per-resource.
//!
//! All stream reads use `tokio::time::timeout`; the stream is infinite, so tests
//! read only the frames they need and then drop the response.

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
    entities::{boards_tasks::NewBoard, identity::MemberRole, workspace_core::NewProject},
    ids::{BoardId, ProjectId},
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    live::LiveEvent,
    persistence::repos::{BoardRepo, MembershipRepo, PgBoardRepo, PgProjectRepo, ProjectRepo},
    state::AppState,
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn events_url(base: &str, ws_slug: &str) -> String {
    format!("{base}/v1/workspaces/{ws_slug}/events")
}

/// A `task.created`-shaped envelope carrying `task_id`, used verbatim as SSE data.
fn task_created_payload(task_id: Uuid) -> Arc<str> {
    Arc::from(
        format!(r#"{{"event_type":"task.created","data":{{"task_id":"{task_id}"}}}}"#).as_str(),
    )
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

/// Reads the next meaningful SSE frame (skipping keep-alive comments) from a
/// streaming reqwest response, returning `(event_name, data)`. Returns `None`
/// when the timeout elapses or the body ends — never drains to EOF, so it is
/// safe to call against the infinite live stream.
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
                // Keep-alive comment frame (":"): skip and look for the next one.
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

async fn open_stream(base: &str, ws_slug: &str, token: &str) -> reqwest::Response {
    let http = reqwest::Client::new();
    let resp = http
        .get(events_url(base, ws_slug))
        .bearer_auth(token)
        .send()
        .await
        .expect("open SSE stream");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "authenticated SSE connect must be 200; got {:?}",
        resp.status()
    );

    resp
}

// ---------------------------------------------------------------------------
// 1. Happy path: a forwarded event reaches the connected principal.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_delivers_forwarded_event_to_connected_principal() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let hub = state.live.clone();
    let server = support::TestServer::spawn_with_state(state).await;

    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "sse-happy").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let token = client.token().expect("token").to_string();

    let (project_id, board_id) = seed_project_and_board(
        &db,
        &ctx,
        "sse-happy-proj",
        "SSH",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    let mut resp = open_stream(server.base_url(), &ws.slug, &token).await;
    let mut buf = String::new();

    let task_id = Uuid::now_v7();
    hub.publish(LiveEvent {
        workspace_id: ws.id.0,
        project_id: Some(project_id.0),
        board_id: Some(board_id.0),
        document_id: None,
        event_type: "task.created".to_string(),
        payload: task_created_payload(task_id),
    });

    let (event_name, data) = next_sse_event(&mut resp, &mut buf, Duration::from_secs(5))
        .await
        .expect("stream must yield the forwarded event within the timeout");

    assert_eq!(
        event_name, "task.created",
        "SSE event name is the event_type"
    );
    assert!(
        data.contains(&task_id.to_string()),
        "SSE data must be the raw envelope carrying the task id; got: {data}"
    );

    drop(resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 2. Unauthenticated connect is rejected with 401 before any stream is opened.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_requires_authentication() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let http = reqwest::Client::new();
    let resp = http
        .get(events_url(server.base_url(), "any-workspace"))
        .send()
        .await
        .expect("request");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "unauthenticated SSE connect must be 401"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 3. Cross-workspace isolation: an event in workspace B never reaches a stream
//    opened on workspace A.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_does_not_deliver_cross_workspace_events() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let hub = state.live.clone();
    let server = support::TestServer::spawn_with_state(state).await;

    let (client, ws_a, _user_a) =
        support::login_user_with_workspace(&server, &db, "sse-xws-a").await;
    let token = client.token().expect("token").to_string();

    let foreign_workspace_id = Uuid::now_v7();

    let mut resp = open_stream(server.base_url(), &ws_a.slug, &token).await;
    let mut buf = String::new();

    // Emitted in a different workspace: must be filtered out (never forwarded).
    hub.publish(LiveEvent {
        workspace_id: foreign_workspace_id,
        project_id: None,
        board_id: None,
        document_id: None,
        event_type: "task.created".to_string(),
        payload: task_created_payload(Uuid::now_v7()),
    });

    // Emitted in workspace A: must be forwarded. Publishing it after the foreign
    // event proves the foreign event was skipped rather than merely delayed —
    // the first frame the stream yields is this one, not the cross-tenant event.
    let local_task_id = Uuid::now_v7();
    hub.publish(LiveEvent {
        workspace_id: ws_a.id.0,
        project_id: None,
        board_id: None,
        document_id: None,
        event_type: "task.created".to_string(),
        payload: task_created_payload(local_task_id),
    });

    let (_event_name, data) = next_sse_event(&mut resp, &mut buf, Duration::from_secs(5))
        .await
        .expect("workspace A event must be delivered");

    assert!(
        data.contains(&local_task_id.to_string()),
        "the first delivered frame must be the workspace A event, not the cross-tenant one; got: {data}"
    );

    drop(resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 4. Per-resource filtering: an event on a board a principal cannot view is not
//    delivered to that principal, but is delivered to one who can.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_filters_events_by_per_resource_access() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let hub = state.live.clone();
    let server = support::TestServer::spawn_with_state(state).await;

    // P1 is the workspace owner (sees everything).
    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "sse-filter-owner").await;
    let owner_ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    let owner_token = owner_client.token().expect("owner token").to_string();

    // P2 is a plain member: has workspace access (passes the connect gate) but no
    // view access to a Private project's board.
    let (member_client, member) = support::login_user(&server, &db, "sse-filter-member").await;
    let member_ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    db.membership_repo()
        .add(&member_ctx, member.id, MemberRole::Member)
        .await
        .expect("add plain member");
    let member_token = member_client.token().expect("member token").to_string();

    let (project_id, board_id) = seed_project_and_board(
        &db,
        &owner_ctx,
        "sse-filter-proj",
        "SFP",
        Visibility::Private,
    )
    .await;

    let mut owner_resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut owner_buf = String::new();
    let mut member_resp = open_stream(server.base_url(), &ws.slug, &member_token).await;
    let mut member_buf = String::new();

    // An event on the Private board: the owner may see it, the plain member may not.
    let board_task_id = Uuid::now_v7();
    hub.publish(LiveEvent {
        workspace_id: ws.id.0,
        project_id: Some(project_id.0),
        board_id: Some(board_id.0),
        document_id: None,
        event_type: "task.created".to_string(),
        payload: task_created_payload(board_task_id),
    });

    let (_name, owner_data) =
        next_sse_event(&mut owner_resp, &mut owner_buf, Duration::from_secs(5))
            .await
            .expect("owner must receive the board event");
    assert!(
        owner_data.contains(&board_task_id.to_string()),
        "owner must receive the private board event; got: {owner_data}"
    );

    // The member must NOT receive the private board event (short timeout: absence).
    let filtered = next_sse_event(&mut member_resp, &mut member_buf, Duration::from_secs(2)).await;
    assert!(
        filtered.is_none(),
        "plain member must not receive an event on a board they cannot view; got: {filtered:?}"
    );

    // Prove the member's stream is alive and correctly scoped: a workspace-level
    // event they DO have access to is delivered.
    let ws_task_id = Uuid::now_v7();
    hub.publish(LiveEvent {
        workspace_id: ws.id.0,
        project_id: None,
        board_id: None,
        document_id: None,
        event_type: "task.created".to_string(),
        payload: task_created_payload(ws_task_id),
    });

    let (_name, member_data) =
        next_sse_event(&mut member_resp, &mut member_buf, Duration::from_secs(5))
            .await
            .expect("member must receive the workspace-level event");
    assert!(
        member_data.contains(&ws_task_id.to_string()),
        "member must receive a workspace-level event they can access; got: {member_data}"
    );

    drop(owner_resp);
    drop(member_resp);
    db.teardown().await;
}
