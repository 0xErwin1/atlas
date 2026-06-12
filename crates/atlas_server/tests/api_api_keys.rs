#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::CreateApiKeyRequest;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, UserRepo};
use support::{TestDb, TestServer, login_user_with_workspace};

async fn add_member_with_role(
    db: &TestDb,
    server: &TestServer,
    ws_id: atlas_domain::ids::WorkspaceId,
    username: &str,
    role: MemberRole,
) -> atlas_client::AtlasClient {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let password_plaintext = "TestPassword1!";
    let hash = password::hash(password_plaintext.to_string())
        .await
        .expect("hash");

    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            password_hash: hash,
            is_root: false,
        })
        .await
        .expect("create user");

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(user.id));
    db.membership_repo()
        .add(&ctx, user.id, role)
        .await
        .expect("add membership");

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: password_plaintext.to_string(),
        })
        .await
        .expect("login");
    client
}

fn key_req(name: &str) -> CreateApiKeyRequest {
    CreateApiKeyRequest {
        name: name.to_string(),
        expires_at: None,
    }
}

#[tokio::test]
async fn create_api_key_requires_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "owner-ak").await;

    // Workspace member (non-admin) cannot create an API key.
    let viewer = add_member_with_role(&db, &server, ws.id, "viewer-ak", MemberRole::Member).await;

    let err = viewer.create_api_key(&ws.slug, key_req("viewer-key")).await;
    assert!(err.is_err(), "viewer should not be able to create api key");

    // Owner (admin-level) succeeds.
    let result = owner.create_api_key(&ws.slug, key_req("owner-key")).await;
    assert!(result.is_ok(), "owner should be able to create api key");
}

#[tokio::test]
async fn create_api_key_returns_secret_once() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "owner-ak2").await;

    let created = owner
        .create_api_key(&ws.slug, key_req("my-key"))
        .await
        .expect("create api key");

    assert!(
        created.secret.starts_with("atlas_"),
        "secret must have atlas_ prefix, got: {}",
        created.secret
    );
    assert_eq!(created.name, "my-key");
    assert!(created.id != uuid::Uuid::nil());
}

#[tokio::test]
async fn list_api_keys_returns_created_key() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "owner-ak3").await;

    owner
        .create_api_key(&ws.slug, key_req("listed-key"))
        .await
        .expect("create api key");

    let page = owner
        .list_api_keys(&ws.slug, None, None)
        .await
        .expect("list api keys");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].name, "listed-key");
    assert!(page.items[0].revoked_at.is_none());
}

#[tokio::test]
async fn revoke_api_key_removes_it_from_active_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "owner-ak4").await;

    let created = owner
        .create_api_key(&ws.slug, key_req("to-revoke"))
        .await
        .expect("create api key");

    let page_before = owner
        .list_api_keys(&ws.slug, None, None)
        .await
        .expect("list before revoke");
    assert!(
        page_before.items.iter().any(|k| k.id == created.id),
        "key should be in active list before revoke"
    );

    owner
        .revoke_api_key(&ws.slug, created.id)
        .await
        .expect("revoke api key");

    let page_after = owner
        .list_api_keys(&ws.slug, None, None)
        .await
        .expect("list after revoke");

    assert!(
        !page_after.items.iter().any(|k| k.id == created.id),
        "revoked key should not appear in active list"
    );
}

#[tokio::test]
async fn create_api_key_rejects_non_member() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_, ws, _) = login_user_with_workspace(&server, &db, "owner-ak5").await;
    let (stranger, _, _) = login_user_with_workspace(&server, &db, "stranger-ak5").await;

    let err = stranger
        .create_api_key(&ws.slug, key_req("should-fail"))
        .await;

    assert!(err.is_err(), "non-member should be rejected");
}
