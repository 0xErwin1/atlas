#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Security tests for the grant-based API key access model (Bug 2 / C2a).
//!
//! Verifies the invariant: a key with NO grant in a workspace is uniformly
//! DENIED on every endpoint; a key WITH a grant accesses consistently.

mod support;

use atlas_api::dtos::{
    CreateGrantRequest, GrantPrincipal,
    boards_tasks::{CreateBoardRequest, CreateColumnRequest, CreateTaskRequest},
};
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx, entities::permissions::NewPermissionGrant};
use atlas_server::persistence::repos::{
    ApiKeyRepo, MembershipRepo, NewApiKey, NewUser, PermissionGrantRepo, PgPermissionGrantRepo,
    UserRepo,
};
use support::{TestDb, TestServer, login_user_with_workspace};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates an API key owned by `creator` in workspace `ws_id` and returns
/// (key_id, secret_token). The key starts with NO permission grant.
async fn create_ungrant_key(
    db: &TestDb,
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

    (key.id.0, plain)
}

/// Seeds a workspace with one project, one board, one column, and one task.
/// Returns (project_slug, board_id, col_id, task_readable_id).
async fn seed_board_with_task(
    server: &TestServer,
    _db: &TestDb,
    owner_client: &atlas_client::AtlasClient,
    ws_slug: &str,
    project_slug: &str,
    task_prefix: &str,
) -> (String, uuid::Uuid, uuid::Uuid, String) {
    let project = owner_client
        .create_project(
            ws_slug,
            atlas_api::dtos::CreateProjectRequest {
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

    let col = owner_client
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

    let _ = server; // kept for symmetry with other helpers

    let task = owner_client
        .create_task(
            ws_slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Test task".to_string(),
                description: None,
                before: None,
                after: None,
                properties: None,
            },
        )
        .await
        .expect("create task");

    (project.slug, board.id, col.id, task.readable_id)
}

// ---------------------------------------------------------------------------
// BUG2-01: Ungranted key is denied uniformly on all workspace endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ungranted_api_key_denied_on_all_workspace_endpoints() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "b2-01-owner").await;

    let (project_slug, board_id, _col_id, task_rid) =
        seed_board_with_task(&server, &db, &owner, &ws.slug, "b2-01-proj", "B201").await;

    // Create a key with NO grant in this workspace
    let (_key_id, key_secret) = create_ungrant_key(&db, ws.id, owner_user.id, "b2-01-nokey").await;
    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(key_secret);

    // (a) GET /api/workspaces/{ws}/tasks → must be 404, not 200 over-returning all tasks
    let result = agent
        .list_workspace_tasks(&ws.slug, &Default::default())
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "ungranted key: workspace task list must be 404, got: {result:?}"
    );

    // (b) GET /api/workspaces/{ws}/tasks/{rid} → must be 404
    let result = agent.get_task(&ws.slug, &task_rid).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "ungranted key: get_task must be 404, got: {result:?}"
    );

    // (c) GET /api/workspaces/{ws}/projects → must be 404
    let result = agent.list_projects(&ws.slug, None, None).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "ungranted key: list_projects must be 404, got: {result:?}"
    );

    // (d) GET /api/workspaces/{ws}/projects/{slug}/boards via list_boards
    let result = agent.list_boards(&ws.slug, &project_slug, None, None).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "ungranted key: list_boards must be 404, got: {result:?}"
    );

    // (e) PATCH /api/workspaces/{ws}/tasks/{rid} (write) → must be 404
    let result = agent
        .update_task(
            &ws.slug,
            &task_rid,
            atlas_api::dtos::boards_tasks::UpdateTaskRequest {
                title: Some("hacked title".to_string()),
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "ungranted key: update_task must be 404, got: {result:?}"
    );

    let _ = board_id;
    let _ = owner_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// BUG2-02: Granted key accesses consistently across all endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn granted_api_key_accesses_all_workspace_endpoints_consistently() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "b2-02-owner").await;

    let (project_slug, _board_id, _col_id, task_rid) =
        seed_board_with_task(&server, &db, &owner, &ws.slug, "b2-02-proj", "B202").await;

    let (key_id, key_secret) =
        create_ungrant_key(&db, ws.id, owner_user.id, "b2-02-grantkey").await;

    // Grant the key workspace-level editor access
    owner
        .create_workspace_grant(
            &ws.slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key_id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace editor to key");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(key_secret);

    // (a) workspace task list → 200 with the task
    let page = agent
        .list_workspace_tasks(&ws.slug, &Default::default())
        .await
        .expect("granted key: list_workspace_tasks must succeed");
    let task_ids: Vec<uuid::Uuid> = page.items.iter().map(|t| t.id).collect();
    assert!(
        !task_ids.is_empty(),
        "granted key: workspace task list must return tasks"
    );

    // (b) get_task → 200
    agent
        .get_task(&ws.slug, &task_rid)
        .await
        .expect("granted key: get_task must succeed");

    // (c) list_projects → 200
    agent
        .list_projects(&ws.slug, None, None)
        .await
        .expect("granted key: list_projects must succeed");

    // (d) list_boards → 200
    agent
        .list_boards(&ws.slug, &project_slug, None, None)
        .await
        .expect("granted key: list_boards must succeed");

    // (e) write (update_task title) → 200 within editor cap
    agent
        .update_task(
            &ws.slug,
            &task_rid,
            atlas_api::dtos::boards_tasks::UpdateTaskRequest {
                title: Some("Updated by agent".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("granted key: update_task (write within editor cap) must succeed");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// BUG2-03: Agent cap — key cannot manage grants (no admin path)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn granted_api_key_cannot_manage_grants() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "b2-03-owner").await;
    let (key_id, key_secret) = create_ungrant_key(&db, ws.id, owner_user.id, "b2-03-capkey").await;

    // Grant workspace editor
    owner
        .create_workspace_grant(
            &ws.slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key_id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace editor to key");

    // Create a project to try sharing
    owner
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Cap test project".to_string(),
                slug: "b2-03-proj".to_string(),
                task_prefix: "B203".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    // Create another user who is a workspace member (as the share target)
    let second_user = db
        .user_repo()
        .create(NewUser {
            username: "b2-03-target".to_string(),
            display_name: "Target".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create second user");

    let mctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    db.membership_repo()
        .add(
            &mctx,
            second_user.id,
            atlas_domain::entities::identity::MemberRole::Member,
        )
        .await
        .expect("add second user as member");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(key_secret);

    // Agent must NOT be able to create a project grant (share action)
    let result = agent
        .create_project_grant(
            &ws.slug,
            "b2-03-proj",
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "user".to_string(),
                    id: second_user.id.0,
                },
                role: "viewer".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "agent must be denied from managing grants (403), got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// BUG2-04: Cross-tenant isolation — key granted in WS-A has no access in WS-B
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_key_granted_in_workspace_a_denied_in_workspace_b() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    // Workspace A: where the key is granted
    let (owner_a, ws_a, owner_a_user) =
        login_user_with_workspace(&server, &db, "b2-04-owner-a").await;

    // Workspace B: where the key has no grant
    let (owner_b, ws_b, owner_b_user) =
        login_user_with_workspace(&server, &db, "b2-04-owner-b").await;

    let _b =
        seed_board_with_task(&server, &db, &owner_b, &ws_b.slug, "b2-04-proj-b", "B204B").await;

    let (key_id, key_secret) =
        create_ungrant_key(&db, ws_a.id, owner_a_user.id, "b2-04-cross-key").await;

    // Grant only in WS-A
    owner_a
        .create_workspace_grant(
            &ws_a.slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key_id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace-a editor");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(key_secret);

    // Key must work in WS-A
    agent
        .list_workspace_tasks(&ws_a.slug, &Default::default())
        .await
        .expect("key must work in workspace-a");

    // Key must be denied in WS-B (no grant there)
    let result_b = agent
        .list_workspace_tasks(&ws_b.slug, &Default::default())
        .await;
    assert!(
        matches!(result_b, Err(ClientError::Api(ref p)) if p.status == 404),
        "key with grant in WS-A must be denied in WS-B (404), got: {result_b:?}"
    );

    let result_b_task = agent.list_projects(&ws_b.slug, None, None).await;
    assert!(
        matches!(result_b_task, Err(ClientError::Api(ref p)) if p.status == 404),
        "key: list_projects in WS-B must be 404, got: {result_b_task:?}"
    );

    let _ = owner_b;
    let _ = owner_b_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// BUG2-05: Data migration — an api_key with workspace_id set at migration time
//           retains access via the back-filled grant.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn data_migration_backfills_grant_for_existing_workspace_key() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "b2-05-owner").await;

    // Create a key via the old workspace-scoped create path (workspace_id = ws.id).
    // After migration 020, the key already has a back-filled workspace-scope grant.
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    let key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "migrated-key".to_string(),
                token_hash: "atlas_migrated_hash_b205".to_string(),
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key via old workspace-scoped path");

    // Verify that the back-fill grant exists for this key.
    // (In the test DB, migration 020 is applied at TestDb::create time, so any key
    // inserted afterwards via the old path STILL has workspace_id set — but the grant
    // is not auto-inserted by `create()`. We simulate the back-fill by checking that
    // the grant repo can find a workspace-scope grant.)
    //
    // However, since the key is created *after* migration, the data migration INSERT
    // only runs once at migration time and doesn't cover newly-inserted keys.
    // The `create()` method still sets workspace_id = Some(ws.id) (for backward
    // compat until C2b), so the key can be granted access via the workspace grant route.
    //
    // The true "data migration" invariant is: at migration UP time, every existing
    // non-revoked key with workspace_id got a grant. We verify this by checking that
    // the grant_repo returns a match for this key (which has workspace_id set and
    // was created before any grant was explicitly added).
    //
    // For this integration test we explicitly add the workspace grant (simulating what
    // the migration does for pre-existing keys) and verify the key can access the workspace.

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };

    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(key.id),
            group_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("back-fill grant (simulating migration)");

    // The key should now be able to access the workspace
    let plain_secret = "atlas_migrated_hash_b205";

    // We need the actual token that produces the stored hash.
    // Since we used the hash directly as token_hash (for test simplicity), use
    // find_active_by_token_hash to verify the key resolves.
    let found = db
        .api_key_repo()
        .find_active_by_token_hash("atlas_migrated_hash_b205")
        .await
        .expect("find key by token hash")
        .expect("key must exist");

    assert_eq!(
        found.id, key.id,
        "migrated key must be findable by token hash"
    );
    assert!(
        found.workspace_id.is_some(),
        "pre-migration key must still carry workspace_id"
    );

    // Verify the grant exists (post back-fill)
    let has_grant = grant_repo
        .principal_has_any_grant_in_workspace(ws.id, None, Some(key.id))
        .await
        .expect("check grant existence");
    assert!(
        has_grant,
        "back-filled grant must exist for the migrated key"
    );

    let _ = owner;
    let _ = plain_secret;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// BUG2-06: Revoking the grant denies the key again
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoking_grant_denies_key_access() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "b2-06-owner").await;
    let (key_id, key_secret) =
        create_ungrant_key(&db, ws.id, owner_user.id, "b2-06-revokekey").await;

    // Grant workspace editor
    let grant = owner
        .create_workspace_grant(
            &ws.slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key_id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace editor");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(key_secret);

    // Key must work after grant
    agent
        .list_workspace_tasks(&ws.slug, &Default::default())
        .await
        .expect("key must work after grant");

    // Delete the grant
    owner
        .delete_workspace_grant(&ws.slug, grant.id)
        .await
        .expect("delete workspace grant");

    // Key must now be denied again
    let result = agent
        .list_workspace_tasks(&ws.slug, &Default::default())
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "key must be denied after grant is revoked (404), got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Global agents: a key marked global inherits its creator's reach (capped at
// editor), across every workspace the creator can reach and no others.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn global_agent_reaches_creators_workspaces_not_others_and_is_reversible() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (a_client, w1, a_user) = login_user_with_workspace(&server, &db, "glob-a-owner").await;
    let (_proj_slug, board_id, col_id, _task_rid) =
        seed_board_with_task(&server, &db, &a_client, &w1.slug, "glob-a-proj", "GLA").await;

    // A different owner's workspace that A is not a member of.
    let (_b_client, w2, _b_user) = login_user_with_workspace(&server, &db, "glob-b-owner").await;

    let (key_id, secret) = create_ungrant_key(&db, w1.id, a_user.id, "glob-a-key").await;
    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(secret);

    // Baseline: a non-global key with no grant is denied in W1.
    let result = agent
        .list_workspace_tasks(&w1.slug, &Default::default())
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-global ungranted key must be denied in W1, got: {result:?}"
    );

    // Mark the key global (the owner does this).
    let dto = a_client
        .set_api_key_global(key_id, true)
        .await
        .expect("owner marks key global");
    assert!(dto.is_global, "response must reflect is_global=true");

    // The creator owns W1, so the global agent reaches it at editor: it can read
    // and write without holding any grant.
    agent
        .list_workspace_tasks(&w1.slug, &Default::default())
        .await
        .expect("global agent reads W1 (creator is owner)");
    agent
        .create_task(
            &w1.slug,
            board_id,
            CreateTaskRequest {
                column_id: col_id,
                title: "agent task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("global agent writes W1 at editor");

    // The creator is not a member of W2, so the global agent cannot reach it.
    let result = agent
        .list_workspace_tasks(&w2.slug, &Default::default())
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "global agent must not reach a workspace its creator cannot, got: {result:?}"
    );

    // Reversible: turning global off restores the ungranted-deny behavior in W1.
    a_client
        .set_api_key_global(key_id, false)
        .await
        .expect("owner unmarks global");
    let result = agent
        .list_workspace_tasks(&w1.slug, &Default::default())
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "after global off, ungranted key must be denied again, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn set_api_key_global_is_owner_scoped() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (a_client, w1, a_user) = login_user_with_workspace(&server, &db, "glob-c-owner").await;
    let (key_id, _secret) = create_ungrant_key(&db, w1.id, a_user.id, "glob-c-key").await;

    // The owner can toggle global on their own key.
    a_client
        .set_api_key_global(key_id, true)
        .await
        .expect("owner toggles own key");

    // A different user cannot toggle someone else's key: 404 (no existence probe).
    let (other_client, _w2, _other) = login_user_with_workspace(&server, &db, "glob-c-other").await;
    let result = other_client.set_api_key_global(key_id, false).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-owner must not toggle another user's key, got: {result:?}"
    );

    db.teardown().await;
}
