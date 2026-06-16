#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{ApiKeyRepo, MembershipRepo, NewApiKey, NewUser, UserRepo};
use support::{TestDb, TestServer, login_user_with_workspace};

async fn add_member(
    db: &TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    username: &str,
    role: MemberRole,
) -> atlas_domain::entities::identity::User {
    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$hash".into(),
            is_root: false,
        })
        .await
        .expect("create user");

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(user.id));
    db.membership_repo()
        .add(&ctx, user.id, role)
        .await
        .expect("add membership");

    user
}

async fn add_agent(
    db: &TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    creator: atlas_domain::ids::UserId,
    name: &str,
) -> atlas_domain::entities::identity::ApiKey {
    let ctx = WorkspaceCtx::new(ws_id, Actor::User(creator));
    db.api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: name.to_string(),
                token_hash: format!("hash-{name}"),
                expires_at: None,
            },
        )
        .await
        .expect("create api key")
}

#[tokio::test]
async fn list_members_returns_users_and_agents() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "members-owner").await;

    let member = add_member(&db, ws.id, "members-second", MemberRole::Member).await;
    let agent = add_agent(&db, ws.id, owner_user.id, "ci-bot").await;

    let members = owner
        .list_workspace_members(&ws.slug)
        .await
        .expect("list members");

    assert!(
        members
            .iter()
            .any(|p| p.principal_type == "user" && p.id == owner_user.id.0),
        "owner user should be listed"
    );
    assert!(
        members
            .iter()
            .any(|p| p.principal_type == "user" && p.id == member.id.0),
        "second member user should be listed"
    );
    assert!(
        members
            .iter()
            .any(|p| p.principal_type == "api_key" && p.id == agent.id.0 && p.display == "ci-bot"),
        "agent api key should be listed with its name as display"
    );
}

#[tokio::test]
async fn list_members_visible_to_plain_member() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _) = login_user_with_workspace(&server, &db, "members-owner2").await;

    let member_user = add_member(&db, ws.id, "members-plain", MemberRole::Member).await;

    // Log in as the plain member (viewer-level should be enough to see the list).
    let member_client = {
        use atlas_api::dtos::LoginRequest;
        use atlas_server::auth::password;

        let hash = password::hash("TestPassword1!".to_string())
            .await
            .expect("hash");
        // Recreate the user with a real password to log in.
        let user = db
            .user_repo()
            .create(NewUser {
                username: "members-plain-login".to_string(),
                display_name: "members-plain-login".to_string(),
                password_hash: hash,
                is_root: false,
            })
            .await
            .expect("create login user");
        let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
        db.membership_repo()
            .add(&ctx, user.id, MemberRole::Member)
            .await
            .expect("add membership");

        let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
        client
            .login(LoginRequest {
                username: "members-plain-login".to_string(),
                password: "TestPassword1!".to_string(),
            })
            .await
            .expect("login");
        client
    };

    let members = member_client
        .list_workspace_members(&ws.slug)
        .await
        .expect("member can list");

    assert!(
        members.iter().any(|p| p.id == member_user.id.0),
        "the workspace member should be visible to another member"
    );
}

#[tokio::test]
async fn list_members_cross_tenant_returns_not_found() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner_a, _ws_a, _) = login_user_with_workspace(&server, &db, "members-tenant-a").await;
    let (outsider, ws_b, _) = login_user_with_workspace(&server, &db, "members-tenant-b").await;

    // outsider is a member of ws_b only; asking for ws_a's members must 404 (conceal).
    let result = outsider.list_workspace_members("ws-members-tenant-a").await;

    assert!(result.is_err(), "outsider must not read another workspace");
    let _ = ws_b;
}
