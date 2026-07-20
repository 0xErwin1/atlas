//! Integration tests for the board presence endpoints (work unit P1):
//! `POST/DELETE /api/workspaces/{ws}/boards/{board_id}/presence`.
//!
//! Covers the heartbeat/leave HTTP contract plus the `presence.updated` live-event
//! fan-out delivered over the SSE stream:
//! - a heartbeat lists the caller as present;
//! - a second principal joining broadcasts a presence.updated carrying both;
//! - a heartbeat refresh by an already-present principal broadcasts nothing;
//! - leaving broadcasts a presence.updated with the actor removed;
//! - a principal who cannot view the board is rejected.
//!
//! SSE reads use `tokio::time::timeout`; the stream is infinite, so tests read only
//! the frames they need and then drop the response.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::time::Duration;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{boards_tasks::NewBoard, identity::MemberRole, workspace_core::NewProject},
    ids::{BoardId, ProjectId},
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    BoardRepo, MembershipRepo, PgBoardRepo, PgProjectRepo, ProjectRepo,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn presence_url(base: &str, ws_slug: &str, board_id: &uuid::Uuid) -> String {
    format!("{base}/api/workspaces/{ws_slug}/boards/{board_id}/presence")
}

fn events_url(base: &str, ws_slug: &str) -> String {
    format!("{base}/api/workspaces/{ws_slug}/events")
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
                folder_id: None,
                project_id: project.id,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("seed board");

    (project.id, board.id)
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

/// Reads presence.updated frames until one satisfies `predicate`, or the timeout
/// elapses. Intermediate frames (e.g. an earlier presence snapshot) are skipped.
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
        "authenticated SSE connect must be 200"
    );
    resp
}

async fn heartbeat(
    base: &str,
    ws_slug: &str,
    board_id: &uuid::Uuid,
    token: &str,
) -> reqwest::Response {
    reqwest::Client::new()
        .post(presence_url(base, ws_slug, board_id))
        .bearer_auth(token)
        .send()
        .await
        .expect("heartbeat request")
}

async fn leave(base: &str, ws_slug: &str, board_id: &uuid::Uuid, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .delete(presence_url(base, ws_slug, board_id))
        .bearer_auth(token)
        .send()
        .await
        .expect("leave request")
}

