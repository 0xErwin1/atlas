#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{ApiKeyType, MemberRole},
    entities::permissions::NewPermissionGrant,
    ids::{UserId, WorkspaceId},
    permissions::{Capability, CapabilityAction, CapabilityFamily, ResourceRole},
};
use atlas_server::{
    auth::password,
    crypto::WebhookCrypto,
    persistence::repos::{
        ApiKeyRepo, MembershipRepo, NewApiKey, NewUser, PermissionGrantRepo, PgApiKeyRepo,
        PgMembershipRepo, PgPermissionGrantRepo, PgUserRepo, PgWebhookSubscriptionRepo, UserRepo,
        WebhookSubscriptionPatch,
    },
};
use serde_json::Value;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Encrypts a fixed dummy secret and calls `PgWebhookSubscriptionRepo::create`.
async fn insert_test_subscription(
    db: &support::TestDb,
    ws_id: Uuid,
    user_id: atlas_domain::ids::UserId,
    url: &str,
    event_types: Vec<String>,
) -> Uuid {
    let crypto = WebhookCrypto::new(&[0x42u8; 32]);
    let dummy_plaintext = b"test-hmac-secret-32-bytes-dummy!";
    let (enc, nonce) = crypto.encrypt(dummy_plaintext).unwrap();

    let row = PgWebhookSubscriptionRepo::create(
        db.conn(),
        ws_id,
        url.to_string(),
        event_types,
        "workspace".to_string(),
        None,
        enc,
        nonce,
        None,
        &Actor::User(user_id),
    )
    .await
    .expect("create subscription");

    row.id
}

