//! Integration tests for `GET /api/workspaces/{ws}/events` — the live-updates
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

use atlas_api::dtos::{CreateProjectRequest, documents::CreateDocumentRequest};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{
        boards_tasks::NewBoard,
        identity::{ApiKeyType, MemberRole},
        permissions::NewPermissionGrant,
        workspace_core::NewProject,
    },
    ids::{BoardId, ProjectId, UserId, WorkspaceId},
    permissions::{
        Capability, CapabilityAction, CapabilityFamily, ResourceRole, Visibility, VisibilityRole,
    },
};
use atlas_server::{
    auth::tokens::{generate_api_key, hash_token},
    live::LiveEvent,
    persistence::repos::{
        ApiKeyRepo, BoardRepo, MembershipRepo, NewApiKey, PermissionGrantRepo, PgApiKeyRepo,
        PgBoardRepo, PgPermissionGrantRepo, PgProjectRepo, ProjectRepo,
    },
    state::AppState,
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn events_url(base: &str, ws_slug: &str) -> String {
    format!("{base}/api/workspaces/{ws_slug}/events")
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

// ---------------------------------------------------------------------------
// API-key read-capability scoping (ATL-39 S2, part 2)
//
// The stream applies a per-event capability pre-filter for an `ApiKey` (agent)
// principal: it receives events only for families it holds `{family}:read` on.
// Humans, root, and groups carry no scope axis and are unaffected.
//
// The family-matrix tests publish workspace-level envelopes (no routing ids) so
// the per-resource ROLE check is trivially satisfied and only the capability
// axis is exercised. The presence tests route to real resources the agent may
// view, so both the capability family and the routed-id resolution are covered.
// ---------------------------------------------------------------------------

fn read_cap(family: CapabilityFamily) -> Capability {
    Capability {
        family,
        action: CapabilityAction::Read,
    }
}

/// A minimal envelope carrying a unique `marker`, used verbatim as SSE data so a
/// test can assert which published event a delivered frame corresponds to.
fn marked_payload(event_type: &str, marker: Uuid) -> Arc<str> {
    Arc::from(format!(r#"{{"event_type":"{event_type}","data":{{"marker":"{marker}"}}}}"#).as_str())
}

/// Publishes a workspace-level event (no routing ids) so only the capability
/// pre-filter can gate it, and returns the marker id it carries.
fn publish_ws_level_event(
    hub: &atlas_server::live::LiveEventHub,
    workspace_id: WorkspaceId,
    event_type: &str,
) -> Uuid {
    let marker = Uuid::now_v7();
    hub.publish(LiveEvent {
        workspace_id: workspace_id.0,
        project_id: None,
        board_id: None,
        document_id: None,
        event_type: event_type.to_string(),
        payload: marked_payload(event_type, marker),
    });
    marker
}

/// Creates an agent (api-key) principal owned by `owner_id` with an explicit read
/// scope set, plus a workspace-scope Viewer grant so it passes the SSE connect
/// gate and resolves at least Viewer on workspace-visible resources. Returns the
/// raw bearer token.
async fn create_scoped_agent(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    owner_id: UserId,
    name: &str,
    scopes: Vec<Capability>,
) -> String {
    let raw_token = generate_api_key();
    let token_hash = hash_token(&raw_token);

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(owner_id));
    let key = PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewApiKey {
            name: name.to_string(),
            token_hash,
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes,
        },
    )
    .await
    .expect("create scoped agent key");

    PgPermissionGrantRepo {
        conn: db.conn().clone(),
    }
    .upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: None,
        api_key_id: Some(key.id),
        group_id: None,
        project_id: None,
        folder_id: None,
        document_id: None,
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: Some(owner_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant workspace viewer to agent");

    raw_token
}

/// Seeds a workspace-visible project and a document inside it via the owner
/// client, returning the document's canonical UUID.
async fn seed_workspace_visible_document(
    client: &atlas_client::AtlasClient,
    ws_slug: &str,
    proj_slug: &str,
    prefix: &str,
) -> Uuid {
    client
        .create_project(
            ws_slug,
            CreateProjectRequest {
                name: format!("Project {proj_slug}"),
                slug: proj_slug.to_string(),
                task_prefix: prefix.to_string(),
                visibility: Some("workspace".to_string()),
                visibility_role: Some("editor".to_string()),
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(
            ws_slug,
            proj_slug,
            CreateDocumentRequest {
                title: "Scoped doc".to_string(),
                folder_id: None,
                content: Some("# Doc".to_string()),
            },
        )
        .await
        .expect("create document");

    doc.id
}

// ---------------------------------------------------------------------------
// 5. A scoped agent receives ONLY the families it holds `{family}:read` on.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scoped_agent_receives_only_held_families() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let hub = state.live.clone();
    let server = support::TestServer::spawn_with_state(state).await;

    let (_owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "sse-scope-families").await;

    let agent_token = create_scoped_agent(
        &db,
        ws.id,
        owner.id,
        "sse-scope-tasks-docs",
        vec![
            read_cap(CapabilityFamily::Tasks),
            read_cap(CapabilityFamily::Docs),
        ],
    )
    .await;

    let mut resp = open_stream(server.base_url(), &ws.slug, &agent_token).await;
    let mut buf = String::new();

    // Disallowed families published FIRST: none may be delivered.
    publish_ws_level_event(&hub, ws.id, "board.created");
    publish_ws_level_event(&hub, ws.id, "folder.created");
    publish_ws_level_event(&hub, ws.id, "project.created");

    // Allowed families published AFTER: they must be the frames the agent sees,
    // proving the three disallowed events were dropped rather than merely delayed.
    let task_marker = publish_ws_level_event(&hub, ws.id, "task.created");
    let doc_marker = publish_ws_level_event(&hub, ws.id, "document.updated");

    let (first_name, first_data) = next_sse_event(&mut resp, &mut buf, Duration::from_secs(5))
        .await
        .expect("scoped agent must receive the task event");
    assert_eq!(
        first_name, "task.created",
        "first delivered frame is the task"
    );
    assert!(
        first_data.contains(&task_marker.to_string()),
        "the first frame is the published task event; got: {first_data}"
    );

    let (second_name, second_data) = next_sse_event(&mut resp, &mut buf, Duration::from_secs(5))
        .await
        .expect("scoped agent must receive the document event");
    assert_eq!(second_name, "document.updated");
    assert!(
        second_data.contains(&doc_marker.to_string()),
        "the second frame is the published document event; got: {second_data}"
    );

    drop(resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 6. Presence is gated by the family its routing id resolves to: a document
//    presence event needs docs:read; a board presence event needs boards:read.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn presence_is_gated_by_routed_family() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let hub = state.live.clone();
    let server = support::TestServer::spawn_with_state(state).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "sse-scope-presence").await;
    let owner_ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));

    let (project_id, board_id) = seed_project_and_board(
        &db,
        &owner_ctx,
        "sse-scope-presence-board-proj",
        "SPB",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    let document_id = seed_workspace_visible_document(
        &owner_client,
        &ws.slug,
        "sse-scope-presence-doc-proj",
        "SPD",
    )
    .await;

    // One agent holds docs:read only, the other boards:read only. Both hold a
    // workspace Viewer grant, so per-resource ROLE would admit both events; only
    // the capability family differs.
    let docs_agent = create_scoped_agent(
        &db,
        ws.id,
        owner.id,
        "sse-scope-docs-agent",
        vec![read_cap(CapabilityFamily::Docs)],
    )
    .await;
    let boards_agent = create_scoped_agent(
        &db,
        ws.id,
        owner.id,
        "sse-scope-boards-agent",
        vec![read_cap(CapabilityFamily::Boards)],
    )
    .await;

    let mut docs_resp = open_stream(server.base_url(), &ws.slug, &docs_agent).await;
    let mut docs_buf = String::new();
    let mut boards_resp = open_stream(server.base_url(), &ws.slug, &boards_agent).await;
    let mut boards_buf = String::new();

    // Board presence: gated by Boards. The boards agent receives it; the docs
    // agent must not (a short-timeout absence check).
    let board_marker = Uuid::now_v7();
    hub.publish(LiveEvent {
        workspace_id: ws.id.0,
        project_id: Some(project_id.0),
        board_id: Some(board_id.0),
        document_id: None,
        event_type: "presence.updated".to_string(),
        payload: marked_payload("presence.updated", board_marker),
    });

    let (name, data) = next_sse_event(&mut boards_resp, &mut boards_buf, Duration::from_secs(5))
        .await
        .expect("boards agent must receive board presence");
    assert_eq!(name, "presence.updated");
    assert!(
        data.contains(&board_marker.to_string()),
        "boards agent receives the board presence event; got: {data}"
    );

    let leaked = next_sse_event(&mut docs_resp, &mut docs_buf, Duration::from_secs(2)).await;
    assert!(
        leaked.is_none(),
        "docs-only agent must not receive board presence; got: {leaked:?}"
    );

    // Document presence: gated by Docs. The docs agent receives it; the boards
    // agent must not.
    let doc_marker = Uuid::now_v7();
    hub.publish(LiveEvent {
        workspace_id: ws.id.0,
        project_id: None,
        board_id: None,
        document_id: Some(document_id),
        event_type: "presence.updated".to_string(),
        payload: marked_payload("presence.updated", doc_marker),
    });

    let (name, data) = next_sse_event(&mut docs_resp, &mut docs_buf, Duration::from_secs(5))
        .await
        .expect("docs agent must receive document presence");
    assert_eq!(name, "presence.updated");
    assert!(
        data.contains(&doc_marker.to_string()),
        "docs agent receives the document presence event; got: {data}"
    );

    let leaked = next_sse_event(&mut boards_resp, &mut boards_buf, Duration::from_secs(2)).await;
    assert!(
        leaked.is_none(),
        "boards-only agent must not receive document presence; got: {leaked:?}"
    );

    drop(docs_resp);
    drop(boards_resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 7. A workspace-level presence event (no routing id) is delivered to a scoped
//    agent once connected, regardless of the families it holds.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workspace_level_presence_delivered_to_scoped_agent() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let hub = state.live.clone();
    let server = support::TestServer::spawn_with_state(state).await;

    let (_owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "sse-scope-ws-presence").await;

    // An agent holding no read families at all still receives workspace-level
    // events once the connect gate has admitted it.
    let agent_token =
        create_scoped_agent(&db, ws.id, owner.id, "sse-scope-empty-agent", vec![]).await;

    let mut resp = open_stream(server.base_url(), &ws.slug, &agent_token).await;
    let mut buf = String::new();

    let marker = Uuid::now_v7();
    hub.publish(LiveEvent {
        workspace_id: ws.id.0,
        project_id: None,
        board_id: None,
        document_id: None,
        event_type: "presence.updated".to_string(),
        payload: marked_payload("presence.updated", marker),
    });

    let (name, data) = next_sse_event(&mut resp, &mut buf, Duration::from_secs(5))
        .await
        .expect("workspace-level presence must reach a connected agent");
    assert_eq!(name, "presence.updated");
    assert!(
        data.contains(&marker.to_string()),
        "the workspace-level presence frame is delivered; got: {data}"
    );

    drop(resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 8. Negative control: a human member and a root/bypass principal receive events
//    of EVERY family (they carry no scope axis).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn humans_and_root_receive_all_families() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let hub = state.live.clone();
    let server = support::TestServer::spawn_with_state(state).await;

    // The workspace owner is a human member (Admin role, no scope axis).
    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "sse-scope-human").await;
    let owner_token = owner_client.token().expect("owner token").to_string();

    // Root bypasses membership and carries no scope axis either.
    let root_client = support::login_root_user(&server, &db).await;
    let root_token = root_client.token().expect("root token").to_string();

    let mut owner_resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut owner_buf = String::new();
    let mut root_resp = open_stream(server.base_url(), &ws.slug, &root_token).await;
    let mut root_buf = String::new();

    // One workspace-level event per family, in a fixed order.
    let families = [
        "task.created",
        "document.updated",
        "board.created",
        "folder.created",
        "project.created",
    ];

    let mut markers = Vec::new();
    for event_type in families {
        markers.push((event_type, publish_ws_level_event(&hub, ws.id, event_type)));
    }

    for (expected_type, marker) in &markers {
        let (owner_name, owner_data) =
            next_sse_event(&mut owner_resp, &mut owner_buf, Duration::from_secs(5))
                .await
                .expect("human member must receive every family");
        assert_eq!(&owner_name, expected_type, "human receives {expected_type}");
        assert!(
            owner_data.contains(&marker.to_string()),
            "human receives the {expected_type} marker; got: {owner_data}"
        );

        let (root_name, root_data) =
            next_sse_event(&mut root_resp, &mut root_buf, Duration::from_secs(5))
                .await
                .expect("root must receive every family");
        assert_eq!(&root_name, expected_type, "root receives {expected_type}");
        assert!(
            root_data.contains(&marker.to_string()),
            "root receives the {expected_type} marker; got: {root_data}"
        );
    }

    drop(owner_resp);
    drop(root_resp);
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// 9. Fail-closed: an event whose type maps to no known family is dropped for a
//    scoped agent even when it holds every read family, while a human still
//    receives it — proving the drop is the capability filter, not the transport.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unknown_event_type_dropped_for_scoped_agent_only() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let hub = state.live.clone();
    let server = support::TestServer::spawn_with_state(state).await;

    let (owner_client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "sse-scope-failclosed").await;
    let owner_token = owner_client.token().expect("owner token").to_string();

    // The agent holds EVERY read family, so only the unknown-type fail-closed rule
    // can drop the event.
    let agent_token = create_scoped_agent(
        &db,
        ws.id,
        owner.id,
        "sse-scope-allfamilies-agent",
        vec![
            read_cap(CapabilityFamily::Tasks),
            read_cap(CapabilityFamily::Docs),
            read_cap(CapabilityFamily::Boards),
            read_cap(CapabilityFamily::Folders),
            read_cap(CapabilityFamily::Projects),
        ],
    )
    .await;

    let mut agent_resp = open_stream(server.base_url(), &ws.slug, &agent_token).await;
    let mut agent_buf = String::new();
    let mut owner_resp = open_stream(server.base_url(), &ws.slug, &owner_token).await;
    let mut owner_buf = String::new();

    // An unmapped type published first, then a known task sentinel.
    let unknown_marker = publish_ws_level_event(&hub, ws.id, "widget.exploded");
    let task_marker = publish_ws_level_event(&hub, ws.id, "task.created");

    // The scoped agent skips the unknown event: its first frame is the task.
    let (agent_name, agent_data) =
        next_sse_event(&mut agent_resp, &mut agent_buf, Duration::from_secs(5))
            .await
            .expect("agent must still receive the known task event");
    assert_eq!(
        agent_name, "task.created",
        "the unknown-type event must be skipped, so the first frame is the task"
    );
    assert!(
        agent_data.contains(&task_marker.to_string()),
        "the agent's first frame is the task sentinel; got: {agent_data}"
    );

    // The human owner receives the unknown event verbatim: the drop is the
    // capability filter, not the transport.
    let (owner_name, owner_data) =
        next_sse_event(&mut owner_resp, &mut owner_buf, Duration::from_secs(5))
            .await
            .expect("human must receive the unknown event");
    assert_eq!(owner_name, "widget.exploded");
    assert!(
        owner_data.contains(&unknown_marker.to_string()),
        "human receives the unmapped event; got: {owner_data}"
    );

    drop(agent_resp);
    drop(owner_resp);
    db.teardown().await;
}
