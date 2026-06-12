#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{CreateGrantRequest, CreateProjectRequest, GrantPrincipal};

fn project_req(name: &str, slug: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: name.to_string(),
        slug: slug.to_string(),
        task_prefix: "TEN".to_string(),
        visibility: None,
        visibility_role: None,
    }
}

fn grant_req(user_id: uuid::Uuid) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "user".to_string(),
            id: user_id,
        },
        role: "viewer".to_string(),
    }
}

/// Sets up two independent workspaces. Returns (client for workspace A, slug
/// of workspace B) so every test can attempt cross-tenant access.
async fn setup() -> (
    atlas_client::AtlasClient,
    support::TestDb,
    support::TestServer,
    String,
) {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client_a, _ws_a, _user_a) =
        support::login_user_with_workspace(&server, &db, "ten-alice").await;
    let (_client_b, ws_b, _user_b) =
        support::login_user_with_workspace(&server, &db, "ten-bob").await;

    (client_a, db, server, ws_b.slug)
}

#[tokio::test]
async fn cross_tenant_get_workspace_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.get_workspace(&ws_b_slug).await;

    assert!(result.is_err(), "cross-tenant workspace read must fail");
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_list_api_keys_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.list_api_keys(&ws_b_slug, None, None).await;

    assert!(result.is_err(), "cross-tenant api-key list must fail");
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_list_projects_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.list_projects(&ws_b_slug, None, None).await;

    assert!(result.is_err(), "cross-tenant project list must fail");
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_create_project_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a
        .create_project(&ws_b_slug, project_req("Intruder", "intruder-ten"))
        .await;

    assert!(result.is_err(), "cross-tenant project create must fail");
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_get_project_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.get_project(&ws_b_slug, "any-project").await;

    assert!(result.is_err(), "cross-tenant project get must fail");
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_list_workspace_grants_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.list_workspace_grants(&ws_b_slug, None, None).await;

    assert!(
        result.is_err(),
        "cross-tenant workspace grant list must fail"
    );
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_create_workspace_grant_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let dummy_id = uuid::Uuid::now_v7();
    let result = client_a
        .create_workspace_grant(&ws_b_slug, grant_req(dummy_id))
        .await;

    assert!(
        result.is_err(),
        "cross-tenant workspace grant create must fail"
    );
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_list_project_grants_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a
        .list_project_grants(&ws_b_slug, "any-project", None, None)
        .await;

    assert!(result.is_err(), "cross-tenant project grant list must fail");
    db.teardown().await;
}
