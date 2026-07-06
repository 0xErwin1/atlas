#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! B4 read-endpoint integration tests.
//!
//! RED tests (written first). Covers:
//! - GET /api/workspaces/{ws}/audit — workspace audit feed (owner/admin only)
//! - GET /api/admin/audit — platform audit feed (root/system_admin only)
//! - Actor enrichment: actor carries display_name + key_type + account_status
//! - Workspace isolation: only rows for that workspace are returned
//! - Access control: plain member → 403, api_key → 403, break-glass → 200
//! - Filters: actor (user), date range, action verb
//! - Keyset pagination: has_more / cursor

mod support;

use atlas_api::{dtos::audit::AuditEntryDto, pagination::Page};
use atlas_domain::{
    Actor,
    entities::identity::{ApiKeyType, MemberRole},
    entities::security_audit::{NewSecurityAuditEvent, SecurityAction},
    ids::{UserId, WorkspaceId},
};
use atlas_server::persistence::repos::{
    ApiKeyRepo, MembershipRepo, NewApiKey, PgSecurityAuditRepo,
};
use support::{TestDb, TestServer, login_root_user, login_user_with_workspace};

// ─── helpers ─────────────────────────────────────────────────────────────────

async fn insert_workspace_audit_row(
    db: &TestDb,
    ws_id: WorkspaceId,
    actor_user_id: UserId,
    action: SecurityAction,
) {
    PgSecurityAuditRepo::append_in(
        db.conn(),
        NewSecurityAuditEvent {
            workspace_id: Some(ws_id),
            actor: Actor::User(actor_user_id),
            action,
            target_type: "user".to_string(),
            target_id: Some(uuid::Uuid::now_v7()),
            metadata: serde_json::json!({}),
        },
    )
    .await
    .expect("insert workspace audit row");
}

async fn insert_platform_audit_row(db: &TestDb, actor_user_id: UserId, action: SecurityAction) {
    PgSecurityAuditRepo::append_in(
        db.conn(),
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: Actor::User(actor_user_id),
            action,
            target_type: "user".to_string(),
            target_id: Some(uuid::Uuid::now_v7()),
            metadata: serde_json::json!({}),
        },
    )
    .await
    .expect("insert platform audit row");
}

async fn get_workspace_audit(
    client: &atlas_client::AtlasClient,
    ws: &str,
) -> Result<Page<AuditEntryDto>, atlas_client::ClientError> {
    client
        .list_workspace_audit(ws, None, None, None, None, None)
        .await
}

async fn get_platform_audit(
    client: &atlas_client::AtlasClient,
) -> Result<Page<AuditEntryDto>, atlas_client::ClientError> {
    client
        .list_platform_audit(None, None, None, None, None)
        .await
}

// ─── workspace audit — owner sees own workspace rows ─────────────────────────

#[tokio::test]
async fn workspace_audit_owner_sees_rows() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (client, ws, user) = login_user_with_workspace(&server, &db, "audit-owner1").await;

    insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRoleChanged).await;

    let page = get_workspace_audit(&client, &ws.slug)
        .await
        .expect("GET audit");

    assert_eq!(page.items.len(), 1);
    let entry = &page.items[0];
    assert_eq!(entry.action, "membership.role_changed");
    assert_eq!(entry.workspace_id, Some(ws.id.0));

    assert_eq!(entry.actor.r#type, "user");
    assert_eq!(entry.actor.id, user.id.0);
    assert!(
        entry.actor.display_name.is_some(),
        "actor must carry display_name"
    );
    assert!(
        entry.actor.account_status.is_some(),
        "actor must carry account_status"
    );

    db.teardown().await;
}

// ─── workspace audit — admin sees rows ───────────────────────────────────────

