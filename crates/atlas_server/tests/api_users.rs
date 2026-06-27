#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{CreateUserRequest, LoginRequest};
use atlas_client::AtlasClient;
use support::{TestDb, TestServer, login_user_with_workspace};

// ── T50/T15: create_user requires admin (unchanged guard) ────────────────────

#[tokio::test]
async fn create_user_requires_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (non_admin, ws, _) = login_user_with_workspace(&server, &db, "non-admin-cu").await;

    let status = non_admin
        .create_user(CreateUserRequest {
            username: "newuser-noadmin".to_string(),
            display_name: "New User".to_string(),
            email: None,
            workspace: ws.slug.clone(),
            role: "member".to_string(),
        })
        .await;

    assert!(
        matches!(status, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 but got {status:?}"
    );

    db.teardown().await;
}

// ── T15: create user as pending — returns activation_link, membership added ──

#[tokio::test]
async fn create_user_succeeds_for_root_and_returns_activation_link() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (_, ws, _) = login_user_with_workspace(&server, &db, "owner-for-cu").await;

    let result = root
        .create_user(CreateUserRequest {
            username: "brandnew-pending".to_string(),
            display_name: "Brand New".to_string(),
            email: None,
            workspace: ws.slug.clone(),
            role: "member".to_string(),
        })
        .await
        .expect("create_user");

    assert_eq!(result.user.username, "brandnew-pending");
    assert_eq!(result.user.display_name, "Brand New");
    assert!(!result.user.is_root);
    assert!(result.user.disabled_at.is_none());
    assert!(
        result.user.activated_at.is_none(),
        "newly created user must be pending (activated_at = None)"
    );
    assert!(
        result.activation_link.contains("/activate/"),
        "activation_link must contain '/activate/' path, got: {}",
        result.activation_link
    );

    db.teardown().await;
}

// ── T15: role=owner yields 422 ────────────────────────────────────────────────

#[tokio::test]
async fn create_user_with_owner_role_returns_422() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (_, ws, _) = login_user_with_workspace(&server, &db, "owner-for-422").await;

    let result = root
        .create_user(CreateUserRequest {
            username: "owner-attempt".to_string(),
            display_name: "Owner Attempt".to_string(),
            email: None,
            workspace: ws.slug.clone(),
            role: "owner".to_string(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 422),
        "role=owner must return 422, got {result:?}"
    );

    db.teardown().await;
}

// ── T15: missing/unknown workspace slug yields 422 ────────────────────────────

#[tokio::test]
async fn create_user_with_unknown_workspace_returns_422() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;

    let result = root
        .create_user(CreateUserRequest {
            username: "ws-unknown-user".to_string(),
            display_name: "WS Unknown".to_string(),
            email: None,
            workspace: "this-workspace-does-not-exist".to_string(),
            role: "member".to_string(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 422),
        "unknown workspace must return 422, got {result:?}"
    );

    db.teardown().await;
}

// ── duplicate username on create_user → 409, not 500 ─────────────────────────

#[tokio::test]
async fn create_user_duplicate_username_returns_409() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (_, ws, _) = login_user_with_workspace(&server, &db, "owner-for-dup").await;

    root.create_user(CreateUserRequest {
        username: "dup-user".to_string(),
        display_name: "Dup User".to_string(),
        email: None,
        workspace: ws.slug.clone(),
        role: "member".to_string(),
    })
    .await
    .expect("first create_user");

    let second = root
        .create_user(CreateUserRequest {
            username: "dup-user".to_string(),
            display_name: "Dup User Again".to_string(),
            email: None,
            workspace: ws.slug.clone(),
            role: "member".to_string(),
        })
        .await;

    assert!(
        matches!(second, Err(atlas_client::ClientError::Api(ref p)) if p.status == 409),
        "duplicate username must return 409, got {second:?}"
    );

    // The unique index is on lower(username): a case-variant duplicate also 409s.
    let case_variant = root
        .create_user(CreateUserRequest {
            username: "DUP-USER".to_string(),
            display_name: "Dup Upper".to_string(),
            email: None,
            workspace: ws.slug.clone(),
            role: "member".to_string(),
        })
        .await;

    assert!(
        matches!(case_variant, Err(atlas_client::ClientError::Api(ref p)) if p.status == 409),
        "case-variant duplicate username must also return 409, got {case_variant:?}"
    );

    db.teardown().await;
}

