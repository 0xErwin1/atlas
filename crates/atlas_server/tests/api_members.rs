#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_client::ClientError;
use atlas_domain::{
    Actor, WorkspaceCtx, entities::identity::MemberRole, entities::permissions::NewPermissionGrant,
    permissions::ResourceRole,
};
use atlas_server::persistence::repos::{
    ApiKeyRepo, MembershipRepo, NewApiKey, NewUser, PermissionGrantRepo, PgPermissionGrantRepo,
    UserRepo,
};
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
            email: None,
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$hash".into(),
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
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
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

    // Keys now appear in the members list only when they hold a workspace grant.
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(agent.id),
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("grant agent workspace access");

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
                email: None,
                password_hash: hash,
                is_root: false,
                is_system_admin: false,
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
async fn list_members_returns_role_for_user_members_and_no_role_for_api_key_principals() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) =
        login_user_with_workspace(&server, &db, "members-role-owner").await;

    let member = add_member(&db, ws.id, "members-role-member", MemberRole::Admin).await;

    let agent = add_agent(&db, ws.id, owner_user.id, "role-ci-bot").await;
    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_domain::permissions::ResourceRole;
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(agent.id),
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("grant agent workspace access");

    let members = owner_client
        .list_workspace_members(&ws.slug)
        .await
        .expect("list members");

    let owner_entry = members
        .iter()
        .find(|p| p.principal_type == "user" && p.id == owner_user.id.0)
        .expect("owner entry must be present");
    assert_eq!(
        owner_entry.role.as_deref(),
        Some("owner"),
        "owner member must have role='owner'"
    );

    let member_entry = members
        .iter()
        .find(|p| p.principal_type == "user" && p.id == member.id.0)
        .expect("admin member entry must be present");
    assert_eq!(
        member_entry.role.as_deref(),
        Some("admin"),
        "admin member must have role='admin'"
    );

    let agent_entry = members
        .iter()
        .find(|p| p.principal_type == "api_key" && p.id == agent.id.0)
        .expect("api_key entry must be present");
    assert!(
        agent_entry.role.is_none(),
        "api_key principal must have no role field, got: {:?}",
        agent_entry.role
    );
}

#[tokio::test]
async fn list_members_cross_tenant_returns_not_found() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner_a, _ws_a, _) = login_user_with_workspace(&server, &db, "members-tenant-a").await;
    let (outsider, ws_b, _) = login_user_with_workspace(&server, &db, "members-tenant-b").await;

    // outsider is a member of ws_b only; asking for ws_a's members must 404 (conceal).
    let err = outsider
        .list_workspace_members("ws-members-tenant-a")
        .await
        .expect_err("outsider must not read another workspace");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 404, "expected 404, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    let _ = ws_b;
}