#[tokio::test]
async fn workspace_audit_admin_sees_rows() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (_, ws, owner) = login_user_with_workspace(&server, &db, "audit-admin-ws-owner").await;

    // Create + log in an admin member directly using login_user_with_workspace
    // then override their membership to Admin.
    let (admin_client, _ws2, admin_user) =
        login_user_with_workspace(&server, &db, "audit-ws-admin-user").await;

    // Add them as Admin to the first workspace too.
    let ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(admin_user.id));
    db.membership_repo()
        .add(&ctx, admin_user.id, MemberRole::Admin)
        .await
        .expect("add admin membership");

    insert_workspace_audit_row(&db, ws.id, owner.id, SecurityAction::MembershipRoleChanged).await;

    let page = admin_client
        .list_workspace_audit(&ws.slug, None, None, None, None, None)
        .await
        .expect("admin GET audit");

    assert_eq!(page.items.len(), 1);

    db.teardown().await;
}

// ─── workspace audit — plain member → 403 ────────────────────────────────────

#[tokio::test]
async fn workspace_audit_member_forbidden() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (_, ws, _) = login_user_with_workspace(&server, &db, "audit-member-owner").await;

    // Create a member user via login_user_with_workspace then add as member to ws
    let (member_client, _, member_user) =
        login_user_with_workspace(&server, &db, "audit-member-plain-user").await;
    let ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(member_user.id));
    db.membership_repo()
        .add(&ctx, member_user.id, MemberRole::Member)
        .await
        .expect("add member membership");

    let err = member_client
        .list_workspace_audit(&ws.slug, None, None, None, None, None)
        .await
        .expect_err("should be 403");

    match err {
        atlas_client::ClientError::Api(p) => assert_eq!(p.status, 403),
        other => panic!("expected 403 Api error, got {other:?}"),
    }

    db.teardown().await;
}

// ─── workspace audit — non-member → 403 (WorkspaceOwnerOrAdmin rejects plain users) ──

#[tokio::test]
async fn workspace_audit_non_member_forbidden() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (_, ws, _) = login_user_with_workspace(&server, &db, "audit-outsider-ws").await;
    let (outsider, _) = support::login_user(&server, &db, "audit-outsider-user").await;

    let err = outsider
        .list_workspace_audit(&ws.slug, None, None, None, None, None)
        .await
        .expect_err("should be 403");

    match err {
        atlas_client::ClientError::Api(p) => assert_eq!(p.status, 403),
        other => panic!("expected 403 Api error, got {other:?}"),
    }

    db.teardown().await;
}

// ─── workspace audit — break-glass (root) → 200 ──────────────────────────────

#[tokio::test]
async fn workspace_audit_break_glass_allowed() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (_, ws, user) = login_user_with_workspace(&server, &db, "audit-bg-ws-owner").await;
    insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRoleChanged).await;

    let root_client = login_root_user(&server, &db).await;
    let page = root_client
        .list_workspace_audit(&ws.slug, None, None, None, None, None)
        .await
        .expect("root GET audit");

    assert_eq!(page.items.len(), 1);

    db.teardown().await;
}

// ─── workspace isolation: only own workspace rows ─────────────────────────────

#[tokio::test]
async fn workspace_audit_isolation() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (client_a, ws_a, user_a) = login_user_with_workspace(&server, &db, "audit-iso-ws-a").await;
    let (_, ws_b, user_b) = login_user_with_workspace(&server, &db, "audit-iso-ws-b").await;

    insert_workspace_audit_row(
        &db,
        ws_a.id,
        user_a.id,
        SecurityAction::MembershipRoleChanged,
    )
    .await;
    insert_workspace_audit_row(&db, ws_b.id, user_b.id, SecurityAction::MembershipRemoved).await;

    let page = get_workspace_audit(&client_a, &ws_a.slug)
        .await
        .expect("GET audit ws_a");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].workspace_id, Some(ws_a.id.0));

    db.teardown().await;
}

// ─── workspace audit — no platform rows bleed through ────────────────────────

#[tokio::test]
async fn workspace_audit_excludes_platform_rows() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (client, ws, user) = login_user_with_workspace(&server, &db, "audit-plat-excl-owner").await;

    insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRoleChanged).await;
    insert_platform_audit_row(&db, user.id, SecurityAction::UserCreated).await;

    let page = get_workspace_audit(&client, &ws.slug)
        .await
        .expect("GET workspace audit");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].action, "membership.role_changed");

    db.teardown().await;
}

// ─── platform audit — root sees platform rows ────────────────────────────────

