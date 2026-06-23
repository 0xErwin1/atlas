#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::CreateApiKeyRequest;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::{
    auth::tokens::hash_token,
    persistence::repos::{ApiKeyRepo, MembershipRepo, NewUser, UserRepo},
};
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
            email: None,
            password_hash: hash,
            is_root: false,
            is_system_admin: false,
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

#[tokio::test]
async fn expired_api_key_is_rejected() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "owner-ak-exp").await;

    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    let created = owner
        .create_api_key(
            &ws.slug,
            CreateApiKeyRequest {
                name: "expired-key".to_string(),
                expires_at: Some(past),
            },
        )
        .await
        .expect("create api key with past expiry");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(created.secret);
    let result = agent.me().await;
    assert!(
        result.is_err(),
        "a key with a past expires_at must be rejected"
    );

    db.teardown().await;
}

#[tokio::test]
async fn unexpired_api_key_is_accepted() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "owner-ak-unexp").await;

    let future = chrono::Utc::now() + chrono::Duration::hours(24);
    let created = owner
        .create_api_key(
            &ws.slug,
            CreateApiKeyRequest {
                name: "future-key".to_string(),
                expires_at: Some(future),
            },
        )
        .await
        .expect("create api key with future expiry");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(created.secret);
    let result = agent.me().await;
    assert!(result.is_ok(), "a key with a future expires_at must work");

    db.teardown().await;
}

#[tokio::test]
async fn no_expiry_api_key_never_expires() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "owner-ak-noexp").await;

    let created = owner
        .create_api_key(&ws.slug, key_req("no-expiry-key"))
        .await
        .expect("create api key without expiry");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(created.secret);
    let result = agent.me().await;
    assert!(result.is_ok(), "a key with no expiry must always work");

    db.teardown().await;
}

#[tokio::test]
async fn disabled_creator_blocks_api_key() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "owner-ak-disable").await;

    let created = owner
        .create_api_key(&ws.slug, key_req("disable-test-key"))
        .await
        .expect("create api key");

    let agent =
        atlas_client::AtlasClient::new(server.base_url()).with_token(created.secret.clone());

    let me_before = agent.me().await;
    assert!(
        me_before.is_ok(),
        "key must work before creator is disabled"
    );

    db.user_repo()
        .disable(owner_user.id)
        .await
        .expect("disable owner");

    let me_after = agent.me().await;
    assert!(
        me_after.is_err(),
        "key must be rejected after creator is disabled"
    );

    db.user_repo()
        .enable(owner_user.id)
        .await
        .expect("re-enable owner");

    let me_reenabled = agent.me().await;
    assert!(
        me_reenabled.is_ok(),
        "key must work again after creator is re-enabled"
    );

    db.teardown().await;
}

#[tokio::test]
async fn api_key_last_used_at_is_set_after_authenticated_request() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "owner-ak-luat").await;

    let created = owner
        .create_api_key(&ws.slug, key_req("luat-key"))
        .await
        .expect("create api key");

    let token_hash = hash_token(&created.secret);
    let api_key_before = db
        .api_key_repo()
        .find_active_by_token_hash(&token_hash)
        .await
        .expect("find key before request")
        .expect("key must exist");

    assert!(
        api_key_before.last_used_at.is_none(),
        "last_used_at must be NULL before first use, got: {:?}",
        api_key_before.last_used_at
    );

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(created.secret);
    agent
        .me()
        .await
        .expect("authenticated request with api key");

    let api_key_after = db
        .api_key_repo()
        .find_active_by_token_hash(&token_hash)
        .await
        .expect("find key after request")
        .expect("key must exist");

    assert!(
        api_key_after.last_used_at.is_some(),
        "last_used_at must be set after an authenticated api-key request"
    );

    db.teardown().await;
}
