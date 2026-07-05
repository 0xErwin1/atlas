#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use hmac::{Hmac, Mac};
use sea_orm::TransactionTrait;
use serde_json::Value;
use sha2::Sha256;
use uuid::Uuid;

use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{ApiKeyType, MemberRole},
    ids::WorkspaceId,
    permissions::Capability,
};
use atlas_server::{
    auth::password,
    persistence::repos::{
        ApiKeyRepo, MembershipRepo, NewApiKey, NewUser, PgApiKeyRepo, PgMembershipRepo, PgUserRepo,
        UserRepo,
    },
};

type HmacSha256 = Hmac<Sha256>;

fn compute_sig(secret: &[u8], body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).unwrap();
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256={hex}")
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

/// Creates a Member-role user in `ws_id` with a known password and logs in,
/// returning the bearer token.
async fn add_member_and_login(
    server: &support::TestServer,
    db: &support::TestDb,
    ws_id: Uuid,
    username: &str,
) -> String {
    use atlas_api::dtos::LoginRequest;

    let plaintext = "TestPassword1!";
    let hash = password::hash(plaintext.to_string()).await.expect("hash");

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
            password_hash: Some(hash),
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

    let resp = http()
        .post(format!("{}/v1/auth/login", server.base_url()))
        .json(&LoginRequest {
            username: username.to_string(),
            password: plaintext.to_string(),
        })
        .send()
        .await
        .expect("login");

    let body: Value = resp.json().await.expect("login body");
    body["token"].as_str().expect("token").to_string()
}