#[tokio::test]
async fn platform_audit_root_sees_rows() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (_, ws, user) = login_user_with_workspace(&server, &db, "audit-plat-ws-owner").await;
    let root_client = login_root_user(&server, &db).await;

    insert_platform_audit_row(&db, user.id, SecurityAction::UserCreated).await;
    insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRoleChanged).await;

    let page = get_platform_audit(&root_client)
        .await
        .expect("GET platform audit");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].action, "user.created");
    assert!(page.items[0].workspace_id.is_none());

    db.teardown().await;
}

// ─── platform audit — normal user → 403 ─────────────────────────────────────

#[tokio::test]
async fn platform_audit_normal_user_forbidden() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (client, _, _) = login_user_with_workspace(&server, &db, "audit-plat-normal").await;

    let err = get_platform_audit(&client)
        .await
        .expect_err("should be 403");

    match err {
        atlas_client::ClientError::Api(p) => assert_eq!(p.status, 403),
        other => panic!("expected 403 Api error, got {other:?}"),
    }

    db.teardown().await;
}

// ─── workspace audit filter by action ────────────────────────────────────────

#[tokio::test]
async fn workspace_audit_filter_by_action() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (client, ws, user) =
        login_user_with_workspace(&server, &db, "audit-filter-action-owner").await;

    insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRoleChanged).await;
    insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRemoved).await;

    let page = client
        .list_workspace_audit(
            &ws.slug,
            None,
            Some("membership.role_changed"),
            None,
            None,
            None,
        )
        .await
        .expect("GET audit filtered by action");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].action, "membership.role_changed");

    db.teardown().await;
}

// ─── workspace audit keyset pagination ───────────────────────────────────────

#[tokio::test]
async fn workspace_audit_pagination() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (client, ws, user) =
        login_user_with_workspace(&server, &db, "audit-pagination-owner").await;

    for _ in 0..3 {
        insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRoleChanged)
            .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
    }

    let page1 = client
        .list_workspace_audit(&ws.slug, None, None, None, None, Some(2))
        .await
        .expect("GET audit page 1");

    assert_eq!(page1.items.len(), 2);
    assert!(page1.has_more);
    assert!(page1.next_cursor.is_some());

    let cursor = page1.next_cursor.as_deref().unwrap();
    // With explicit cursor:
    let page2_with_cursor = client
        .list_workspace_audit_with_cursor(&ws.slug, None, None, None, Some(cursor), Some(2))
        .await
        .expect("GET audit page 2 with cursor");

    assert_eq!(page2_with_cursor.items.len(), 1);
    assert!(!page2_with_cursor.has_more);

    db.teardown().await;
}

// ─── actor-type filter: human vs agent ───────────────────────────────────────

async fn seed_api_key_for_user(
    db: &TestDb,
    ws_id: WorkspaceId,
    user: &atlas_server::persistence::repos::User,
) -> atlas_domain::entities::identity::ApiKey {
    let ctx = atlas_domain::WorkspaceCtx::new(ws_id, Actor::User(user.id));
    let token_hash = atlas_server::auth::tokens::hash_token("audit-test-api-key-secret-unique-123");
    db.api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "audit-actor-filter-key".into(),
                token_hash,
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key for actor-filter test")
}

/// Seeds a workspace audit row whose actor is an API key, not a user.
async fn insert_workspace_audit_row_api_key(
    db: &TestDb,
    ws_id: WorkspaceId,
    api_key_id: atlas_domain::ids::ApiKeyId,
    action: SecurityAction,
) {
    PgSecurityAuditRepo::append_in(
        db.conn(),
        NewSecurityAuditEvent {
            workspace_id: Some(ws_id),
            actor: Actor::ApiKey(api_key_id),
            action,
            target_type: "user".to_string(),
            target_id: Some(uuid::Uuid::now_v7()),
            metadata: serde_json::json!({}),
        },
    )
    .await
    .expect("insert workspace audit row api_key");
}