// ---------------------------------------------------------------------------
// 1. Heartbeat lists the caller as present.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn heartbeat_lists_caller_as_present() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "presence-solo").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let token = client.token().expect("token").to_string();

    let (_project_id, board_id) = seed_project_and_board(
        &db,
        &ctx,
        "presence-solo-proj",
        "PSO",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    let resp = heartbeat(server.base_url(), &ws.slug, &board_id.0, &token).await;
    assert_eq!(resp.status().as_u16(), 200, "heartbeat must be 200");

    let body: serde_json::Value = resp.json().await.expect("json body");
    let actors = body["actors"].as_array().expect("actors array");

    assert_eq!(actors.len(), 1, "exactly the caller is present");
    assert_eq!(actors[0]["id"].as_str().unwrap(), user.id.0.to_string());
    assert_eq!(actors[0]["type"].as_str().unwrap(), "user");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 2. A second principal joining broadcasts presence.updated carrying both.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn second_principal_join_broadcasts_presence_with_both() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "presence-join-owner").await;
    let owner_ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    let owner_token = owner_client.token().expect("owner token").to_string();

    // P2 is a plain member; a workspace-visibility board is viewable by any member.
    let (member_client, member) = support::login_user(&server, &db, "presence-join-member").await;
    let member_ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    db.membership_repo()
        .add(&member_ctx, member.id, MemberRole::Member)
        .await
        .expect("add member");
    let member_token = member_client.token().expect("member token").to_string();

    let (_project_id, board_id) = seed_project_and_board(
        &db,
        &owner_ctx,
        "presence-join-proj",
        "PJN",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    // Owner establishes presence before opening the stream so the first broadcast
    // ([owner]) is not observed; the stream then only sees the join of P2.
    let r = heartbeat(server.base_url(), &ws.slug, &board_id.0, &owner_token).await;
    assert_eq!(r.status().as_u16(), 200);

    let mut owner_resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut owner_buf = String::new();

    let r = heartbeat(server.base_url(), &ws.slug, &board_id.0, &member_token).await;
    assert_eq!(r.status().as_u16(), 200);

    let owner_id = owner.id.0.to_string();
    let member_id = member.id.0.to_string();

    let data = wait_for_presence(&mut owner_resp, &mut owner_buf, |d| {
        d.contains(&owner_id) && d.contains(&member_id)
    })
    .await
    .expect("stream must deliver a presence.updated carrying both principals");

    assert!(
        data.contains("presence.updated"),
        "data mirrors the envelope shape: {data}"
    );

    drop(owner_resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 3. A heartbeat refresh by an already-present principal broadcasts nothing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn refresh_does_not_rebroadcast() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "presence-refresh").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let token = client.token().expect("token").to_string();

    let (_project_id, board_id) = seed_project_and_board(
        &db,
        &ctx,
        "presence-refresh-proj",
        "PRF",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    // Establish presence before streaming, so the join broadcast is not observed.
    let r = heartbeat(server.base_url(), &ws.slug, &board_id.0, &token).await;
    assert_eq!(r.status().as_u16(), 200);

    let mut resp = open_stream(server.base_url(), &ws.slug, &token).await;
    let mut buf = String::new();

    // A refresh of an already-present principal is not a change: no broadcast.
    let r = heartbeat(server.base_url(), &ws.slug, &board_id.0, &token).await;
    assert_eq!(r.status().as_u16(), 200);

    let frame = next_sse_event(&mut resp, &mut buf, Duration::from_secs(2)).await;
    assert!(
        frame.is_none(),
        "a heartbeat refresh must not produce a presence.updated frame; got: {frame:?}"
    );

    drop(resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 4. Leaving broadcasts a presence.updated with the actor removed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn leave_broadcasts_removal() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "presence-leave-owner").await;
    let owner_ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    let owner_token = owner_client.token().expect("owner token").to_string();

    let (member_client, member) = support::login_user(&server, &db, "presence-leave-member").await;
    let member_ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    db.membership_repo()
        .add(&member_ctx, member.id, MemberRole::Member)
        .await
        .expect("add member");
    let member_token = member_client.token().expect("member token").to_string();

    let (_project_id, board_id) = seed_project_and_board(
        &db,
        &owner_ctx,
        "presence-leave-proj",
        "PLV",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    // Both present before streaming, so join broadcasts are not observed.
    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &board_id.0, &owner_token)
            .await
            .status()
            .as_u16(),
        200
    );
    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &board_id.0, &member_token)
            .await
            .status()
            .as_u16(),
        200
    );

    let mut owner_resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut owner_buf = String::new();

    let r = leave(server.base_url(), &ws.slug, &board_id.0, &member_token).await;
    assert_eq!(r.status().as_u16(), 204, "leave must be 204");

    let owner_id = owner.id.0.to_string();
    let member_id = member.id.0.to_string();

    let data = wait_for_presence(&mut owner_resp, &mut owner_buf, |d| !d.contains(&member_id))
        .await
        .expect("stream must deliver a presence.updated after leave");

    assert!(
        data.contains(&owner_id),
        "the remaining principal is still present: {data}"
    );
    assert!(
        !data.contains(&member_id),
        "the departed principal is removed from the set: {data}"
    );

    drop(owner_resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 5. A principal who cannot view the board is rejected.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn heartbeat_rejected_for_non_viewer() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "presence-authz-owner").await;
    let owner_ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    let _ = owner_client;

    // A plain member with no grant cannot view a Private project's board.
    let (member_client, member) = support::login_user(&server, &db, "presence-authz-member").await;
    let member_ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    db.membership_repo()
        .add(&member_ctx, member.id, MemberRole::Member)
        .await
        .expect("add member");
    let member_token = member_client.token().expect("member token").to_string();

    let (_project_id, board_id) = seed_project_and_board(
        &db,
        &owner_ctx,
        "presence-authz-proj",
        "PAZ",
        Visibility::Private,
    )
    .await;

    let resp = heartbeat(server.base_url(), &ws.slug, &board_id.0, &member_token).await;
    let status = resp.status().as_u16();

    assert!(
        status == 403 || status == 404,
        "a non-viewer must be denied (403/404); got {status}"
    );

    db.teardown().await;
}