/// Creates a Member-role user in the given workspace and logs in, returning the token.
async fn add_member_user_and_login(
    server: &support::TestServer,
    db: &support::TestDb,
    ws_id: Uuid,
    username: &str,
) -> String {
    use atlas_api::dtos::LoginRequest;

    let password_plaintext = "TestPassword1!";
    let password_hash = password::hash(password_plaintext.to_string())
        .await
        .expect("hash");

    let user_repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };

    let user = user_repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(password_hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member user");

    support::activate_user_in_db(db, user.id.0).await;

    let ws_id_typed = WorkspaceId::from(ws_id);
    let ctx = WorkspaceCtx::new(ws_id_typed, Actor::User(user.id));
    membership_repo
        .add(&ctx, user.id, MemberRole::Member)
        .await
        .expect("add member");

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/api/auth/login", server.base_url()))
        .json(&LoginRequest {
            username: username.to_string(),
            password: password_plaintext.to_string(),
        })
        .send()
        .await
        .expect("login");

    let body: Value = resp.json().await.expect("login body");
    body["token"].as_str().expect("token").to_string()
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

/// Builds a single `webhooks:{action}` capability.
fn webhook_cap(action: CapabilityAction) -> Capability {
    Capability {
        family: CapabilityFamily::Webhooks,
        action,
    }
}

/// Creates a GLOBAL agent key owned by `owner_id` with the given scope set and
/// returns its plaintext bearer token. A global key inherits its creator's reach
/// on every resource, capped at `Editor` — so an Owner-created global agent
/// resolves to exactly the `Editor` reach the webhook role floor admits.
async fn create_global_agent(
    db: &support::TestDb,
    owner_id: UserId,
    name: &str,
    scopes: Vec<Capability>,
) -> String {
    let plain = format!("atlas_{name}_secret_{}", Uuid::now_v7().as_simple());
    let hash = atlas_server::auth::tokens::hash_token(&plain);

    let key = db
        .api_key_repo()
        .create_for_user(
            owner_id,
            NewApiKey {
                name: name.to_string(),
                token_hash: hash,
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes,
            },
        )
        .await
        .expect("create global agent key");

    PgApiKeyRepo::set_global_for_user_in(db.conn(), owner_id, key.id, true)
        .await
        .expect("make key global");

    plain
}

/// Creates a NON-global agent key owned by `owner_id`, granted only `Viewer` on
/// the workspace, and returns its plaintext bearer token. Its effective reach is
/// `min(Viewer grant, Owner creator) = Viewer`, i.e. below the webhook `Editor`
/// role floor even though its creator is an admin — used to prove capability
/// never bypasses the role floor.
async fn create_viewer_reach_agent(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    owner_id: UserId,
    name: &str,
    scopes: Vec<Capability>,
) -> String {
    let plain = format!("atlas_{name}_secret_{}", Uuid::now_v7().as_simple());
    let hash = atlas_server::auth::tokens::hash_token(&plain);

    let key = db
        .api_key_repo()
        .create_for_user(
            owner_id,
            NewApiKey {
                name: name.to_string(),
                token_hash: hash,
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes,
            },
        )
        .await
        .expect("create viewer-reach agent key");

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
    .expect("grant viewer reach to agent");

    plain
}

/// Creates a workspace `Member` user and grants them an explicit `Editor` role on
/// the workspace, then logs in and returns the token. This yields a human whose
/// effective role is `Editor` — a non-admin below the webhook `AdminMin` human
/// floor, used to prove the floor is not leaked to human editors.
async fn add_editor_user_and_login(
    server: &support::TestServer,
    db: &support::TestDb,
    ws_id: WorkspaceId,
    owner_id: UserId,
    username: &str,
) -> String {
    use atlas_api::dtos::LoginRequest;

    let password_plaintext = "TestPassword1!";
    let password_hash = password::hash(password_plaintext.to_string())
        .await
        .expect("hash");

    let user_repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };

    let user = user_repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(password_hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create editor user");

    support::activate_user_in_db(db, user.id.0).await;

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(user.id));
    membership_repo
        .add(&ctx, user.id, MemberRole::Member)
        .await
        .expect("add member");

    PgPermissionGrantRepo {
        conn: db.conn().clone(),
    }
    .upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: Some(user.id),
        api_key_id: None,
        group_id: None,
        project_id: None,
        folder_id: None,
        document_id: None,
        board_id: None,
        role: ResourceRole::Editor,
        created_by_user_id: Some(owner_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant editor to member");

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/api/auth/login", server.base_url()))
        .json(&LoginRequest {
            username: username.to_string(),
            password: password_plaintext.to_string(),
        })
        .send()
        .await
        .expect("login");

    let body: Value = resp.json().await.expect("login body");
    body["token"].as_str().expect("token").to_string()
}

/// The six webhook routes as `(method, path-suffix-after-{ws}/webhooks)` pairs,
/// each paired with the capability its per-action gate requires. `{id}` is
/// substituted with a real webhook id by the callers. Used by the authz matrix
/// to exercise every route uniformly.
fn webhook_routes(id: &str) -> Vec<(&'static str, String, CapabilityAction)> {
    vec![
        ("POST", String::new(), CapabilityAction::Create),
        ("GET", String::new(), CapabilityAction::Read),
        ("GET", format!("/{id}"), CapabilityAction::Read),
        ("PATCH", format!("/{id}"), CapabilityAction::Update),
        ("DELETE", format!("/{id}"), CapabilityAction::Delete),
        ("GET", format!("/{id}/deliveries"), CapabilityAction::Read),
    ]
}

/// Fires one webhook route with the given bearer token, returning the HTTP
/// status. `POST`/`PATCH` carry a valid body so an authorized call reaches a 2xx
/// rather than a body-validation error.
async fn call_webhook_route(
    base_url: &str,
    ws_slug: &str,
    token: &str,
    method: &str,
    suffix: &str,
) -> reqwest::StatusCode {
    let url = format!("{base_url}/api/workspaces/{ws_slug}/webhooks{suffix}");
    let client = http();

    let builder = match method {
        "POST" => client.post(&url).json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.created"],
            "scope_type": "workspace"
        })),
        "PATCH" => client
            .patch(&url)
            .json(&serde_json::json!({ "is_active": false })),
        "DELETE" => client.delete(&url),
        _ => client.get(&url),
    };

    builder
        .bearer_auth(token)
        .send()
        .await
        .expect("webhook request")
        .status()
}