#[tokio::test]
async fn workspace_audit_filter_actor_user() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (client, ws, user) =
        login_user_with_workspace(&server, &db, "audit-actor-user-filter").await;

    let key = seed_api_key_for_user(&db, ws.id, &user).await;

    // Seed one user-actor row and one api_key-actor row in the same workspace.
    insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRoleChanged).await;
    insert_workspace_audit_row_api_key(&db, ws.id, key.id, SecurityAction::MembershipRemoved).await;

    // Without filter: both rows visible.
    let all_page = get_workspace_audit(&client, &ws.slug)
        .await
        .expect("GET audit all");
    assert_eq!(all_page.items.len(), 2, "no filter must return both rows");

    // Filter actor=user: only the user-actor row.
    let user_page = client
        .list_workspace_audit(&ws.slug, Some("user"), None, None, None, None)
        .await
        .expect("GET audit actor=user");
    assert_eq!(
        user_page.items.len(),
        1,
        "actor=user must return exactly one row"
    );
    assert_eq!(
        user_page.items[0].actor.r#type, "user",
        "filtered row must be a user actor"
    );

    // Filter actor=api_key: only the api_key-actor row.
    let key_page = client
        .list_workspace_audit(&ws.slug, Some("api_key"), None, None, None, None)
        .await
        .expect("GET audit actor=api_key");
    assert_eq!(
        key_page.items.len(),
        1,
        "actor=api_key must return exactly one row"
    );
    assert_eq!(
        key_page.items[0].actor.r#type, "api_key",
        "filtered row must be an api_key actor"
    );

    db.teardown().await;
}

#[tokio::test]
async fn platform_audit_filter_actor_user() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (_, ws, user) =
        login_user_with_workspace(&server, &db, "audit-plat-actor-user-filter").await;
    let root_client = login_root_user(&server, &db).await;

    let key = seed_api_key_for_user(&db, ws.id, &user).await;

    // Seed one user-actor platform row and one api_key-actor platform row.
    insert_platform_audit_row(&db, user.id, SecurityAction::UserCreated).await;
    PgSecurityAuditRepo::append_in(
        db.conn(),
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: Actor::ApiKey(key.id),
            action: SecurityAction::UserDisabled,
            target_type: "user".to_string(),
            target_id: Some(uuid::Uuid::now_v7()),
            metadata: serde_json::json!({}),
        },
    )
    .await
    .expect("insert platform api_key audit row");

    // Without filter: both rows visible.
    let all_page = root_client
        .list_platform_audit(None, None, None, None, None)
        .await
        .expect("GET platform audit all");
    assert_eq!(all_page.items.len(), 2, "no filter must return both rows");

    // Filter actor=user: only user row.
    let user_page = root_client
        .list_platform_audit(Some("user"), None, None, None, None)
        .await
        .expect("GET platform audit actor=user");
    assert_eq!(user_page.items.len(), 1, "actor=user must return one row");
    assert_eq!(user_page.items[0].actor.r#type, "user");

    // Filter actor=api_key: only api_key row.
    let key_page = root_client
        .list_platform_audit(Some("api_key"), None, None, None, None)
        .await
        .expect("GET platform audit actor=api_key");
    assert_eq!(key_page.items.len(), 1, "actor=api_key must return one row");
    assert_eq!(key_page.items[0].actor.r#type, "api_key");

    db.teardown().await;
}

// ─── actor enrichment: display_name carried through ──────────────────────────

#[tokio::test]
async fn workspace_audit_actor_enrichment() {
    let db = TestDb::create().await.expect("db");
    let server = TestServer::spawn(&db).await;

    let (client, ws, user) = login_user_with_workspace(&server, &db, "audit-enrich-owner").await;

    insert_workspace_audit_row(&db, ws.id, user.id, SecurityAction::MembershipRoleChanged).await;

    let page = get_workspace_audit(&client, &ws.slug)
        .await
        .expect("GET audit");

    let entry = &page.items[0];
    assert_eq!(entry.actor.id, user.id.0);
    assert_eq!(
        entry.actor.display_name.as_deref(),
        Some("audit-enrich-owner"),
        "display_name must match"
    );
    assert_eq!(
        entry.actor.account_status.as_deref(),
        Some("active"),
        "account_status must be 'active'"
    );

    db.teardown().await;
}