// ── T15: created pending user cannot log in (401, no account-state oracle) ────

#[tokio::test]
async fn pending_user_created_via_api_cannot_login() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (_, ws, _) = login_user_with_workspace(&server, &db, "owner-for-pending-login").await;

    root.create_user(CreateUserRequest {
        username: "pending-api-user".to_string(),
        display_name: "Pending API".to_string(),
        email: None,
        workspace: ws.slug.clone(),
        role: "member".to_string(),
    })
    .await
    .expect("create_user");

    let result = AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: "pending-api-user".into(),
            password: "anypassword".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "pending user must return 401 on login (no account-state oracle), got: {result:?}"
    );

    db.teardown().await;
}

// ── T16: regenerate issues fresh token, invalidates prior token ───────────────

#[tokio::test]
async fn regenerate_activation_link_invalidates_prior_and_returns_new() {
    use atlas_server::persistence::repos::{ActivationTokenRepo, UserRepo};

    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (_, ws, _) = login_user_with_workspace(&server, &db, "owner-for-regen").await;

    let create_resp = root
        .create_user(CreateUserRequest {
            username: "regen-target".to_string(),
            display_name: "Regen Target".to_string(),
            email: None,
            workspace: ws.slug.clone(),
            role: "member".to_string(),
        })
        .await
        .expect("create_user");

    let user = db
        .user_repo()
        .find_by_username("regen-target")
        .await
        .expect("find_by_username")
        .expect("user must exist");

    // Extract token from the first link path: /activate/<token>
    let first_token = create_resp
        .activation_link
        .trim_start_matches('/')
        .split('/')
        .next_back()
        .unwrap()
        .to_string();

    let first_hash = atlas_server::auth::tokens::hash_token(&first_token);

    // Regenerate
    let regen_resp = root
        .regenerate_activation_link(user.id.0)
        .await
        .expect("regenerate_activation_link");

    let new_token = regen_resp
        .activation_link
        .trim_start_matches('/')
        .split('/')
        .next_back()
        .unwrap()
        .to_string();

    assert_ne!(first_token, new_token, "regenerated token must differ");
    assert!(
        regen_resp.activation_link.contains("/activate/"),
        "regenerated link must contain '/activate/', got: {}",
        regen_resp.activation_link
    );

    // Old token must no longer be active.
    let old_active = db
        .activation_token_repo()
        .find_active_by_token_hash(&first_hash)
        .await
        .expect("find_active_by_token_hash");
    assert!(
        old_active.is_none(),
        "old activation token must be invalidated after regenerate"
    );

    // New token must be active.
    let new_hash = atlas_server::auth::tokens::hash_token(&new_token);
    let new_active = db
        .activation_token_repo()
        .find_active_by_token_hash(&new_hash)
        .await
        .expect("find_active_by_token_hash for new");
    assert!(
        new_active.is_some(),
        "new activation token must be active after regenerate"
    );

    db.teardown().await;
}

// ── T16: regenerate on already-activated user yields 409 ─────────────────────

#[tokio::test]
async fn regenerate_activation_link_on_activated_user_returns_409() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (_, _, activated_user) =
        login_user_with_workspace(&server, &db, "activated-regen-target").await;

    let result = root.regenerate_activation_link(activated_user.id.0).await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 409),
        "regenerating link for already-activated user must return 409, got {result:?}"
    );

    db.teardown().await;
}

// ── T17: registry/openapi entries exist for new routes ────────────────────────

// (covered by openapi_drift test binary which runs against the router)

// ── disable_user_revokes_sessions (unchanged behavior test) ──────────────────

