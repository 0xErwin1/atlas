#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{CreateUserApiKeyRequest, InitialGrantRequest};
use support::{TestDb, TestServer, login_user, login_user_with_workspace};

fn user_key_req(name: &str) -> CreateUserApiKeyRequest {
    CreateUserApiKeyRequest {
        name: name.to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: None,
    }
}

// ---------------------------------------------------------------------------
// POST /v1/api-keys
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_user_api_key_returns_secret_and_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "cuk-user1").await;

    let created = user
        .create_user_api_key(user_key_req("my-agent-key"))
        .await
        .expect("create user api key");

    assert!(
        created.secret.starts_with("atlas_"),
        "secret must have atlas_ prefix"
    );
    assert_eq!(created.name, "my-agent-key");
    assert_eq!(created.r#type, "agent", "default type must be agent");
    assert_ne!(created.id, uuid::Uuid::nil());

    db.teardown().await;
}

#[tokio::test]
async fn create_user_api_key_respects_explicit_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "cuk-user2").await;

    let req = CreateUserApiKeyRequest {
        name: "cli-key".to_string(),
        r#type: Some("cli".to_string()),
        expires_at: None,
        initial_grant: None,
    };

    let created = user
        .create_user_api_key(req)
        .await
        .expect("create cli api key");

    assert_eq!(created.r#type, "cli");

    db.teardown().await;
}

#[tokio::test]
async fn create_user_api_key_rejects_invalid_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "cuk-user3").await;

    let req = CreateUserApiKeyRequest {
        name: "bad-type-key".to_string(),
        r#type: Some("superuser".to_string()),
        expires_at: None,
        initial_grant: None,
    };

    let err = user.create_user_api_key(req).await;

    assert!(err.is_err(), "invalid type must be rejected");

    db.teardown().await;
}

#[tokio::test]
async fn api_key_principal_cannot_create_user_api_key() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "cuk-user4").await;

    let ws_key = owner
        .create_api_key(
            &ws.slug,
            atlas_api::dtos::CreateApiKeyRequest {
                name: "agent-key".to_string(),
                expires_at: None,
            },
        )
        .await
        .expect("create workspace key");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(ws_key.secret);

    let err = agent.create_user_api_key(user_key_req("forbidden")).await;

    assert!(err.is_err(), "api key principal must get 403");

    db.teardown().await;
}

#[tokio::test]
async fn create_user_api_key_with_initial_grant() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "cuk-user5").await;

    let req = CreateUserApiKeyRequest {
        name: "granted-key".to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: Some(InitialGrantRequest {
            workspace: ws.slug.clone(),
            role: "editor".to_string(),
        }),
    };

    let created = owner
        .create_user_api_key(req)
        .await
        .expect("create key with initial grant");

    assert_eq!(created.name, "granted-key");

    let members = owner
        .list_workspace_members(&ws.slug)
        .await
        .expect("list members");

    let key_member = members
        .iter()
        .find(|m| m.principal_type == "api_key" && m.id == created.id);

    assert!(
        key_member.is_some(),
        "key should appear as a workspace member after grant"
    );

    db.teardown().await;
}

#[tokio::test]
async fn initial_grant_rejects_admin_role() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "cuk-user6").await;

    let req = CreateUserApiKeyRequest {
        name: "admin-grant-key".to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: Some(InitialGrantRequest {
            workspace: ws.slug.clone(),
            role: "admin".to_string(),
        }),
    };

    let err = owner.create_user_api_key(req).await;

    assert!(
        err.is_err(),
        "admin role in initial_grant must be rejected for api keys"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// GET /v1/api-keys
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_user_api_keys_returns_own_keys_only() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (alice, _) = login_user(&server, &db, "list-alice").await;
    let (bob, _) = login_user(&server, &db, "list-bob").await;

    alice
        .create_user_api_key(user_key_req("alice-key"))
        .await
        .expect("alice creates key");

    bob.create_user_api_key(user_key_req("bob-key"))
        .await
        .expect("bob creates key");

    let alice_page = alice
        .list_user_api_keys(None, None)
        .await
        .expect("alice lists her keys");

    assert_eq!(alice_page.items.len(), 1, "alice sees exactly 1 key");
    assert_eq!(alice_page.items[0].name, "alice-key");

    let names: Vec<&str> = alice_page.items.iter().map(|k| k.name.as_str()).collect();
    assert!(!names.contains(&"bob-key"), "alice must not see bob's key");

    db.teardown().await;
}

