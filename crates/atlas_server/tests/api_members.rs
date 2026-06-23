#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_client::{AtlasClient, ClientError};
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

// ── helpers for B2 security matrix tests ─────────────────────────────────────

async fn login_member_with_role(
    server: &TestServer,
    db: &TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    username: &str,
    role: MemberRole,
) -> (AtlasClient, atlas_domain::entities::identity::User) {
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

    let mut client = AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    (client, user)
}

async fn login_break_glass_user(
    server: &TestServer,
    db: &TestDb,
    username: &str,
    is_root: bool,
    is_system_admin: bool,
) -> AtlasClient {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    db.user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: hash,
            is_root,
            is_system_admin,
        })
        .await
        .expect("create break-glass user");

    let mut client = AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    client
}

fn assert_forbidden(result: Result<impl std::fmt::Debug, ClientError>, context: &str) {
    match result {
        Err(ClientError::Api(p)) => assert_eq!(
            p.status, 403,
            "{context}: expected 403 but got {}: {}",
            p.status, p.title
        ),
        other => panic!("{context}: expected 403 ClientError, got {other:?}"),
    }
}

fn assert_not_found(result: Result<impl std::fmt::Debug, ClientError>, context: &str) {
    match result {
        Err(ClientError::Api(p)) => assert_eq!(
            p.status, 404,
            "{context}: expected 404 but got {}: {}",
            p.status, p.title
        ),
        other => panic!("{context}: expected 404 ClientError, got {other:?}"),
    }
}

fn assert_last_owner(result: Result<impl std::fmt::Debug, ClientError>, context: &str) {
    match result {
        Err(ClientError::Api(p)) => {
            assert_eq!(
                p.status, 409,
                "{context}: expected 409 but got {}: {}",
                p.status, p.title
            );
            assert_eq!(
                p.r#type, "urn:atlas:error:last-owner",
                "{context}: expected last-owner urn"
            );
        }
        other => panic!("{context}: expected 409 last-owner error, got {other:?}"),
    }
}

// ── T11: WorkspaceOwnerOrAdmin — plain member → 403 ─────────────────────────

#[tokio::test]
async fn workspace_owner_or_admin_plain_member_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "bg-member-403-owner").await;

    let (member_client, target_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "bg-member-403-member",
        MemberRole::Member,
    )
    .await;

    let result = member_client
        .update_member_role(&ws.slug, owner_user.id.0, "admin")
        .await;

    assert_forbidden(result, "plain member PATCH");

    let _ = target_user;
    db.teardown().await;
}

// ── T12: WorkspaceOwnerOrAdmin — api-key principal → 403 ────────────────────

#[tokio::test]
async fn workspace_owner_or_admin_api_key_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "bg-apikey-403-owner").await;

    let target_user = add_member(&db, ws.id, "bg-apikey-403-target", MemberRole::Member).await;

    let plain_token = "atlas_bg_apikey_403_secret".to_string();
    let token_hash = atlas_server::auth::tokens::hash_token(&plain_token);

    let api_key_repo = db.api_key_repo();
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    api_key_repo
        .create(
            &ctx,
            atlas_server::persistence::repos::NewApiKey {
                name: "test-key-403".to_string(),
                token_hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
            },
        )
        .await
        .expect("create api key");

    let api_client = AtlasClient::new(server.base_url().to_string()).with_token(plain_token);

    let result = api_client
        .update_member_role(&ws.slug, target_user.id.0, "admin")
        .await;

    assert_forbidden(result, "api-key PATCH");

    db.teardown().await;
}

// ── T13: WorkspaceOwnerOrAdmin — unauthenticated → 401 ──────────────────────

#[tokio::test]
async fn workspace_owner_or_admin_unauthenticated_returns_401() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "bg-unauth-401-owner").await;

    let anon = AtlasClient::new(server.base_url().to_string());

    let result = anon
        .update_member_role(&ws.slug, owner_user.id.0, "admin")
        .await;

    match result {
        Err(ClientError::Api(p)) => assert_eq!(
            p.status, 401,
            "expected 401 but got {}: {}",
            p.status, p.title
        ),
        other => panic!("expected 401, got {other:?}"),
    }

    db.teardown().await;
}