// ---------------------------------------------------------------------------
// B4.4-1: create persists a row retrievable by get_by_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_repo_create_and_get_by_id() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "wh-repo-create").await;

    let id = insert_test_subscription(
        &db,
        ws.id.0,
        user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await;

    let row = PgWebhookSubscriptionRepo::get_by_id(db.conn(), ws.id.0, id)
        .await
        .expect("get_by_id")
        .expect("must exist");

    assert_eq!(row.id, id);
    assert_eq!(row.workspace_id, ws.id.0);
    assert_eq!(row.target_url, "https://example.com/hook");
    assert_eq!(row.event_types, vec!["task.created"]);
    assert_eq!(row.scope_type, "workspace");
    assert!(row.scope_id.is_none());
    assert!(row.is_active);
    assert!(row.deleted_at.is_none());

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.4-2: list_active returns rows, supports cursor-based pagination
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_repo_list_active_pagination() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "wh-repo-list").await;

    for i in 0..3u32 {
        insert_test_subscription(
            &db,
            ws.id.0,
            user.id,
            &format!("https://example.com/hook{i}"),
            vec!["task.created".to_string()],
        )
        .await;
    }

    let page1 = PgWebhookSubscriptionRepo::list_active(db.conn(), ws.id.0, None, 2)
        .await
        .expect("list page 1");
    assert_eq!(page1.len(), 2);

    let cursor = page1.last().unwrap().id;
    let page2 = PgWebhookSubscriptionRepo::list_active(db.conn(), ws.id.0, Some(cursor), 10)
        .await
        .expect("list page 2");
    assert_eq!(page2.len(), 1);
    assert!(
        page2[0].id > cursor,
        "page-2 items must be after the cursor"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.4-3: soft_delete hides the row from get_by_id and list_active
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_repo_soft_delete() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "wh-repo-del").await;

    let id = insert_test_subscription(
        &db,
        ws.id.0,
        user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await;

    PgWebhookSubscriptionRepo::soft_delete(db.conn(), ws.id.0, id)
        .await
        .expect("soft_delete");

    let found = PgWebhookSubscriptionRepo::get_by_id(db.conn(), ws.id.0, id)
        .await
        .expect("get_by_id");
    assert!(
        found.is_none(),
        "deleted subscription must not be returned by get_by_id"
    );

    let list = PgWebhookSubscriptionRepo::list_active(db.conn(), ws.id.0, None, 100)
        .await
        .expect("list_active");
    assert!(
        list.iter().all(|r| r.id != id),
        "deleted subscription must not appear in list_active"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.4-4: update patches selected fields
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_repo_update_patches_fields() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "wh-repo-upd").await;

    let id = insert_test_subscription(
        &db,
        ws.id.0,
        user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await;

    let patch = WebhookSubscriptionPatch {
        target_url: Some("https://example.com/hook-v2".to_string()),
        event_types: Some(vec!["task.updated".to_string()]),
        scope_type: None,
        scope_id: None,
        encrypted_secret: None,
        secret_nonce: None,
        is_active: Some(false),
        label: Some(Some("my-hook".to_string())),
    };

    let updated = PgWebhookSubscriptionRepo::update(db.conn(), ws.id.0, id, patch)
        .await
        .expect("update");

    assert_eq!(updated.target_url, "https://example.com/hook-v2");
    assert_eq!(updated.event_types, vec!["task.updated"]);
    assert!(!updated.is_active);
    assert_eq!(updated.label.as_deref(), Some("my-hook"));

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.4-5: cross-workspace isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn webhook_repo_cross_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws1, user1) = support::seed_workspace(&db, "wh-iso-ws1").await;
    let (ws2, _user2) = support::seed_workspace(&db, "wh-iso-ws2").await;

    let id = insert_test_subscription(
        &db,
        ws1.id.0,
        user1.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await;

    let found = PgWebhookSubscriptionRepo::get_by_id(db.conn(), ws2.id.0, id)
        .await
        .expect("get_by_id ws2");
    assert!(found.is_none(), "ws2 must not see ws1 subscriptions");

    let list = PgWebhookSubscriptionRepo::list_active(db.conn(), ws2.id.0, None, 100)
        .await
        .expect("list ws2");
    assert!(
        list.is_empty(),
        "ws2 list_active must not see ws1 subscriptions"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.5-1: admin (Owner role) creates webhook — 201, secret present
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_creates_webhook_returns_201_with_secret() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "wh-admin-create").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.created", "task.deleted"],
            "scope_type": "workspace"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201, "admin create must return 201");

    let body: Value = resp.json().await.unwrap();
    assert!(body["id"].is_string(), "id must be present");
    assert!(
        body["secret"].is_string(),
        "secret must be present in create response"
    );

    let secret = body["secret"].as_str().unwrap();
    assert!(
        secret.starts_with("whsec_"),
        "secret must start with whsec_: {secret}"
    );
    assert_eq!(body["scope_type"], "workspace");
    assert!(body["scope_id"].is_null());
    assert_eq!(body["is_active"], true);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.5-2: non-admin (Member role) is rejected on all endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_admin_is_rejected_on_all_crud() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (admin_client, ws, _admin_user) =
        support::login_user_with_workspace(&server, &db, "wh-nonadmin-admin").await;

    let admin_token = admin_client.token().expect("admin token");
    let ws_slug = &ws.slug;
    let base_url = server.base_url();

    let create_resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.created"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201, "admin create must succeed first");
    let created: Value = create_resp.json().await.unwrap();
    let hook_id = created["id"].as_str().unwrap().to_string();

    let member_token = add_member_user_and_login(&server, &db, ws.id.0, "wh-nonadmin-editor").await;

    // The permission engine returns 404 for non-admin members on workspace resources
    // with no explicit visibility — this is intentional security-by-obscurity behavior
    // that prevents existence disclosure. The important property is that Members are
    // rejected; the exact status (404) is derived from the permission engine's design.
    let list_resp = http()
        .get(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(&member_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        list_resp.status(),
        404,
        "non-admin list must be rejected (404)"
    );

    let get_resp = http()
        .get(format!(
            "{base_url}/api/workspaces/{ws_slug}/webhooks/{hook_id}"
        ))
        .bearer_auth(&member_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        get_resp.status(),
        404,
        "non-admin get must be rejected (404)"
    );

    let patch_resp = http()
        .patch(format!(
            "{base_url}/api/workspaces/{ws_slug}/webhooks/{hook_id}"
        ))
        .bearer_auth(&member_token)
        .json(&serde_json::json!({"is_active": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        patch_resp.status(),
        404,
        "non-admin patch must be rejected (404)"
    );

    let post_resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(&member_token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.created"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        post_resp.status(),
        404,
        "non-admin create must be rejected (404)"
    );

    let delete_resp = http()
        .delete(format!(
            "{base_url}/api/workspaces/{ws_slug}/webhooks/{hook_id}"
        ))
        .bearer_auth(&member_token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        delete_resp.status(),
        404,
        "non-admin delete must be rejected (404)"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.5-3: list and get responses contain no secret field
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_and_get_responses_contain_no_secret() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) = support::login_user_with_workspace(&server, &db, "wh-nosecret").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.created"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let created: Value = create_resp.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let list_resp = http()
        .get(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200);
    let list_body: Value = list_resp.json().await.unwrap();
    let items = list_body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(
        items[0]["secret"].is_null(),
        "list item must not expose secret"
    );
    assert!(
        items[0]["encrypted_secret"].is_null(),
        "list item must not expose encrypted_secret"
    );

    let get_resp = http()
        .get(format!("{base_url}/api/workspaces/{ws_slug}/webhooks/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), 200);
    let get_body: Value = get_resp.json().await.unwrap();
    assert!(
        get_body["secret"].is_null(),
        "get response must not expose secret"
    );
    assert!(
        get_body["encrypted_secret"].is_null(),
        "get response must not expose encrypted_secret"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.5-4: validation — empty event_types rejected with 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_webhook_rejects_private_target_url_by_default() {
    let db = support::TestDb::create().await.expect("TestDb");
    let mut state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    state.allow_private_webhook_targets = false;
    let server = support::TestServer::spawn_with_state(state).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "wh-val-private-url").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": "http://169.254.169.254/latest/meta-data",
            "event_types": ["task.created"]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 422, "private target_url must return 422");

    db.teardown().await;
}

#[tokio::test]
async fn create_webhook_rejects_empty_event_types() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "wh-val-empty-types").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": []
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 422, "empty event_types must return 422");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.5-5: validation — unknown event_type rejected with 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_webhook_rejects_unknown_event_type() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "wh-val-unknown-type").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.nonexistent"]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 422, "unknown event_type must return 422");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.5-6: validation — board scope without scope_id rejected with 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_webhook_rejects_missing_scope_id_for_board_scope() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "wh-val-scope").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.created"],
            "scope_type": "board"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        422,
        "board scope without scope_id must return 422"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.5-7: PATCH toggles is_active, response has no secret
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_can_toggle_is_active_and_patch_has_no_secret() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) = support::login_user_with_workspace(&server, &db, "wh-toggle").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.created"]
        }))
        .send()
        .await
        .unwrap();
    let created: Value = create_resp.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let patch_resp = http()
        .patch(format!("{base_url}/api/workspaces/{ws_slug}/webhooks/{id}"))
        .bearer_auth(token)
        .json(&serde_json::json!({"is_active": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_resp.status(), 200);
    let patched: Value = patch_resp.json().await.unwrap();
    assert_eq!(patched["is_active"], false);
    assert!(
        patched["secret"].is_null(),
        "patch response must not expose secret"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.5-8: DELETE soft-deletes (204), subsequent GET returns 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_delete_returns_204_then_get_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) = support::login_user_with_workspace(&server, &db, "wh-delete").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!("{base_url}/api/workspaces/{ws_slug}/webhooks"))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "target_url": "https://example.com/hook",
            "event_types": ["task.created"]
        }))
        .send()
        .await
        .unwrap();
    let created: Value = create_resp.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let delete_resp = http()
        .delete(format!("{base_url}/api/workspaces/{ws_slug}/webhooks/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(delete_resp.status(), 204, "delete must return 204");

    let get_resp = http()
        .get(format!("{base_url}/api/workspaces/{ws_slug}/webhooks/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        get_resp.status(),
        404,
        "deleted subscription must return 404 on get"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Principal-aware role floor: agents reach webhooks at Editor + capability
// ---------------------------------------------------------------------------

/// Post-change posture (inverts the former `agent_with_all_capabilities_cannot_list_webhooks`):
/// webhooks are no longer agent-unreachable. An `ApiKey` principal is admitted at
/// the `Editor` role floor when it holds the matching `webhooks:{action}`
/// capability, so a global agent (Owner-created, hence `Editor` reach) scoped to
/// `webhooks:read` can now list webhooks. Humans still require `AdminMin`.
#[tokio::test]
async fn agent_with_webhooks_read_capability_can_list_webhooks() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "wh-agent-read-owner").await;

    insert_test_subscription(
        &db,
        ws.id.0,
        owner_user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await;

    let token = create_global_agent(
        &db,
        owner_user.id,
        "wh-agent-read",
        vec![webhook_cap(CapabilityAction::Read)],
    )
    .await;

    let resp = http()
        .get(format!(
            "{}/api/workspaces/{}/webhooks",
            server.base_url(),
            ws.slug
        ))
        .bearer_auth(&token)
        .send()
        .await
        .expect("request");

    assert_eq!(
        resp.status(),
        200,
        "an agent with webhooks:read and Editor reach must be able to list webhooks"
    );

    let body: Value = resp.json().await.expect("list body");
    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "the seeded webhook must be listed");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Authz matrix: human admin / human editor / agent per-action / role floor
// ---------------------------------------------------------------------------

/// A human workspace admin (Owner) still succeeds on all six webhook routes —
/// the change preserves the human path byte-for-byte. Routes are exercised in an
/// order that keeps the shared subscription alive until the terminal delete.
#[tokio::test]
async fn human_admin_succeeds_on_all_webhook_routes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "wh-matrix-admin").await;

    let token = client.token().expect("token");
    let base = server.base_url();
    let slug = &ws.slug;

    assert_eq!(
        call_webhook_route(base, slug, token, "POST", "").await,
        201,
        "admin create must be 201"
    );

    let id = insert_test_subscription(
        &db,
        ws.id.0,
        owner_user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await
    .to_string();

    assert_eq!(
        call_webhook_route(base, slug, token, "GET", "").await,
        200,
        "admin list must be 200"
    );
    assert_eq!(
        call_webhook_route(base, slug, token, "GET", &format!("/{id}")).await,
        200,
        "admin get must be 200"
    );
    assert_eq!(
        call_webhook_route(base, slug, token, "GET", &format!("/{id}/deliveries")).await,
        200,
        "admin list-deliveries must be 200"
    );
    assert_eq!(
        call_webhook_route(base, slug, token, "PATCH", &format!("/{id}")).await,
        200,
        "admin patch must be 200"
    );
    assert_eq!(
        call_webhook_route(base, slug, token, "DELETE", &format!("/{id}")).await,
        204,
        "admin delete must be 204"
    );

    db.teardown().await;
}

/// A non-admin human whose effective role is `Editor` is rejected on every
/// webhook route. This is the critical guard: the principal-aware floor lowers
/// the bar for `ApiKey` principals only, so the human `AdminMin` floor must not
/// leak to human editors. Every call is denied at the role floor (403) before
/// the handler body, so the shared subscription survives the whole loop.
#[tokio::test]
async fn human_editor_is_forbidden_on_all_webhook_routes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "wh-matrix-humedit").await;

    let id = insert_test_subscription(
        &db,
        ws.id.0,
        owner_user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await
    .to_string();

    let editor_token =
        add_editor_user_and_login(&server, &db, ws.id, owner_user.id, "wh-matrix-editor").await;

    let base = server.base_url();
    let slug = &ws.slug;

    for (method, suffix, _action) in webhook_routes(&id) {
        let status = call_webhook_route(base, slug, &editor_token, method, &suffix).await;
        assert_eq!(
            status, 403,
            "human editor must be forbidden on {method} /webhooks{suffix}"
        );
    }

    db.teardown().await;
}

/// Per-action gating for an agent at the `Editor` role floor: each capability
/// admits exactly its own route(s) and rejects the others. `webhooks:read` reads
/// but cannot mutate; `create`/`update`/`delete` each pass only their own verb.
/// Ordered so the terminal `delete` runs last, keeping the fixture alive for the
/// read/update assertions.
#[tokio::test]
async fn agent_per_action_capability_is_enforced() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "wh-matrix-agent").await;

    let id = insert_test_subscription(
        &db,
        ws.id.0,
        owner_user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await
    .to_string();

    let base = server.base_url();
    let slug = &ws.slug;

    // Read agent: reads succeed, every mutation is scope-denied (403).
    let read_token = create_global_agent(
        &db,
        owner_user.id,
        "wh-agent-read-only",
        vec![webhook_cap(CapabilityAction::Read)],
    )
    .await;
    assert_eq!(
        call_webhook_route(base, slug, &read_token, "GET", "").await,
        200,
        "read agent must list"
    );
    assert_eq!(
        call_webhook_route(base, slug, &read_token, "GET", &format!("/{id}")).await,
        200,
        "read agent must get"
    );
    assert_eq!(
        call_webhook_route(base, slug, &read_token, "GET", &format!("/{id}/deliveries")).await,
        200,
        "read agent must list deliveries"
    );
    assert_eq!(
        call_webhook_route(base, slug, &read_token, "POST", "").await,
        403,
        "read agent must NOT create"
    );
    assert_eq!(
        call_webhook_route(base, slug, &read_token, "PATCH", &format!("/{id}")).await,
        403,
        "read agent must NOT update"
    );
    assert_eq!(
        call_webhook_route(base, slug, &read_token, "DELETE", &format!("/{id}")).await,
        403,
        "read agent must NOT delete"
    );

    // Create agent: create succeeds, list is scope-denied.
    let create_token = create_global_agent(
        &db,
        owner_user.id,
        "wh-agent-create-only",
        vec![webhook_cap(CapabilityAction::Create)],
    )
    .await;
    assert_eq!(
        call_webhook_route(base, slug, &create_token, "POST", "").await,
        201,
        "create agent must create"
    );
    assert_eq!(
        call_webhook_route(base, slug, &create_token, "GET", "").await,
        403,
        "create agent must NOT list"
    );

    // Update agent: patch succeeds, list is scope-denied.
    let update_token = create_global_agent(
        &db,
        owner_user.id,
        "wh-agent-update-only",
        vec![webhook_cap(CapabilityAction::Update)],
    )
    .await;
    assert_eq!(
        call_webhook_route(base, slug, &update_token, "PATCH", &format!("/{id}")).await,
        200,
        "update agent must patch"
    );
    assert_eq!(
        call_webhook_route(base, slug, &update_token, "GET", "").await,
        403,
        "update agent must NOT list"
    );

    // Delete agent (runs last, removes the fixture): delete succeeds, list denied.
    let delete_token = create_global_agent(
        &db,
        owner_user.id,
        "wh-agent-delete-only",
        vec![webhook_cap(CapabilityAction::Delete)],
    )
    .await;
    assert_eq!(
        call_webhook_route(base, slug, &delete_token, "GET", "").await,
        403,
        "delete agent must NOT list"
    );
    assert_eq!(
        call_webhook_route(base, slug, &delete_token, "DELETE", &format!("/{id}")).await,
        204,
        "delete agent must delete"
    );

    db.teardown().await;
}

