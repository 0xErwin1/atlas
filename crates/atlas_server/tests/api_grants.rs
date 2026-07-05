#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateGrantRequest, CreateProjectRequest, CreateUserApiKeyRequest, GrantPrincipal,
};
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{ApiKeyRepo, MembershipRepo, NewApiKey, NewUser, UserRepo};
use support::{TestDb, TestServer, login_user, login_user_with_workspace};

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
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key")
}

fn agent_grant_req(api_key_id: uuid::Uuid, role: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "api_key".to_string(),
            id: api_key_id,
        },
        role: role.to_string(),
    }
}

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
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    support::activate_user_in_db(db, user.id.0).await;

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
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    support::activate_user_in_db(db, user.id.0).await;

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

#[tokio::test]
async fn create_project_grant_admin_to_agent_is_rejected() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "grant-agent-admin-proj").await;

    owner
        .create_project(&ws.slug, proj_req("Agent Admin Proj", "agent-admin-proj"))
        .await
        .expect("create project");

    let agent = add_agent(&db, ws.id, owner_user.id, "admin-bot-proj").await;

    let err = owner
        .create_project_grant(
            &ws.slug,
            "agent-admin-proj",
            agent_grant_req(agent.id.0, "admin"),
        )
        .await
        .expect_err("admin grant to an agent must be rejected");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 403, "expected 403, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    let page = owner
        .list_project_grants(&ws.slug, "agent-admin-proj", None, None)
        .await
        .expect("list grants");

    assert!(
        !page.items.iter().any(|g| g.principal.id == agent.id.0),
        "rejected admin grant must not be persisted"
    );
}

#[tokio::test]
async fn create_workspace_grant_admin_to_agent_is_rejected() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "grant-agent-admin-ws").await;

    let agent = add_agent(&db, ws.id, owner_user.id, "admin-bot-ws").await;

    let err = owner
        .create_workspace_grant(&ws.slug, agent_grant_req(agent.id.0, "admin"))
        .await
        .expect_err("admin grant to an agent must be rejected");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 403, "expected 403, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    let page = owner
        .list_workspace_grants(&ws.slug, None, None)
        .await
        .expect("list grants");

    assert!(
        !page.items.iter().any(|g| g.principal.id == agent.id.0),
        "rejected admin grant must not be persisted"
    );
}

#[tokio::test]
async fn create_workspace_grant_editor_to_agent_succeeds() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "grant-agent-editor-ws").await;

    let agent = add_agent(&db, ws.id, owner_user.id, "editor-bot-ws").await;

    let grant = owner
        .create_workspace_grant(&ws.slug, agent_grant_req(agent.id.0, "editor"))
        .await
        .expect("editor grant to an agent must succeed");

    assert_eq!(grant.principal.id, agent.id.0);
    assert_eq!(grant.role, "editor");

    let page = owner
        .list_workspace_grants(&ws.slug, None, None)
        .await
        .expect("list grants");

    assert!(
        page.items
            .iter()
            .any(|g| g.principal.id == agent.id.0 && g.role == "editor"),
        "editor grant to an agent must be persisted"
    );
}

// ---------------------------------------------------------------------------
// C2 workspace-independent (top-level) key grant tests
// ---------------------------------------------------------------------------

fn toplevel_key_req(name: &str) -> CreateUserApiKeyRequest {
    CreateUserApiKeyRequest {
        name: name.to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: None,
    }
}

#[tokio::test]
async fn grant_toplevel_api_key_to_workspace_succeeds() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "tl-key-ws-owner").await;

    let created = owner
        .create_user_api_key(toplevel_key_req("tl-agent-ws"))
        .await
        .expect("create top-level api key");

    let grant = owner
        .create_workspace_grant(&ws.slug, agent_grant_req(created.id, "editor"))
        .await
        .expect("granting a top-level key to its owner's workspace must succeed");

    assert_eq!(grant.principal.id, created.id);
    assert_eq!(grant.role, "editor");

    let page = owner
        .list_workspace_grants(&ws.slug, None, None)
        .await
        .expect("list workspace grants");

    assert!(
        page.items
            .iter()
            .any(|g| g.principal.id == created.id && g.role == "editor"),
        "grant must appear in workspace grant list"
    );
}