// ── T14: WorkspaceOwnerOrAdmin — non-member root (break-glass) → PASS ───────

#[tokio::test]
async fn workspace_owner_or_admin_non_member_root_passes() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "bg-root-pass-owner").await;

    let target_user = add_member(&db, ws.id, "bg-root-pass-target", MemberRole::Member).await;

    let another_member = add_member(&db, ws.id, "bg-root-pass-second", MemberRole::Owner).await;

    let root_client = login_break_glass_user(&server, &db, "bg-root-pass-root", true, false).await;

    let result = root_client
        .update_member_role(&ws.slug, target_user.id.0, "admin")
        .await;

    assert!(
        result.is_ok(),
        "non-member root must be able to update roles, got: {result:?}"
    );

    let _ = another_member;
    db.teardown().await;
}

// ── T15: WorkspaceOwnerOrAdmin — non-member sysadmin → PASS ─────────────────

#[tokio::test]
async fn workspace_owner_or_admin_non_member_sysadmin_passes() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "bg-sysadmin-pass-owner").await;

    let target_user = add_member(&db, ws.id, "bg-sysadmin-pass-target", MemberRole::Member).await;

    let another_member = add_member(&db, ws.id, "bg-sysadmin-pass-second", MemberRole::Owner).await;

    let sysadmin_client =
        login_break_glass_user(&server, &db, "bg-sysadmin-pass-sysadmin", false, true).await;

    let result = sysadmin_client
        .update_member_role(&ws.slug, target_user.id.0, "admin")
        .await;

    assert!(
        result.is_ok(),
        "non-member sysadmin must be able to update roles, got: {result:?}"
    );

    let _ = another_member;
    db.teardown().await;
}

// ── T16: WorkspaceOwnerOrAdmin — owner member → PASS ────────────────────────

#[tokio::test]
async fn workspace_owner_or_admin_owner_passes() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "bg-owner-pass-owner").await;

    let target_user = add_member(&db, ws.id, "bg-owner-pass-target", MemberRole::Member).await;

    let another_owner =
        add_member(&db, ws.id, "bg-owner-pass-second-owner", MemberRole::Owner).await;

    let result = owner_client
        .update_member_role(&ws.slug, target_user.id.0, "admin")
        .await;

    assert!(
        result.is_ok(),
        "owner must be able to update member roles, got: {result:?}"
    );

    let _ = another_owner;
    db.teardown().await;
}

// ── T17: WorkspaceOwnerOrAdmin — admin member → PASS ────────────────────────

#[tokio::test]
async fn workspace_owner_or_admin_admin_passes() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "bg-admin-pass-owner").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "bg-admin-pass-admin",
        MemberRole::Admin,
    )
    .await;

    let target_user = add_member(&db, ws.id, "bg-admin-pass-target", MemberRole::Member).await;

    let result = admin_client
        .update_member_role(&ws.slug, target_user.id.0, "admin")
        .await;

    assert!(
        result.is_ok(),
        "admin must be able to update member roles, got: {result:?}"
    );

    db.teardown().await;
}

// ── T18: PATCH — admin on owner target → 403 ────────────────────────────────

#[tokio::test]
async fn patch_admin_on_owner_target_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "pat-admin-owner-403-ws").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "pat-admin-owner-403-admin",
        MemberRole::Admin,
    )
    .await;

    let result = admin_client
        .update_member_role(&ws.slug, owner_user.id.0, "member")
        .await;

    match result {
        Err(ClientError::Api(p)) => {
            assert_eq!(p.status, 403, "expected 403, got {}", p.status);
            assert!(
                p.detail
                    .as_deref()
                    .unwrap_or("")
                    .contains("Admins cannot modify an owner's membership"),
                "detail must mention the reason, got: {:?}",
                p.detail
            );
        }
        other => panic!("expected 403, got {other:?}"),
    }

    db.teardown().await;
}

// ── T19: PATCH — admin sets new_role=owner on non-owner target → 403 ─────────