/// An agent with Editor reach but only a foreign capability (`tasks:read`, no
/// webhooks scope) is scope-denied (403) on every webhook route: capability is
/// per-action and per-family, and nothing here grants webhooks.
#[tokio::test]
async fn agent_without_webhooks_capability_is_forbidden_on_all_routes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "wh-matrix-nocap").await;

    let id = insert_test_subscription(
        &db,
        ws.id.0,
        owner_user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await
    .to_string();

    let token = create_global_agent(
        &db,
        owner_user.id,
        "wh-agent-tasks-read",
        vec![Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        }],
    )
    .await;

    let base = server.base_url();
    let slug = &ws.slug;

    for (method, suffix, _action) in webhook_routes(&id) {
        let status = call_webhook_route(base, slug, &token, method, &suffix).await;
        assert_eq!(
            status, 403,
            "agent without webhooks capability must be forbidden on {method} /webhooks{suffix}"
        );
    }

    db.teardown().await;
}

/// Capability never bypasses the role floor: an agent holding the full
/// `webhooks:*` capability set but only `Viewer` reach (below the `Editor`
/// floor) is rejected on every route (403). The floor check runs before the
/// capability gate, so all calls are role-denied and no fixture is mutated.
#[tokio::test]
async fn agent_with_all_webhooks_caps_but_viewer_reach_is_forbidden() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "wh-matrix-viewer").await;

    let id = insert_test_subscription(
        &db,
        ws.id.0,
        owner_user.id,
        "https://example.com/hook",
        vec!["task.created".to_string()],
    )
    .await
    .to_string();

    let token = create_viewer_reach_agent(
        &db,
        ws.id,
        owner_user.id,
        "wh-agent-viewer",
        vec![
            webhook_cap(CapabilityAction::Read),
            webhook_cap(CapabilityAction::Create),
            webhook_cap(CapabilityAction::Update),
            webhook_cap(CapabilityAction::Delete),
        ],
    )
    .await;

    let base = server.base_url();
    let slug = &ws.slug;

    for (method, suffix, _action) in webhook_routes(&id) {
        let status = call_webhook_route(base, slug, &token, method, &suffix).await;
        assert_eq!(
            status, 403,
            "viewer-reach agent must be forbidden on {method} /webhooks{suffix} despite holding the capability"
        );
    }

    db.teardown().await;
}
