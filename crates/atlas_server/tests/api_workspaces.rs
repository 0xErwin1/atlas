#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_client::ClientError;
use support::{TestDb, TestServer, login_user_with_workspace};

#[tokio::test]
async fn list_workspaces_returns_the_users_own_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _user) = login_user_with_workspace(&server, &db, "ws-list-owner").await;

    let workspaces = client.list_workspaces().await.expect("list_workspaces");

    assert!(
        workspaces.iter().any(|w| w.slug == ws.slug),
        "the seeded workspace slug '{}' must appear in the list",
        ws.slug,
    );
}

#[tokio::test]
async fn list_workspaces_does_not_leak_other_tenants_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_client_a, ws_a, _user_a) =
        login_user_with_workspace(&server, &db, "ws-list-tenant-a").await;
    let (client_b, _ws_b, _user_b) =
        login_user_with_workspace(&server, &db, "ws-list-tenant-b").await;

    let workspaces_b = client_b
        .list_workspaces()
        .await
        .expect("list_workspaces for tenant-b");

    assert!(
        !workspaces_b.iter().any(|w| w.slug == ws_a.slug),
        "tenant-b must not see tenant-a's workspace '{}'",
        ws_a.slug,
    );
}

#[tokio::test]
async fn list_workspaces_returns_401_for_unauthenticated() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let err = server
        .client()
        .list_workspaces()
        .await
        .expect_err("unauthenticated list_workspaces must fail");

    match err {
        ClientError::Api(p) => {
            assert_eq!(p.status, 401, "expected 401, got {}", p.status)
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