#[tokio::test]
async fn patch_admin_promote_to_owner_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "pat-admin-owner-prom-ws").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "pat-admin-owner-prom-admin",
        MemberRole::Admin,
    )
    .await;

    let target_user = add_member(
        &db,
        ws.id,
        "pat-admin-owner-prom-target",
        MemberRole::Member,
    )
    .await;

    let result = admin_client
        .update_member_role(&ws.slug, target_user.id.0, "owner")
        .await;

    match result {
        Err(ClientError::Api(p)) => {
            assert_eq!(p.status, 403, "expected 403, got {}", p.status);
            assert!(
                p.detail
                    .as_deref()
                    .unwrap_or("")
                    .contains("Only an owner can grant the owner role"),
                "detail must mention reason, got: {:?}",
                p.detail
            );
        }
        other => panic!("expected 403, got {other:?}"),
    }

    db.teardown().await;
}

// ── T20: PATCH — admin demotes admin→member (legal) → 200 ───────────────────

#[tokio::test]
async fn patch_admin_demotes_admin_to_member_returns_200() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "pat-admin-demote-ws").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "pat-admin-demote-caller",
        MemberRole::Admin,
    )
    .await;

    let target_user = add_member(&db, ws.id, "pat-admin-demote-target", MemberRole::Admin).await;

    let result = admin_client
        .update_member_role(&ws.slug, target_user.id.0, "member")
        .await;

    assert!(
        result.is_ok(),
        "admin demoting admin to member must succeed, got: {result:?}"
    );
    assert_eq!(
        result.unwrap().role.as_deref(),
        Some("member"),
        "role must be updated to member"
    );

    db.teardown().await;
}

// ── T21: PATCH — admin promotes member→admin (legal) → 200 ──────────────────

#[tokio::test]
async fn patch_admin_promotes_member_to_admin_returns_200() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "pat-admin-promote-ws").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "pat-admin-promote-caller",
        MemberRole::Admin,
    )
    .await;

    let target_user = add_member(&db, ws.id, "pat-admin-promote-target", MemberRole::Member).await;

    let result = admin_client
        .update_member_role(&ws.slug, target_user.id.0, "admin")
        .await;

    assert!(
        result.is_ok(),
        "admin promoting member to admin must succeed, got: {result:?}"
    );
    assert_eq!(
        result.unwrap().role.as_deref(),
        Some("admin"),
        "role must be updated to admin"
    );

    db.teardown().await;
}

// ── T22: PATCH — owner demotes non-last owner→member (legal) → 200 ──────────

#[tokio::test]
async fn patch_owner_demotes_non_last_owner_returns_200() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "pat-owner-demote-ws").await;

    let target_owner = add_member(&db, ws.id, "pat-owner-demote-target", MemberRole::Owner).await;

    let result = owner_client
        .update_member_role(&ws.slug, target_owner.id.0, "member")
        .await;

    assert!(
        result.is_ok(),
        "owner demoting another owner (not last) must succeed, got: {result:?}"
    );

    db.teardown().await;
}

// ── T23: PATCH — owner demotes LAST owner → 409 ─────────────────────────────

#[tokio::test]
async fn patch_owner_demotes_last_owner_returns_409() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) =
        login_user_with_workspace(&server, &db, "pat-last-owner-409-ws").await;

    let result = owner_client
        .update_member_role(&ws.slug, owner_user.id.0, "admin")
        .await;

    assert_last_owner(result, "owner demoting self (last owner)");

    db.teardown().await;
}

// ── T24: PATCH — break-glass demotes LAST owner → 409 ───────────────────────

#[tokio::test]
async fn patch_break_glass_demotes_last_owner_returns_409() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "pat-bg-last-owner-409-ws").await;

    let root_client =
        login_break_glass_user(&server, &db, "pat-bg-last-owner-409-root", true, false).await;

    let result = root_client
        .update_member_role(&ws.slug, owner_user.id.0, "admin")
        .await;

    assert_last_owner(result, "break-glass demoting last owner");

    db.teardown().await;
}

// ── T25: PATCH — break-glass sets any role on non-last owner → 200 ──────────