// ---------------------------------------------------------------------------
// B4.8 [I] Admin creates integration config → 201 + secret returned once
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_creates_integration_config_returns_201_with_secret() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ic-admin-create").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201, "admin create must return 201");

    let body: Value = resp.json().await.unwrap();
    assert!(body["id"].is_string(), "id must be present");
    assert_eq!(body["integration"], "github");
    assert_eq!(body["is_active"], true);
    assert!(
        body["secret"].is_string(),
        "secret must be in create response"
    );
    assert!(
        body["integration_api_key_id"].is_string(),
        "integration_api_key_id must be present"
    );

    let secret = body["secret"].as_str().unwrap();
    assert!(
        secret.starts_with("integ_"),
        "secret must start with integ_: {secret}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.8 [I] List and get responses do NOT expose the secret
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_and_get_do_not_expose_secret() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) = support::login_user_with_workspace(&server, &db, "ic-nosecret").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let created: Value = create_resp.json().await.unwrap();
    let config_id = created["id"].as_str().unwrap().to_string();

    let list_resp = http()
        .get(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200);
    let list_body: Value = list_resp.json().await.unwrap();
    let items = list_body.as_array().unwrap();
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
        .get(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs/{config_id}"
        ))
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
// B4.8 [I] Non-admin member receives 404 on all integration-config endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_admin_rejected_on_integration_config_endpoints() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (admin_client, ws, _admin_user) =
        support::login_user_with_workspace(&server, &db, "ic-nonadmin-admin").await;

    let admin_token = admin_client.token().expect("admin token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201, "admin setup must succeed");
    let created: Value = create_resp.json().await.unwrap();
    let config_id = created["id"].as_str().unwrap().to_string();

    let member_token = add_member_and_login(&server, &db, ws.id.0, "ic-nonadmin-member").await;

    let list_resp = http()
        .get(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(&member_token)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 404, "non-admin list must be 404");

    let get_resp = http()
        .get(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs/{config_id}"
        ))
        .bearer_auth(&member_token)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), 404, "non-admin get must be 404");

    let post_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(&member_token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    assert_eq!(post_resp.status(), 404, "non-admin create must be 404");

    let delete_resp = http()
        .delete(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs/{config_id}"
        ))
        .bearer_auth(&member_token)
        .send()
        .await
        .unwrap();
    assert_eq!(delete_resp.status(), 404, "non-admin delete must be 404");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.8 [I] Delete soft-deletes the config and revokes the api key
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_delete_soft_deletes_and_revokes_key() {
    use atlas_server::persistence::entities::identity::api_key;
    use sea_orm::EntityTrait;

    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) = support::login_user_with_workspace(&server, &db, "ic-delete").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let created: Value = create_resp.json().await.unwrap();
    let config_id = created["id"].as_str().unwrap();
    let api_key_id: Uuid =
        Uuid::parse_str(created["integration_api_key_id"].as_str().unwrap()).unwrap();

    let delete_resp = http()
        .delete(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs/{config_id}"
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(delete_resp.status(), 204, "delete must return 204");

    let get_resp = http()
        .get(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs/{config_id}"
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), 404, "deleted config must return 404");

    let key_row = api_key::Entity::find_by_id(api_key_id)
        .one(db.conn())
        .await
        .expect("find key")
        .expect("key must still exist");
    assert!(
        key_row.revoked_at.is_some(),
        "integration api key must be revoked on config delete"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// [I] PATCH is_active toggles the config and gates the inbound ingest
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_is_active_toggles_and_gates_ingest() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) = support::login_user_with_workspace(&server, &db, "ic-toggle").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let created: Value = create_resp.json().await.unwrap();
    let config_id = created["id"].as_str().unwrap().to_string();
    let secret = created["secret"].as_str().unwrap().to_string();

    let deactivate = http()
        .patch(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs/{config_id}"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "is_active": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(deactivate.status(), 200, "patch must return 200");
    let patched: Value = deactivate.json().await.unwrap();
    assert_eq!(patched["is_active"], false, "config must be deactivated");
    assert!(patched["secret"].is_null(), "patch must not expose secret");

    // A deactivated config makes the ingest resolve no active config → 404.
    let body = b"{\"action\":\"completed\"}";
    let sig = compute_sig(secret.as_bytes(), body);
    let ingest_off = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", &sig)
        .header("x-github-delivery", Uuid::now_v7().to_string())
        .header("x-github-event", "workflow_run")
        .header("content-type", "application/json")
        .body(body.as_ref())
        .send()
        .await
        .unwrap();
    assert_eq!(
        ingest_off.status(),
        404,
        "ingest must be rejected while the config is inactive"
    );

    let reactivate = http()
        .patch(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs/{config_id}"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "is_active": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(reactivate.status(), 200);
    let reactivated: Value = reactivate.json().await.unwrap();
    assert_eq!(reactivated["is_active"], true, "config must be reactivated");

    let ingest_on = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", &sig)
        .header("x-github-delivery", Uuid::now_v7().to_string())
        .header("x-github-event", "workflow_run")
        .header("content-type", "application/json")
        .body(body.as_ref())
        .send()
        .await
        .unwrap();
    assert_eq!(
        ingest_on.status(),
        200,
        "ingest must succeed again after reactivation"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.7 [I] Ingestion: valid signed event → 200 + outbox row
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_valid_signed_event_returns_200() {
    use atlas_server::persistence::entities::events_outbox::event_outbox;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ic-ingest-valid").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let created: Value = create_resp.json().await.unwrap();
    let secret = created["secret"].as_str().unwrap().to_string();

    let body = serde_json::to_vec(&serde_json::json!({
        "action": "completed",
        "workflow_run": {
            "id": 1,
            "name": "CI",
            "conclusion": "failure",
            "html_url": "https://github.com/actions/runs/1",
            "head_branch": "main",
            "event": "push"
        },
        "repository": { "full_name": "owner/repo" }
    }))
    .unwrap();

    let delivery_id = Uuid::now_v7();
    let sig = compute_sig(secret.as_bytes(), &body);

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", &sig)
        .header("x-github-delivery", delivery_id.to_string())
        .header("x-github-event", "workflow_run")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200, "valid ingest must return 200");

    let rows = event_outbox::Entity::find()
        .filter(event_outbox::Column::Id.eq(delivery_id))
        .all(db.conn())
        .await
        .expect("query outbox");

    assert_eq!(rows.len(), 1, "one outbox row must be written");
    assert_eq!(rows[0].source, "external/github");
    assert_eq!(rows[0].event_type, "external.github.workflow_run");
    assert_eq!(rows[0].aggregate_type, "external");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.7 [I] Ingestion: bad signature → 401
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_bad_sig_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ic-ingest-badsig").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();

    let body = b"{\"action\":\"completed\"}";
    let bad_sig = "sha256=0000000000000000000000000000000000000000000000000000000000000000";

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", bad_sig)
        .header("x-github-delivery", Uuid::now_v7().to_string())
        .header("content-type", "application/json")
        .body(body.as_ref())
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401, "bad sig must return 401");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.7 [I] Ingestion: missing signature header → 401
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_missing_sig_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ic-ingest-nosig").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-github-delivery", Uuid::now_v7().to_string())
        .header("content-type", "application/json")
        .body(b"{\"action\":\"completed\"}".as_ref())
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401, "missing sig must return 401");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.7 [I] Ingestion: no active config for workspace → 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_no_config_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (_client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ic-ingest-nocfg").await;

    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", "sha256=aabbcc")
        .header("x-github-delivery", Uuid::now_v7().to_string())
        .header("content-type", "application/json")
        .body(b"{}".as_ref())
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404, "no config must return 404");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.7 [I] Ingestion: body exceeds 1 MiB → 413
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_oversized_body_returns_413() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ic-ingest-oversize").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let created: Value = create_resp.json().await.unwrap();
    let secret = created["secret"].as_str().unwrap().to_string();

    let big_body = vec![b'x'; 1024 * 1024 + 1];
    let sig = compute_sig(secret.as_bytes(), &big_body);

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", &sig)
        .header("x-github-delivery", Uuid::now_v7().to_string())
        .header("content-type", "application/octet-stream")
        .body(big_body)
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    assert!(
        status == 413 || status == 400,
        "oversized body must return 413 or 400, got {status}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.7 [I] Ingestion: duplicate delivery is a no-op (200, single row)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_duplicate_delivery_is_noop() {
    use atlas_server::persistence::entities::events_outbox::event_outbox;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ic-ingest-dup").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    let created: Value = create_resp.json().await.unwrap();
    let secret = created["secret"].as_str().unwrap().to_string();

    let body = b"{\"action\":\"completed\"}";
    let delivery_id = Uuid::now_v7();
    let sig = compute_sig(secret.as_bytes(), body);

    for _ in 0..2u8 {
        let resp = http()
            .post(format!(
                "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
            ))
            .header("x-hub-signature-256", &sig)
            .header("x-github-delivery", delivery_id.to_string())
            .header("content-type", "application/json")
            .body(body.as_ref())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "both deliveries must return 200");
    }

    let rows = event_outbox::Entity::find()
        .filter(event_outbox::Column::Id.eq(delivery_id))
        .all(db.conn())
        .await
        .expect("query outbox");
    assert_eq!(
        rows.len(),
        1,
        "duplicate delivery must produce exactly one outbox row"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.7 [I] Ingestion: filter match creates a task
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_filter_match_creates_task() {
    use atlas_domain::{
        entities::boards_tasks::{NewBoard, PositionBetween},
        entities::workspace_core::NewProject,
        permissions::{Visibility, VisibilityRole},
    };
    use atlas_server::persistence::entities::boards_tasks::task;
    use atlas_server::persistence::repos::{
        BoardRepo, PgAutomationRuleRepo, PgProjectRepo, ProjectRepo,
    };
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "ic-ingest-match").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let ctx = support::ctx(&ws, &user);

    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewProject {
            name: "Ingest Match Project".to_string(),
            slug: "ingest-match-proj".to_string(),
            task_prefix: "IMP".to_string(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("create project");

    let board_repo = db.board_repo();
    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                name: "Test Board".to_string(),
                project_id: project.id,
            },
        )
        .await
        .expect("create board");

    let column = board_repo
        .add_column(
            &ctx,
            board.id,
            "To Do".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    let created: Value = create_resp.json().await.unwrap();
    let secret = created["secret"].as_str().unwrap().to_string();

    let txn = db.conn().begin().await.expect("begin");
    PgAutomationRuleRepo::create(
        &txn,
        ws.id.0,
        "CI failure → task".to_string(),
        "external.github.workflow_run".to_string(),
        Some(serde_json::json!({"conclusion": "failure"})),
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
    txn.commit().await.expect("commit");

    let body = serde_json::to_vec(&serde_json::json!({
        "action": "completed",
        "workflow_run": {
            "id": 42,
            "name": "CI Pipeline",
            "conclusion": "failure",
            "html_url": "https://github.com/runs/42",
            "head_branch": "main",
            "event": "push"
        },
        "repository": { "full_name": "owner/repo" }
    }))
    .unwrap();
    let delivery_id = Uuid::now_v7();
    let sig = compute_sig(secret.as_bytes(), &body);

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", &sig)
        .header("x-github-delivery", delivery_id.to_string())
        .header("x-github-event", "workflow_run")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "ingest must return 200");

    let tasks = task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ws.id.0))
        .filter(task::Column::BoardId.eq(board.id.0))
        .all(db.conn())
        .await
        .expect("query tasks");
    assert_eq!(tasks.len(), 1, "one task must be created by the rule");
    assert_eq!(tasks[0].title, "CI failed: CI Pipeline");
    assert!(
        tasks[0].created_by_api_key_id.is_some(),
        "task must be created by integration api key"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.7 [I] Ingestion: filter non-match does NOT create a task
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_filter_no_match_no_task() {
    use atlas_domain::{
        entities::boards_tasks::{NewBoard, PositionBetween},
        entities::workspace_core::NewProject,
        permissions::{Visibility, VisibilityRole},
    };
    use atlas_server::persistence::entities::boards_tasks::task;
    use atlas_server::persistence::repos::{
        BoardRepo, PgAutomationRuleRepo, PgProjectRepo, ProjectRepo,
    };
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "ic-ingest-nomatch").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let ctx = support::ctx(&ws, &user);

    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewProject {
            name: "Ingest Nomatch Project".to_string(),
            slug: "ingest-nomatch-proj".to_string(),
            task_prefix: "INP".to_string(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("create project");

    let board_repo = db.board_repo();
    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                name: "Test Board".to_string(),
                project_id: project.id,
            },
        )
        .await
        .expect("create board");

    let column = board_repo
        .add_column(
            &ctx,
            board.id,
            "To Do".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integration-configs"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({ "integration": "github" }))
        .send()
        .await
        .unwrap();
    let created: Value = create_resp.json().await.unwrap();
    let secret = created["secret"].as_str().unwrap().to_string();

    let txn = db.conn().begin().await.expect("begin");
    PgAutomationRuleRepo::create(
        &txn,
        ws.id.0,
        "CI failure only".to_string(),
        "external.github.workflow_run".to_string(),
        Some(serde_json::json!({"conclusion": "failure"})),
        None,
        "create_task".to_string(),
        serde_json::json!({
            "board_id": board.id.0,
            "column_id": column.id.0,
            "title_template": "CI failed"
        }),
        user.id.0,
    )
    .await
    .expect("create rule");
    txn.commit().await.expect("commit");

    let body = serde_json::to_vec(&serde_json::json!({
        "action": "completed",
        "workflow_run": {
            "id": 99,
            "name": "CI",
            "conclusion": "success",
            "html_url": "https://github.com/runs/99",
            "head_branch": "main",
            "event": "push"
        },
        "repository": { "full_name": "owner/repo" }
    }))
    .unwrap();
    let sig = compute_sig(secret.as_bytes(), &body);

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/integrations/github/events"
        ))
        .header("x-hub-signature-256", &sig)
        .header("x-github-delivery", Uuid::now_v7().to_string())
        .header("x-github-event", "workflow_run")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "ingest must return 200");

    let tasks = task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ws.id.0))
        .filter(task::Column::BoardId.eq(board.id.0))
        .all(db.conn())
        .await
        .expect("query tasks");
    assert!(tasks.is_empty(), "filter non-match must produce no task");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Regression: integration configs stay agent-blocked regardless of scope