#[tokio::test]
async fn grant_toplevel_api_key_to_project_succeeds() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "tl-key-proj-owner").await;

    owner
        .create_project(&ws.slug, proj_req("TL Key Project", "tl-key-proj"))
        .await
        .expect("create project");

    let created = owner
        .create_user_api_key(toplevel_key_req("tl-agent-proj"))
        .await
        .expect("create top-level api key");

    let grant = owner
        .create_project_grant(
            &ws.slug,
            "tl-key-proj",
            agent_grant_req(created.id, "viewer"),
        )
        .await
        .expect("granting a top-level key to its owner's project must succeed");

    assert_eq!(grant.principal.id, created.id);
    assert_eq!(grant.role, "viewer");
}

#[tokio::test]
async fn grant_api_key_not_owned_by_caller_is_rejected() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "tl-key-owner-check").await;

    let (other, other_user) = login_user(&server, &db, "tl-key-other-user").await;
    let _ = other_user;

    let other_key = other
        .create_user_api_key(toplevel_key_req("other-tl-agent"))
        .await
        .expect("create other user's top-level api key");

    let err = owner
        .create_workspace_grant(&ws.slug, agent_grant_req(other_key.id, "editor"))
        .await
        .expect_err("granting another user's key must be rejected");

    match err {
        ClientError::Api(p) => assert!(
            p.status == 422 || p.status == 403,
            "expected 422 or 403 for non-owner grant, got {}",
            p.status
        ),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn grant_toplevel_api_key_admin_is_still_rejected() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "tl-key-admin-cap").await;

    let created = owner
        .create_user_api_key(toplevel_key_req("tl-admin-cap-key"))
        .await
        .expect("create top-level api key");

    let err = owner
        .create_workspace_grant(&ws.slug, agent_grant_req(created.id, "admin"))
        .await
        .expect_err("admin grant to an api key must be rejected");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 403, "expected 403, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Regression: grant management stays agent-blocked regardless of scope
// ---------------------------------------------------------------------------

/// Grant management is a hard-coded human-only surface (`authorize_share`
/// rejects any `Principal::ApiKey` actor with `ShareDenied::AgentsNeverManageGrants`,
/// independent of the capability gate). An agent with an Editor grant on the
/// project — enough to clear the extractor's role threshold — and all 20
/// catalog capabilities must still be rejected when it tries to create a grant.
#[tokio::test]
async fn agent_with_all_capabilities_and_editor_grant_cannot_create_project_grant() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "grant-agent-allcap-owner").await;

    let project = owner
        .create_project(
            &ws.slug,
            proj_req("Agent AllCap Project", "agent-allcap-proj"),
        )
        .await
        .expect("create project");

    let plain = "atlas_grant_allcap_agent_secret";
    let hash = atlas_server::auth::tokens::hash_token(plain);

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    let key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "grant-allcap-agent".to_string(),
                token_hash: hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create all-capability agent key");

    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_domain::ids::{ApiKeyId, ProjectId};
    use atlas_server::persistence::repos::PermissionGrantRepo;
    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(ApiKeyId(key.id.0)),
            group_id: None,
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed agent editor grant on project");

    let (_, grantee_user) = add_user_to_workspace(
        &db,
        &server,
        ws.id,
        "grant-agent-allcap-target",
        MemberRole::Member,
    )
    .await;

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(plain);
    let result = agent
        .create_project_grant(
            &ws.slug,
            "agent-allcap-proj",
            grant_req(grantee_user.id.0, "viewer"),
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "an agent holding all 20 capabilities and an Editor grant must still be blocked from creating grants, got: {result:?}"
    );

    db.teardown().await;
}