#[tokio::test]
async fn patch_break_glass_sets_role_on_non_last_owner_returns_200() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "pat-bg-non-last-ws").await;

    let target_owner = add_member(&db, ws.id, "pat-bg-non-last-target", MemberRole::Owner).await;

    let root_client =
        login_break_glass_user(&server, &db, "pat-bg-non-last-root", true, false).await;

    let result = root_client
        .update_member_role(&ws.slug, target_owner.id.0, "member")
        .await;

    assert!(
        result.is_ok(),
        "break-glass can demote a non-last owner, got: {result:?}"
    );

    db.teardown().await;
}

// ── T26: PATCH — same-role no-op → idempotent 200 ───────────────────────────

#[tokio::test]
async fn patch_same_role_is_idempotent_200() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "pat-idempotent-ws").await;

    let target_user = add_member(&db, ws.id, "pat-idempotent-target", MemberRole::Admin).await;

    let result = owner_client
        .update_member_role(&ws.slug, target_user.id.0, "admin")
        .await;

    assert!(
        result.is_ok(),
        "same-role PATCH must be idempotent 200, got: {result:?}"
    );

    db.teardown().await;
}

// ── T27: PATCH — admin same-role on owner target → still 403 ────────────────

#[tokio::test]
async fn patch_admin_same_role_on_owner_still_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "pat-admin-owner-same-ws").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "pat-admin-owner-same-admin",
        MemberRole::Admin,
    )
    .await;

    let result = admin_client
        .update_member_role(&ws.slug, owner_user.id.0, "owner")
        .await;

    assert_forbidden(
        result,
        "admin setting owner to owner (same-role) must still 403",
    );

    db.teardown().await;
}

// ── T28: PATCH — target not a member → 404 ──────────────────────────────────

#[tokio::test]
async fn patch_target_not_member_returns_404() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "pat-target-404-ws").await;

    let stranger = db
        .user_repo()
        .create(NewUser {
            username: "pat-target-404-stranger".into(),
            display_name: "stranger".into(),
            email: None,
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$hash".into(),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create stranger");

    let result = owner_client
        .update_member_role(&ws.slug, stranger.id.0, "admin")
        .await;

    assert_not_found(result, "target not a member PATCH");

    db.teardown().await;
}

// ── T29: PATCH — unknown role string → 422 ──────────────────────────────────

#[tokio::test]
async fn patch_unknown_role_returns_422() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "pat-unknown-role-ws").await;

    let target_user = add_member(&db, ws.id, "pat-unknown-role-target", MemberRole::Member).await;

    let result = owner_client
        .update_member_role(&ws.slug, target_user.id.0, "superuser")
        .await;

    match result {
        Err(ClientError::Api(p)) => assert_eq!(
            p.status, 422,
            "unknown role must return 422, got {}: {}",
            p.status, p.title
        ),
        other => panic!("expected 422, got {other:?}"),
    }

    db.teardown().await;
}

// ── T30: PATCH — malformed JSON body → 400 ──────────────────────────────────

#[tokio::test]
async fn patch_malformed_json_returns_400() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_, ws, owner_user) =
        login_user_with_workspace(&server, &db, "pat-malformed-json-ws").await;

    let (owner2, _owner2_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "pat-malformed-json-owner2",
        MemberRole::Owner,
    )
    .await;

    let target_user = add_member(&db, ws.id, "pat-malformed-json-target", MemberRole::Member).await;

    let response = owner2
        .http_client()
        .patch(format!(
            "{}/v1/workspaces/{}/members/{}",
            server.base_url(),
            ws.slug,
            target_user.id.0
        ))
        .bearer_auth(owner2.token().unwrap_or(""))
        .header("x-atlas-csrf", "1")
        .header("content-type", "application/json")
        .body("not valid json at all {{{")
        .send()
        .await
        .expect("send");

    let status = response.status().as_u16();
    assert_eq!(status, 400, "malformed JSON must return 400, got {status}");

    let _ = owner_user;
    db.teardown().await;
}

// ── T31: DELETE — admin removes owner → 403 ─────────────────────────────────

#[tokio::test]
async fn delete_admin_removes_owner_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "del-admin-owner-403-ws").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "del-admin-owner-403-admin",
        MemberRole::Admin,
    )
    .await;

    let result = admin_client.remove_member(&ws.slug, owner_user.id.0).await;

    assert_forbidden(result, "admin removing owner");

    db.teardown().await;
}