#[tokio::test]
async fn disable_user_revokes_sessions() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;

    let (victim_client, _, victim) =
        login_user_with_workspace(&server, &db, "victim-disable").await;

    victim_client.me().await.expect("me before disable");

    root.disable_user(victim.id.0).await.expect("disable_user");

    let err = victim_client.me().await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 after disable, got {err:?}"
    );

    let mut fresh_client = AtlasClient::new(server.base_url().to_string());
    let login_err = fresh_client
        .login(LoginRequest {
            username: "victim-disable".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await;
    assert!(
        matches!(login_err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "expected 401 on login after disable, got {login_err:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn enable_user_restores_access() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let (_, _, victim) = login_user_with_workspace(&server, &db, "victim2-enable").await;

    root.disable_user(victim.id.0).await.expect("disable");
    root.enable_user(victim.id.0).await.expect("enable");

    let mut restored = AtlasClient::new(server.base_url().to_string());
    restored
        .login(LoginRequest {
            username: "victim2-enable".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login after re-enable");

    db.teardown().await;
}

#[tokio::test]
async fn disable_requires_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (actor, _, _) = login_user_with_workspace(&server, &db, "actor-disable-req").await;
    let (_, _, target) = login_user_with_workspace(&server, &db, "target-disable-req").await;

    let err = actor.disable_user(target.id.0).await;
    assert!(
        matches!(err, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 but got {err:?}"
    );

    db.teardown().await;
}

// ── list_user_memberships: every workspace the user belongs to, with role ─────

#[tokio::test]
async fn list_user_memberships_returns_workspaces_with_roles() {
    use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole, ids::WorkspaceId};
    use atlas_server::persistence::repos::{
        MembershipRepo, NewUser, NewWorkspace, UserRepo, WorkspaceRepo,
    };

    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;

    let target = db
        .user_repo()
        .create(NewUser {
            username: "membership-target".to_string(),
            display_name: "Membership Target".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create target user");

    let ws_alpha = db
        .workspace_repo()
        .create(NewWorkspace {
            id: WorkspaceId::new(),
            name: "Alpha".to_string(),
            slug: "ws-alpha".to_string(),
        })
        .await
        .expect("create ws alpha");

    let ws_beta = db
        .workspace_repo()
        .create(NewWorkspace {
            id: WorkspaceId::new(),
            name: "Beta".to_string(),
            slug: "ws-beta".to_string(),
        })
        .await
        .expect("create ws beta");

    db.membership_repo()
        .add(
            &WorkspaceCtx::new(ws_alpha.id, Actor::User(target.id)),
            target.id,
            MemberRole::Member,
        )
        .await
        .expect("add member to ws alpha");

    db.membership_repo()
        .add(
            &WorkspaceCtx::new(ws_beta.id, Actor::User(target.id)),
            target.id,
            MemberRole::Admin,
        )
        .await
        .expect("add admin to ws beta");

    let memberships = root
        .list_user_memberships(target.id.0)
        .await
        .expect("list memberships");

    assert_eq!(
        memberships.len(),
        2,
        "expected two memberships, got {memberships:?}"
    );

    let alpha = memberships
        .iter()
        .find(|m| m.workspace_slug == "ws-alpha")
        .expect("alpha membership present");
    assert_eq!(alpha.workspace_name, "Alpha");
    assert_eq!(alpha.role, "member");

    let beta = memberships
        .iter()
        .find(|m| m.workspace_slug == "ws-beta")
        .expect("beta membership present");
    assert_eq!(beta.workspace_name, "Beta");
    assert_eq!(beta.role, "admin");

    db.teardown().await;
}

#[tokio::test]
async fn list_user_memberships_requires_admin() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (non_admin, _ws, _user) =
        login_user_with_workspace(&server, &db, "non-admin-memberships").await;
    let (_, _, target) = login_user_with_workspace(&server, &db, "memberships-target-403").await;

    let result = non_admin.list_user_memberships(target.id.0).await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "expected 403 but got {result:?}"
    );

    db.teardown().await;
}