// ---------------------------------------------------------------------------

/// Integration configs are an `AdminMin`-gated, agent-unreachable surface: an
/// agent is capped at `Editor` regardless of its creator's role, so it can
/// never clear the `AdminMin` threshold. Granting it every capability in the
/// catalog must not change that.
#[tokio::test]
async fn agent_with_all_capabilities_cannot_list_integration_configs() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "ic-agent-allcap-owner").await;

    let plain = "atlas_ic_allcap_agent_secret";
    let hash = atlas_server::auth::tokens::hash_token(plain);

    let key = db
        .api_key_repo()
        .create_for_user(
            owner_user.id,
            NewApiKey {
                name: "ic-allcap-agent".to_string(),
                token_hash: hash,
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes: Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create all-capability agent key");

    PgApiKeyRepo::set_global_for_user_in(db.conn(), owner_user.id, key.id, true)
        .await
        .expect("make key global");

    let resp = http()
        .get(format!(
            "{}/v1/workspaces/{}/integration-configs",
            server.base_url(),
            ws.slug
        ))
        .bearer_auth(plain)
        .send()
        .await
        .expect("request");

    assert_eq!(
        resp.status(),
        403,
        "an agent holding all 20 capabilities must still be blocked from integration configs by the AdminMin role ceiling"
    );

    db.teardown().await;
}