// ── T32: DELETE — admin removes member (legal) → 204 ────────────────────────

#[tokio::test]
async fn delete_admin_removes_member_returns_204() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "del-admin-member-204-ws").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "del-admin-member-204-admin",
        MemberRole::Admin,
    )
    .await;

    let target_user = add_member(
        &db,
        ws.id,
        "del-admin-member-204-target",
        MemberRole::Member,
    )
    .await;

    let result = admin_client.remove_member(&ws.slug, target_user.id.0).await;

    assert!(
        result.is_ok(),
        "admin removing a member must succeed, got: {result:?}"
    );

    db.teardown().await;
}

// ── T33: DELETE — admin removes admin (legal) → 204 ─────────────────────────

#[tokio::test]
async fn delete_admin_removes_admin_returns_204() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "del-admin-admin-204-ws").await;

    let (admin_client, _admin_user) = login_member_with_role(
        &server,
        &db,
        ws.id,
        "del-admin-admin-204-caller",
        MemberRole::Admin,
    )
    .await;

    let target_user = add_member(&db, ws.id, "del-admin-admin-204-target", MemberRole::Admin).await;

    let result = admin_client.remove_member(&ws.slug, target_user.id.0).await;

    assert!(
        result.is_ok(),
        "admin removing another admin must succeed, got: {result:?}"
    );

    db.teardown().await;
}

// ── T34: DELETE — owner removes non-last owner (legal) → 204 ────────────────

#[tokio::test]
async fn delete_owner_removes_non_last_owner_returns_204() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "del-owner-non-last-ws").await;

    let target_owner = add_member(&db, ws.id, "del-owner-non-last-target", MemberRole::Owner).await;

    let result = owner_client
        .remove_member(&ws.slug, target_owner.id.0)
        .await;

    assert!(
        result.is_ok(),
        "owner removing non-last owner must succeed, got: {result:?}"
    );

    db.teardown().await;
}

// ── T35: DELETE — owner tries to remove LAST owner → 409 ────────────────────

#[tokio::test]
async fn delete_owner_removes_last_owner_returns_409() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) =
        login_user_with_workspace(&server, &db, "del-last-owner-409-ws").await;

    let result = owner_client.remove_member(&ws.slug, owner_user.id.0).await;

    assert_last_owner(result, "owner removing self (last owner)");

    db.teardown().await;
}

// ── T36: DELETE — break-glass removes non-last owner (legal) → 204 ──────────

#[tokio::test]
async fn delete_break_glass_removes_non_last_owner_returns_204() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "del-bg-non-last-ws").await;

    let target_owner = add_member(&db, ws.id, "del-bg-non-last-target", MemberRole::Owner).await;

    let root_client =
        login_break_glass_user(&server, &db, "del-bg-non-last-root", true, false).await;

    let result = root_client.remove_member(&ws.slug, target_owner.id.0).await;

    assert!(
        result.is_ok(),
        "break-glass removing non-last owner must succeed, got: {result:?}"
    );

    db.teardown().await;
}

// ── T37: DELETE — break-glass tries to remove LAST owner → 409 ──────────────

#[tokio::test]
async fn delete_break_glass_removes_last_owner_returns_409() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "del-bg-last-owner-409-ws").await;

    let root_client =
        login_break_glass_user(&server, &db, "del-bg-last-owner-409-root", true, false).await;

    let result = root_client.remove_member(&ws.slug, owner_user.id.0).await;

    assert_last_owner(result, "break-glass removing last owner");

    db.teardown().await;
}

// ── T38: DELETE — target not a member → 404 ─────────────────────────────────

#[tokio::test]
async fn delete_target_not_member_returns_404() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _owner_user) =
        login_user_with_workspace(&server, &db, "del-target-404-ws").await;

    let stranger = db
        .user_repo()
        .create(NewUser {
            username: "del-target-404-stranger".into(),
            display_name: "stranger".into(),
            email: None,
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$hash".into(),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create stranger");

    let result = owner_client.remove_member(&ws.slug, stranger.id.0).await;

    assert_not_found(result, "target not a member DELETE");

    db.teardown().await;
}
