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
use support::{TestDb, TestServer, login_user_with_workspace};

async fn add_user_to_workspace(
    db: &TestDb,
    server: &TestServer,
    ws_id: atlas_domain::ids::WorkspaceId,
    username: &str,
    role: MemberRole,
) -> (
    atlas_client::AtlasClient,
    atlas_domain::entities::identity::User,
) {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
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
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    (client, user)
}

async fn create_non_member_user(
    db: &TestDb,
    server: &TestServer,
    username: &str,
) -> (
    atlas_client::AtlasClient,
    atlas_domain::entities::identity::User,
) {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
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

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    (client, user)
}

fn grant_req(user_id: uuid::Uuid, role: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "user".to_string(),
            id: user_id,
        },
        role: role.to_string(),
    }
}

fn proj_req(name: &str, slug: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: name.to_string(),
        slug: slug.to_string(),
        task_prefix: "GRN".to_string(),
        visibility: None,
        visibility_role: None,
    }
}

#[tokio::test]
async fn create_project_grant_allows_sharing() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "grant-owner").await;

    owner
        .create_project(&ws.slug, proj_req("Grant Project", "grant-proj"))
        .await
        .expect("create project");

    // Add a second workspace member to be granted access.
    let (_, grantee) =
        add_user_to_workspace(&db, &server, ws.id, "grantee-user", MemberRole::Member).await;

    let grant = owner
        .create_project_grant(&ws.slug, "grant-proj", grant_req(grantee.id.0, "viewer"))
        .await
        .expect("create project grant");

    assert_eq!(grant.principal.id, grantee.id.0);
    assert_eq!(grant.role, "viewer");
    let _ = owner_user;
}

#[tokio::test]
async fn list_project_grants_returns_created_grant() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "grant-owner2").await;

    owner
        .create_project(&ws.slug, proj_req("List Grant Project", "list-grant-proj"))
        .await
        .expect("create project");

    let (_, grantee) =
        add_user_to_workspace(&db, &server, ws.id, "grantee-user2", MemberRole::Member).await;

    owner
        .create_project_grant(
            &ws.slug,
            "list-grant-proj",
            grant_req(grantee.id.0, "editor"),
        )
        .await
        .expect("create grant");

    let page = owner
        .list_project_grants(&ws.slug, "list-grant-proj", None, None)
        .await
        .expect("list grants");

    assert!(
        page.items.iter().any(|g| g.principal.id == grantee.id.0),
        "grant should appear in list"
    );
}

#[tokio::test]
async fn delete_project_grant_removes_it() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "grant-owner3").await;

    owner
        .create_project(&ws.slug, proj_req("Del Grant Project", "del-grant-proj"))
        .await
        .expect("create project");

    let (_, grantee) =
        add_user_to_workspace(&db, &server, ws.id, "grantee-user3", MemberRole::Member).await;

    let grant = owner
        .create_project_grant(
            &ws.slug,
            "del-grant-proj",
            grant_req(grantee.id.0, "viewer"),
        )
        .await
        .expect("create grant");

    owner
        .delete_project_grant(&ws.slug, "del-grant-proj", grant.id)
        .await
        .expect("delete grant");

    let page = owner
        .list_project_grants(&ws.slug, "del-grant-proj", None, None)
        .await
        .expect("list after delete");

    assert!(
        !page.items.iter().any(|g| g.id == grant.id),
        "deleted grant should not appear"
    );
}

#[tokio::test]
async fn create_workspace_grant_allows_sharing() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "ws-grant-owner").await;

    let (_, non_member) = create_non_member_user(&db, &server, "non-member-ws").await;

    // First add the non-member as a workspace member (required by parse_principal).
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(non_member.id));
    db.membership_repo()
        .add(&ctx, non_member.id, MemberRole::Member)
        .await
        .expect("add membership");

    let grant = owner
        .create_workspace_grant(&ws.slug, grant_req(non_member.id.0, "editor"))
        .await
        .expect("create workspace grant");

    assert_eq!(grant.principal.id, non_member.id.0);
    assert_eq!(grant.role, "editor");
}

#[tokio::test]
async fn list_and_delete_workspace_grant() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "ws-grant-owner2").await;

    let (_, member) =
        add_user_to_workspace(&db, &server, ws.id, "ws-grantee", MemberRole::Member).await;

    let grant = owner
        .create_workspace_grant(&ws.slug, grant_req(member.id.0, "viewer"))
        .await
        .expect("create workspace grant");

    let page = owner
        .list_workspace_grants(&ws.slug, None, None)
        .await
        .expect("list workspace grants");

    assert!(
        page.items.iter().any(|g| g.id == grant.id),
        "grant should appear in list"
    );

    owner
        .delete_workspace_grant(&ws.slug, grant.id)
        .await
        .expect("delete workspace grant");

    let page_after = owner
        .list_workspace_grants(&ws.slug, None, None)
        .await
        .expect("list after delete");

    assert!(
        !page_after.items.iter().any(|g| g.id == grant.id),
        "deleted grant should not appear"
    );
}
