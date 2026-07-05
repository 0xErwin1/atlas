//! B1 — Global-admin bypass (A1-A5) TDD integration tests.
//!
//! RED direction: proves that `is_root || is_system_admin` grants admin-level
//! access to every workspace's content and management routes without being a
//! member.
//!
//! REGRESSION direction: proves that the bypass does NOT over-grant — a plain
//! `MemberRole::Member`, an api_key, and a non-admin non-member all remain
//! exactly as scoped as before.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::{
    dtos::{LoginRequest, UpdateWorkspaceRequest, search::SearchHitDto},
    pagination::Page,
};
use atlas_client::AtlasClient;
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{documents::NewDocument, identity::ApiKeyType},
};
use atlas_server::{
    auth::{
        password,
        tokens::{generate_api_key, hash_token},
    },
    persistence::repos::{
        ApiKeyRepo, DocumentRepo, NewApiKey, NewUser, PgApiKeyRepo, PgDocumentRepo, UserRepo,
    },
};
use support::TestDb;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

async fn create_system_admin(
    db: &TestDb,
    username: &str,
) -> atlas_domain::entities::identity::User {
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
            is_system_admin: true,
        })
        .await
        .expect("create system admin");

    support::activate_user_in_db(db, user.id.0).await;
    user
}

async fn create_root_user(db: &TestDb, username: &str) -> atlas_domain::entities::identity::User {
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
            is_root: true,
            is_system_admin: false,
        })
        .await
        .expect("create root user");

    support::activate_user_in_db(db, user.id.0).await;
    user
}

async fn login_as(server: &support::TestServer, username: &str) -> AtlasClient {
    let mut client = server.client();
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");
    client
}

/// Seeds a document in the given workspace (owner context).
/// Returns (document_id, document_slug). Slug is auto-generated from the title.
async fn seed_document_in_ws(
    db: &TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    owner: &atlas_domain::entities::identity::User,
    title: &str,
    content: &str,
) -> (atlas_domain::ids::DocumentId, Option<String>) {
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let doc = repo
        .create(
            &ctx,
            NewDocument {
                title: title.to_string(),
                slug: Some(title.to_lowercase().replace(' ', "-")),
                content: content.to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");
    (doc.id, doc.slug)
}

/// Creates an API key for the given workspace.
async fn create_api_key(
    db: &TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    owner: &atlas_domain::entities::identity::User,
    name: &str,
) -> (atlas_domain::ids::ApiKeyId, String) {
    let raw_token = generate_api_key();
    let token_hash = hash_token(&raw_token);
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));

    let key = PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewApiKey {
            name: name.to_string(),
            token_hash,
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
        },
    )
    .await
    .expect("create api key");

    (key.id, raw_token)
}

fn search_url(base: &str, ws: &str, q: &str) -> String {
    format!("{base}/v1/workspaces/{ws}/search?q={q}")
}

async fn search_hits(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    ws_slug: &str,
    q: &str,
) -> (u16, Vec<SearchHitDto>) {
    let resp = http
        .get(search_url(base, ws_slug, q))
        .bearer_auth(token)
        .send()
        .await
        .expect("HTTP request");

    let status = resp.status().as_u16();
    if status == 200 {
        let page: Page<SearchHitDto> = resp.json().await.expect("decode page");
        (status, page.items)
    } else {
        (status, vec![])
    }
}

// ---------------------------------------------------------------------------
// A1 — WorkspaceMember bypass (get_workspace, list_members)
// ---------------------------------------------------------------------------

/// A system_admin who is NOT a member of a workspace can get that workspace
/// via the WorkspaceMember extractor (A1).
#[tokio::test]
async fn system_admin_non_member_gets_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, _owner) = support::seed_workspace(&db, "byp-sa-get-ws-owner").await;
    create_system_admin(&db, "byp-sa-get-ws-admin").await;
    let sysadmin = login_as(&server, "byp-sa-get-ws-admin").await;

    let result = sysadmin.get_workspace(&target_ws.slug).await;
    assert!(
        result.is_ok(),
        "system_admin non-member must be able to get any workspace (A1), got: {result:?}"
    );
    assert_eq!(result.unwrap().slug, target_ws.slug);

    db.teardown().await;
}

/// A root user who is NOT a member of a workspace can get that workspace (A1).
#[tokio::test]
async fn root_non_member_gets_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, _owner) = support::seed_workspace(&db, "byp-root-get-ws-owner").await;
    create_root_user(&db, "byp-root-get-ws-root").await;
    let root = login_as(&server, "byp-root-get-ws-root").await;

    let result = root.get_workspace(&target_ws.slug).await;
    assert!(
        result.is_ok(),
        "root non-member must be able to get any workspace (A1), got: {result:?}"
    );
    assert_eq!(result.unwrap().slug, target_ws.slug);

    db.teardown().await;
}

