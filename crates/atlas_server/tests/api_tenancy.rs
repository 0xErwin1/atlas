#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{CreateGrantRequest, CreateProjectRequest, GrantPrincipal};
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, UserRepo};

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

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant workspace read must return 404, got: {result:?}"
    );
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_list_api_keys_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.list_api_keys(&ws_b_slug, None, None).await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant api-key list must return 404, got: {result:?}"
    );
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_list_projects_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.list_projects(&ws_b_slug, None, None).await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant project list must return 404, got: {result:?}"
    );
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_create_project_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a
        .create_project(&ws_b_slug, project_req("Intruder", "intruder-ten"))
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant project create must return 404, got: {result:?}"
    );
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_get_project_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.get_project(&ws_b_slug, "any-project").await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant project get must return 404, got: {result:?}"
    );
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_list_workspace_grants_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a.list_workspace_grants(&ws_b_slug, None, None).await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant workspace grant list must return 404, got: {result:?}"
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
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant workspace grant create must return 404, got: {result:?}"
    );
    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_list_project_grants_returns_404() {
    let (client_a, db, _server, ws_b_slug) = setup().await;

    let result = client_a
        .list_project_grants(&ws_b_slug, "any-project", None, None)
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant project grant list must return 404, got: {result:?}"
    );
    db.teardown().await;
}

/// A user who is a member of BOTH workspace A and workspace B must not be able to
/// access workspace B's projects when scoped to workspace A. This exercises the
/// query-level `workspace_id` filter, not just the membership short-circuit.
#[tokio::test]
async fn member_of_both_workspaces_cannot_cross_scope_projects() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner_a, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "ten-dual-owner-a").await;
    let (_owner_b, ws_b, _) =
        support::login_user_with_workspace(&server, &db, "ten-dual-owner-b").await;

    let project_b_slug = "dual-b-only";
    _owner_b
        .create_project(&ws_b.slug, project_req("B-Only Project", project_b_slug))
        .await
        .expect("create project in ws_b");

    let password_plaintext = "TestPassword1!";
    let hash = atlas_server::auth::password::hash(password_plaintext.to_string())
        .await
        .expect("hash");
    let dual_user = db
        .user_repo()
        .create(NewUser {
            username: "ten-dual-member".to_string(),
            display_name: "Dual Member".to_string(),
            email: None,
            password_hash: hash,
            is_root: false,
        })
        .await
        .expect("create dual user");

    let ctx_a = WorkspaceCtx::new(ws_a.id, Actor::User(dual_user.id));
    db.membership_repo()
        .add(&ctx_a, dual_user.id, MemberRole::Member)
        .await
        .expect("add to ws_a");

    let ctx_b = WorkspaceCtx::new(ws_b.id, Actor::User(dual_user.id));
    db.membership_repo()
        .add(&ctx_b, dual_user.id, MemberRole::Member)
        .await
        .expect("add to ws_b");

    let mut dual_client = atlas_client::AtlasClient::new(server.base_url().to_string());
    dual_client
        .login(atlas_api::dtos::LoginRequest {
            username: "ten-dual-member".to_string(),
            password: password_plaintext.to_string(),
        })
        .await
        .expect("dual login");

    let result = dual_client.get_project(&ws_a.slug, project_b_slug).await;

    assert!(
        result.is_err(),
        "dual-member must not see workspace B's project when scoped to workspace A"
    );
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "must return 404 (not 200), got: {result:?}"
    );

    db.teardown().await;
}
