#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::CreateUserApiKeyRequest;
use atlas_client::ClientError;
use support::{TestDb, TestServer, login_user_with_workspace};

#[tokio::test]
async fn human_creates_workspace_and_it_appears_in_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _ws, _user) = login_user_with_workspace(&server, &db, "cw-owner").await;

    let created = client
        .create_workspace("My New Space")
        .await
        .expect("create_workspace");

    assert_eq!(created.name, "My New Space");
    assert_eq!(created.slug, "my-new-space");

    let workspaces = client.list_workspaces().await.expect("list_workspaces");

    assert!(
        workspaces.iter().any(|w| w.slug == created.slug),
        "the created workspace slug '{}' must appear in the list",
        created.slug,
    );
}

#[tokio::test]
async fn api_key_principal_cannot_create_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, _ws, _user) = login_user_with_workspace(&server, &db, "cw-agent-owner").await;

    let created_key = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "agent-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create_user_api_key");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(created_key.secret);

    let err = agent
        .create_workspace("Agent Space")
        .await
        .expect_err("api key must not create a workspace");

    match err {
        ClientError::Api(p) => {
            assert_eq!(p.status, 403, "expected 403, got {}", p.status)
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn slug_collision_produces_distinct_slug() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _ws, _user) = login_user_with_workspace(&server, &db, "cw-collision").await;

    let first = client
        .create_workspace("Shared Name")
        .await
        .expect("create first workspace");
    assert_eq!(first.slug, "shared-name");

    let second = client
        .create_workspace("Shared Name")
        .await
        .expect("create second workspace");

    assert_ne!(
        first.slug, second.slug,
        "a name that slugifies to an existing slug must get a distinct slug",
    );
    assert_eq!(second.slug, "shared-name-2");
}

#[tokio::test]
async fn create_workspace_returns_401_for_unauthenticated() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let err = server
        .client()
        .create_workspace("Anon Space")
        .await
        .expect_err("unauthenticated create_workspace must fail");

    match err {
        ClientError::Api(p) => {
            assert_eq!(p.status, 401, "expected 401, got {}", p.status)
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