/// A system_admin who is NOT a member can list members of any workspace (A1).
#[tokio::test]
async fn system_admin_non_member_lists_workspace_members() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, _owner) = support::seed_workspace(&db, "byp-sa-list-mbr-owner").await;
    create_system_admin(&db, "byp-sa-list-mbr-admin").await;
    let sysadmin = login_as(&server, "byp-sa-list-mbr-admin").await;

    let result = sysadmin.list_workspace_members(&target_ws.slug).await;
    assert!(
        result.is_ok(),
        "system_admin non-member must be able to list workspace members (A1), got: {result:?}"
    );

    db.teardown().await;
}

/// A system_admin who is NOT a member can rename any workspace (A1 + write).
#[tokio::test]
async fn system_admin_non_member_renames_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, _owner) = support::seed_workspace(&db, "byp-sa-rename-ws-owner").await;
    create_system_admin(&db, "byp-sa-rename-ws-admin").await;
    let sysadmin = login_as(&server, "byp-sa-rename-ws-admin").await;

    let result = sysadmin
        .update_workspace(
            &target_ws.slug,
            UpdateWorkspaceRequest {
                name: "SA Renamed Workspace".to_string(),
            },
        )
        .await;
    assert!(
        result.is_ok(),
        "system_admin non-member must be able to rename any workspace (A1), got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// A3 — Authorized bypass (content routes: documents)
// ---------------------------------------------------------------------------

/// A system_admin who is NOT a member can read a specific document in any workspace (A3).
/// This exercises the Authorized<R,M> extractor which gates all content routes.
#[tokio::test]
async fn system_admin_non_member_reads_document() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, owner) = support::seed_workspace(&db, "byp-sa-read-doc-owner").await;
    let (_doc_id, doc_slug) =
        seed_document_in_ws(&db, &target_ws, &owner, "SA Test Doc", "saidoc content").await;
    let doc_slug = doc_slug.expect("document must have a slug");
    create_system_admin(&db, "byp-sa-read-doc-admin").await;
    let sysadmin = login_as(&server, "byp-sa-read-doc-admin").await;

    let doc = sysadmin.get_document(&target_ws.slug, &doc_slug).await;
    assert!(
        doc.is_ok(),
        "system_admin non-member must be able to read a document in any workspace (A3/Authorized), got: {doc:?}"
    );

    db.teardown().await;
}

/// A root user who is NOT a member can read a document in any workspace (A3).
#[tokio::test]
async fn root_non_member_reads_document() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, owner) = support::seed_workspace(&db, "byp-root-read-doc-owner").await;
    let (_doc_id, doc_slug) =
        seed_document_in_ws(&db, &target_ws, &owner, "Root Test Doc", "rootdoc content").await;
    let doc_slug = doc_slug.expect("document must have a slug");
    create_root_user(&db, "byp-root-read-doc-root").await;
    let root = login_as(&server, "byp-root-read-doc-root").await;

    let doc = root.get_document(&target_ws.slug, &doc_slug).await;
    assert!(
        doc.is_ok(),
        "root non-member must be able to read a document in any workspace (A3/Authorized), got: {doc:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// A4 — list_workspaces returns ALL for is_root/is_system_admin
// ---------------------------------------------------------------------------

/// GET /v1/workspaces as system_admin returns ALL workspaces including
/// ones they are NOT a member of (A4).
#[tokio::test]
async fn system_admin_list_workspaces_returns_all() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (ws_a, _) = support::seed_workspace(&db, "byp-sa-listall-ws-a").await;
    let (ws_b, _) = support::seed_workspace(&db, "byp-sa-listall-ws-b").await;
    create_system_admin(&db, "byp-sa-listall-admin").await;
    let sysadmin = login_as(&server, "byp-sa-listall-admin").await;

    let workspaces = sysadmin.list_workspaces().await.expect("list_workspaces");

    let slugs: Vec<&str> = workspaces.iter().map(|w| w.slug.as_str()).collect();
    assert!(
        slugs.contains(&ws_a.slug.as_str()),
        "system_admin list_workspaces must include ws_a (A4); got: {slugs:?}"
    );
    assert!(
        slugs.contains(&ws_b.slug.as_str()),
        "system_admin list_workspaces must include ws_b (A4); got: {slugs:?}"
    );

    db.teardown().await;
}

/// GET /v1/workspaces as root returns ALL workspaces (A4).
#[tokio::test]
async fn root_list_workspaces_returns_all() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (ws_a, _) = support::seed_workspace(&db, "byp-root-listall-ws-a").await;
    let (ws_b, _) = support::seed_workspace(&db, "byp-root-listall-ws-b").await;
    create_root_user(&db, "byp-root-listall-root").await;
    let root = login_as(&server, "byp-root-listall-root").await;

    let workspaces = root.list_workspaces().await.expect("list_workspaces");

    let slugs: Vec<&str> = workspaces.iter().map(|w| w.slug.as_str()).collect();
    assert!(
        slugs.contains(&ws_a.slug.as_str()),
        "root list_workspaces must include ws_a (A4); got: {slugs:?}"
    );
    assert!(
        slugs.contains(&ws_b.slug.as_str()),
        "root list_workspaces must include ws_b (A4); got: {slugs:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// A2 + A5 — WorkspaceAccess bypass + search-SQL bypass
// ---------------------------------------------------------------------------

/// system_admin search in a workspace they are NOT a member of returns
/// all documents (A2 + A5 search-SQL bypass).
#[tokio::test]
async fn system_admin_search_in_non_member_workspace_returns_results() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, owner) = support::seed_workspace(&db, "byp-sa-search-owner").await;
    let unique = "sysadmin_bypass_searchtoken_9z4";
    seed_document_in_ws(
        &db,
        &target_ws,
        &owner,
        &format!("SA Search Doc {unique}"),
        unique,
    )
    .await;

    create_system_admin(&db, "byp-sa-search-admin").await;
    let sysadmin = login_as(&server, "byp-sa-search-admin").await;

    let http = reqwest::Client::new();
    let token = sysadmin.token().expect("sysadmin token");
    let (status, hits) =
        search_hits(&http, token, server.base_url(), &target_ws.slug, unique).await;

    assert_eq!(
        status, 200,
        "system_admin non-member must reach search (A2), got status {status}"
    );
    assert!(
        !hits.is_empty(),
        "system_admin search must return the seeded document (A5 bypass), got: {hits:?}"
    );

    db.teardown().await;
}