#[tokio::test]
async fn list_user_api_keys_includes_type_field() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "list-type").await;

    user.create_user_api_key(CreateUserApiKeyRequest {
        name: "typed-key".to_string(),
        r#type: Some("bot".to_string()),
        expires_at: None,
        initial_grant: None,
    })
    .await
    .expect("create typed key");

    let page = user
        .list_user_api_keys(None, None)
        .await
        .expect("list keys");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].r#type, "bot");

    db.teardown().await;
}

#[tokio::test]
async fn api_key_principal_cannot_list_user_api_keys() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "list-agent").await;

    let ws_key = owner
        .create_api_key(
            &ws.slug,
            atlas_api::dtos::CreateApiKeyRequest {
                name: "list-agent-key".to_string(),
                expires_at: None,
            },
        )
        .await
        .expect("create workspace key");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(ws_key.secret);

    let err = agent.list_user_api_keys(None, None).await;

    assert!(err.is_err(), "api key principal must get 403");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// DELETE /v1/api-keys/{id}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_user_api_key_removes_from_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "revoke-user1").await;

    let created = user
        .create_user_api_key(user_key_req("to-revoke"))
        .await
        .expect("create key");

    let page_before = user
        .list_user_api_keys(None, None)
        .await
        .expect("list before revoke");

    assert!(
        page_before.items.iter().any(|k| k.id == created.id),
        "key must appear before revoke"
    );

    user.revoke_user_api_key(created.id)
        .await
        .expect("revoke key");

    let page_after = user
        .list_user_api_keys(None, None)
        .await
        .expect("list after revoke");

    assert!(
        !page_after.items.iter().any(|k| k.id == created.id),
        "revoked key must not appear after revoke"
    );

    db.teardown().await;
}

#[tokio::test]
async fn revoke_user_api_key_rejects_other_users_key() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (alice, _) = login_user(&server, &db, "revoke-alice").await;
    let (bob, _) = login_user(&server, &db, "revoke-bob").await;

    let alice_key = alice
        .create_user_api_key(user_key_req("alice-secret"))
        .await
        .expect("alice creates key");

    let err = bob.revoke_user_api_key(alice_key.id).await;

    assert!(err.is_err(), "bob must not be able to revoke alice's key");

    let page = alice
        .list_user_api_keys(None, None)
        .await
        .expect("list alice's keys");

    assert!(
        page.items.iter().any(|k| k.id == alice_key.id),
        "alice's key must still exist after bob's failed revoke"
    );

    db.teardown().await;
}

#[tokio::test]
async fn revoke_user_api_key_nonexistent_returns_error() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "revoke-notfound").await;

    let err = user.revoke_user_api_key(uuid::Uuid::now_v7()).await;

    assert!(err.is_err(), "revoking a nonexistent key must return error");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Attribution: key_type in member/principal DTOs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_key_member_dto_includes_key_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "attr-user1").await;

    let req = CreateUserApiKeyRequest {
        name: "attr-bot".to_string(),
        r#type: Some("bot".to_string()),
        expires_at: None,
        initial_grant: Some(InitialGrantRequest {
            workspace: ws.slug.clone(),
            role: "editor".to_string(),
        }),
    };

    let key_created = owner
        .create_user_api_key(req)
        .await
        .expect("create key with grant");

    let members = owner
        .list_workspace_members(&ws.slug)
        .await
        .expect("list members");

    let key_member = members
        .iter()
        .find(|m| m.principal_type == "api_key" && m.id == key_created.id)
        .expect("api key must appear as workspace member after grant");

    assert_eq!(
        key_member.key_type.as_deref(),
        Some("bot"),
        "key_type must be 'bot' in member DTO"
    );

    db.teardown().await;
}
