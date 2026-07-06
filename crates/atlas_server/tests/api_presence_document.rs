//! Integration tests for the document presence endpoints:
//! `POST/DELETE /api/workspaces/{ws}/documents/{slug}/presence`.
//!
//! Mirrors `api_presence` (board presence) for the document-scoped case, covering
//! the heartbeat/leave HTTP contract plus the `presence.updated` live-event fan-out
//! delivered over the SSE stream:
//! - a heartbeat lists the caller as present and returns the document's canonical id;
//! - a second principal joining broadcasts a presence.updated carrying both;
//! - a heartbeat refresh by an already-present principal broadcasts nothing;
//! - leaving broadcasts a presence.updated with the actor removed;
//! - a principal who cannot view the document is rejected;
//! - document presence is NOT delivered over SSE to a principal who cannot view the
//!   document (the per-document authorization gate must not leak awareness).
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

use std::time::{Duration, Instant};

use atlas_api::dtos::{CreateProjectRequest, documents::CreateDocumentRequest};
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::MembershipRepo;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn presence_url(base: &str, ws_slug: &str, slug: &str) -> String {
    format!("{base}/api/workspaces/{ws_slug}/documents/{slug}/presence")
}

fn events_url(base: &str, ws_slug: &str) -> String {
    format!("{base}/api/workspaces/{ws_slug}/events")
}

fn project_req(slug: &str, prefix: &str, visibility: Option<&str>) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: visibility.map(str::to_string),
        visibility_role: visibility.map(|_| "editor".to_string()),
    }
}