/// root search in a workspace they are NOT a member of returns all documents (A2 + A5).
#[tokio::test]
async fn root_search_in_non_member_workspace_returns_results() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, owner) = support::seed_workspace(&db, "byp-root-search-owner").await;
    let unique = "root_bypass_searchtoken_9z4";
    seed_document_in_ws(
        &db,
        &target_ws,
        &owner,
        &format!("Root Search Doc {unique}"),
        unique,
    )
    .await;

    create_root_user(&db, "byp-root-search-root").await;
    let root = login_as(&server, "byp-root-search-root").await;

    let http = reqwest::Client::new();
    let token = root.token().expect("root token");
    let (status, hits) =
        search_hits(&http, token, server.base_url(), &target_ws.slug, unique).await;

    assert_eq!(
        status, 200,
        "root non-member must reach search (A2), got status {status}"
    );
    assert!(
        !hits.is_empty(),
        "root search must return the seeded document (A5 bypass), got: {hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// REGRESSION — plain non-member (no admin flags) is UNCHANGED
// ---------------------------------------------------------------------------

/// A plain non-member still gets 404 when trying to access a workspace
/// they are not a member of.
#[tokio::test]
async fn plain_non_member_gets_404_on_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, _) = support::seed_workspace(&db, "byp-reg-nm-owner").await;
    let (plain_client, _, _) =
        support::login_user_with_workspace(&server, &db, "byp-reg-nm-plain").await;

    let result = plain_client.get_workspace(&target_ws.slug).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "plain non-member must still get 404, got: {result:?}"
    );

    db.teardown().await;
}

/// A plain non-member still gets 404 on search.
#[tokio::test]
async fn plain_non_member_search_gets_404() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, owner) = support::seed_workspace(&db, "byp-reg-search-owner").await;
    let unique = "plain_nonmember_search_9z5";
    seed_document_in_ws(&db, &target_ws, &owner, "Non-member Doc", unique).await;

    let (plain_client, _, _) =
        support::login_user_with_workspace(&server, &db, "byp-reg-search-plain").await;
    let http = reqwest::Client::new();
    let token = plain_client.token().expect("plain token");
    let (status, _) = search_hits(&http, token, server.base_url(), &target_ws.slug, unique).await;

    assert_eq!(
        status, 404,
        "plain non-member must get 404 on search, not bypass; got status {status}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// REGRESSION — API key gets NO bypass (bypass must never be true for ApiKey)
// ---------------------------------------------------------------------------

/// An API key without any grant gets 404 on workspace search.
/// This proves WorkspaceAccess.bypass is never true for an ApiKey principal.
#[tokio::test]
async fn api_key_without_grants_search_gets_404() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, owner) = support::seed_workspace(&db, "byp-reg-apikey-search-owner").await;
    let unique = "apikey_nobypass_search_9z5";
    seed_document_in_ws(&db, &target_ws, &owner, "API Key Target Doc", unique).await;

    let (_key_id, raw_token) = create_api_key(&db, &target_ws, &owner, "nobypass-key").await;

    let http = reqwest::Client::new();
    let (status, _) = search_hits(
        &http,
        &raw_token,
        server.base_url(),
        &target_ws.slug,
        unique,
    )
    .await;

    assert_eq!(
        status, 404,
        "api_key without any grant must get 404, not bypass results (bypass must never be true for ApiKey); got: {status}"
    );

    db.teardown().await;
}

/// An API key without any grant gets 404 on the workspace route (WorkspaceMember extractor).
#[tokio::test]
async fn api_key_without_grants_gets_404_on_workspace_route() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (target_ws, owner) = support::seed_workspace(&db, "byp-reg-apikey-ws-owner").await;
    let (_key_id, raw_token) = create_api_key(&db, &target_ws, &owner, "nobypass-ws-key").await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{}/v1/workspaces/{}/members",
            server.base_url(),
            target_ws.slug
        ))
        .bearer_auth(&raw_token)
        .send()
        .await
        .expect("HTTP request");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "api_key without any grant must get 404 on workspace member list; got: {}",
        resp.status()
    );

    db.teardown().await;
}