/// Creates a project with the given visibility and a document inside it, returning
/// the document's slug and canonical UUID.
async fn seed_project_and_document(
    client: &atlas_client::AtlasClient,
    ws_slug: &str,
    proj_slug: &str,
    prefix: &str,
    visibility: Option<&str>,
) -> (String, uuid::Uuid) {
    client
        .create_project(ws_slug, project_req(proj_slug, prefix, visibility))
        .await
        .expect("create project");

    let doc = client
        .create_document(
            ws_slug,
            proj_slug,
            CreateDocumentRequest {
                title: "Presence doc".to_string(),
                folder_id: None,
                content: Some("# Doc\n\nBody".to_string()),
            },
        )
        .await
        .expect("create document");

    (doc.slug.expect("document must have a slug"), doc.id)
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
/// elapses. Intermediate frames are skipped.
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
/// frames are allowed and skipped.
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
                "an unexpected presence.updated frame was delivered: {data}"
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

async fn heartbeat(base: &str, ws_slug: &str, slug: &str, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(presence_url(base, ws_slug, slug))
        .bearer_auth(token)
        .send()
        .await
        .expect("heartbeat request")
}

async fn leave(base: &str, ws_slug: &str, slug: &str, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .delete(presence_url(base, ws_slug, slug))
        .bearer_auth(token)
        .send()
        .await
        .expect("leave request")
}

// ---------------------------------------------------------------------------
// 1. Heartbeat lists the caller as present and returns the document id.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_heartbeat_lists_caller_as_present() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "doc-presence-solo").await;
    let token = client.token().expect("token").to_string();

    let (slug, doc_id) = seed_project_and_document(
        &client,
        &ws.slug,
        "doc-presence-solo-proj",
        "DPS",
        Some("workspace"),
    )
    .await;

    let resp = heartbeat(server.base_url(), &ws.slug, &slug, &token).await;
    assert_eq!(resp.status().as_u16(), 200, "heartbeat must be 200");

    let body: serde_json::Value = resp.json().await.expect("json body");

    assert_eq!(
        body["document_id"].as_str().unwrap(),
        doc_id.to_string(),
        "the response carries the document's canonical id"
    );

    let actors = body["actors"].as_array().expect("actors array");
    assert_eq!(actors.len(), 1, "exactly the caller is present");
    assert_eq!(actors[0]["id"].as_str().unwrap(), user.id.0.to_string());
    assert_eq!(actors[0]["type"].as_str().unwrap(), "user");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 2. Heartbeating by UUID resolves the same presence set as by slug.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_heartbeat_accepts_uuid_and_slug() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "doc-presence-uuid").await;
    let token = client.token().expect("token").to_string();

    let (_slug, doc_id) = seed_project_and_document(
        &client,
        &ws.slug,
        "doc-presence-uuid-proj",
        "DPU",
        Some("workspace"),
    )
    .await;

    // Addressing the document by its UUID resolves the same document, so the caller
    // is the single present principal.
    let resp = heartbeat(server.base_url(), &ws.slug, &doc_id.to_string(), &token).await;
    assert_eq!(resp.status().as_u16(), 200);

    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["document_id"].as_str().unwrap(), doc_id.to_string());

    let actors = body["actors"].as_array().expect("actors array");
    assert_eq!(actors.len(), 1);
    assert_eq!(actors[0]["id"].as_str().unwrap(), user.id.0.to_string());

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 3. A second principal joining broadcasts presence.updated carrying both.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_second_principal_join_broadcasts_both() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "doc-presence-join-owner").await;
    let owner_token = owner_client.token().expect("owner token").to_string();

    // A plain member; a workspace-visibility document is viewable by any member.
    let (member_client, member) =
        support::login_user(&server, &db, "doc-presence-join-member").await;
    let member_ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    db.membership_repo()
        .add(&member_ctx, member.id, MemberRole::Member)
        .await
        .expect("add member");
    let member_token = member_client.token().expect("member token").to_string();

    let (slug, _doc_id) = seed_project_and_document(
        &owner_client,
        &ws.slug,
        "doc-presence-join-proj",
        "DPJ",
        Some("workspace"),
    )
    .await;

    // Owner establishes presence before opening the stream so the first broadcast
    // ([owner]) is not observed; the stream then only sees the join of the member.
    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &slug, &owner_token)
            .await
            .status()
            .as_u16(),
        200
    );

    let mut owner_resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut owner_buf = String::new();

    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &slug, &member_token)
            .await
            .status()
            .as_u16(),
        200
    );

    let owner_id = owner.id.0.to_string();
    let member_id = member.id.0.to_string();

    let data = wait_for_presence(&mut owner_resp, &mut owner_buf, |d| {
        d.contains(&owner_id) && d.contains(&member_id)
    })
    .await
    .expect("stream must deliver a presence.updated carrying both principals");

    assert!(
        data.contains("document_id"),
        "the frame is document-scoped: {data}"
    );

    drop(owner_resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 4. A heartbeat refresh by an already-present principal broadcasts nothing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_refresh_does_not_rebroadcast() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "doc-presence-refresh").await;
    let token = client.token().expect("token").to_string();

    let (slug, _doc_id) = seed_project_and_document(
        &client,
        &ws.slug,
        "doc-presence-refresh-proj",
        "DPR",
        Some("workspace"),
    )
    .await;

    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &slug, &token)
            .await
            .status()
            .as_u16(),
        200
    );

    let mut resp = open_stream(server.base_url(), &ws.slug, &token).await;
    let mut buf = String::new();

    // A refresh of an already-present principal is not a change: no broadcast.
    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &slug, &token)
            .await
            .status()
            .as_u16(),
        200
    );

    assert_no_presence_within(&mut resp, &mut buf, Duration::from_secs(2)).await;

    drop(resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 5. Leaving broadcasts a presence.updated with the actor removed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_leave_broadcasts_removal() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "doc-presence-leave-owner").await;
    let owner_token = owner_client.token().expect("owner token").to_string();

    let (member_client, member) =
        support::login_user(&server, &db, "doc-presence-leave-member").await;
    let member_ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    db.membership_repo()
        .add(&member_ctx, member.id, MemberRole::Member)
        .await
        .expect("add member");
    let member_token = member_client.token().expect("member token").to_string();

    let (slug, _doc_id) = seed_project_and_document(
        &owner_client,
        &ws.slug,
        "doc-presence-leave-proj",
        "DPL",
        Some("workspace"),
    )
    .await;

    // Both present before streaming, so join broadcasts are not observed.
    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &slug, &owner_token)
            .await
            .status()
            .as_u16(),
        200
    );
    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &slug, &member_token)
            .await
            .status()
            .as_u16(),
        200
    );

    let mut owner_resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut owner_buf = String::new();

    let r = leave(server.base_url(), &ws.slug, &slug, &member_token).await;
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
// 6. A principal who cannot view the document is rejected.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_heartbeat_rejected_for_non_viewer() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "doc-presence-authz-owner").await;

    // A plain member with no grant cannot view a Private project's document.
    let (member_client, member) =
        support::login_user(&server, &db, "doc-presence-authz-member").await;
    let member_ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    db.membership_repo()
        .add(&member_ctx, member.id, MemberRole::Member)
        .await
        .expect("add member");
    let member_token = member_client.token().expect("member token").to_string();

    let (slug, _doc_id) = seed_project_and_document(
        &owner_client,
        &ws.slug,
        "doc-presence-authz-proj",
        "DPA",
        Some("private"),
    )
    .await;

    let resp = heartbeat(server.base_url(), &ws.slug, &slug, &member_token).await;
    let status = resp.status().as_u16();

    assert!(
        status == 403 || status == 404,
        "a non-viewer must be denied (403/404); got {status}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 7. Document presence must NOT be delivered over SSE to a non-viewer.
//
// This is the leak guard: a `presence.updated` document event is authorized
// against the per-document permission chain, so a workspace member who cannot view
// the private document never learns who is editing it — even though they may open
// the workspace stream.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn document_presence_not_leaked_to_non_viewer_over_sse() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "doc-presence-leak-owner").await;
    let owner_token = owner_client.token().expect("owner token").to_string();

    // Outsider is a workspace member (so they may open the stream) but has no grant
    // on the private project, so they cannot view its document.
    let (outsider_client, outsider) =
        support::login_user(&server, &db, "doc-presence-leak-outsider").await;
    let outsider_ctx = WorkspaceCtx::new(ws.id, Actor::User(outsider.id));
    db.membership_repo()
        .add(&outsider_ctx, outsider.id, MemberRole::Member)
        .await
        .expect("add member");
    let outsider_token = outsider_client.token().expect("outsider token").to_string();

    let (slug, _doc_id) = seed_project_and_document(
        &owner_client,
        &ws.slug,
        "doc-presence-leak-proj",
        "DPK",
        Some("private"),
    )
    .await;

    // Owner joins the document (a change → broadcast) before the outsider streams,
    // so only the subsequent leave broadcast is under test.
    assert_eq!(
        heartbeat(server.base_url(), &ws.slug, &slug, &owner_token)
            .await
            .status()
            .as_u16(),
        200
    );

    let mut outsider_resp = open_stream(server.base_url(), &ws.slug, &outsider_token).await;
    let mut outsider_buf = String::new();

    // Owner leaves — a change that broadcasts a document-scoped presence.updated. The
    // outsider, who cannot view the document, must not receive it.
    assert_eq!(
        leave(server.base_url(), &ws.slug, &slug, &owner_token)
            .await
            .status()
            .as_u16(),
        204
    );

    assert_no_presence_within(
        &mut outsider_resp,
        &mut outsider_buf,
        Duration::from_secs(2),
    )
    .await;

    drop(outsider_resp);
    db.teardown().await;
}
