//! Permission integration test matrix for `GET /v1/workspaces/{ws}/search`.
//!
//! Covers:
//! - API-key principal access: no-grant → 404 at route gate; ws-scope grant → sees rows;
//!   per-document grant sees only the granted doc. Guards the invariant that
//!   `membership_clause` is FALSE for ApiKey principals (grants are the only access path).
//! - Cross-tenant task isolation: a task in workspace B must never appear in searches
//!   by a workspace A principal.
//! - Cross-tenant document isolation (redundant with repo tests but proves the HTTP route).
//! - Intra-workspace: workspace owner sees documents and tasks; non-member gets 404.
//! - Workspace-scope grant does not leak cross-tenant documents to the grantee.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::search::SearchHitDto;
use atlas_api::pagination::Page;
use atlas_domain::permissions::Principal;
use atlas_domain::ports::search::SearchRepo;
use atlas_domain::search::{SearchQuery, SearchSort, TypeSet};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        documents::NewDocument,
        identity::MemberRole,
        permissions::NewPermissionGrant,
        workspace_core::NewProject,
    },
    permissions::{ResourceRole, Visibility, VisibilityRole},
};
use atlas_server::{
    auth::tokens::{generate_api_key, hash_token},
    persistence::repos::{
        ApiKeyRepo, BoardRepo, DocumentRepo, MembershipRepo, NewApiKey, NewUser,
        PermissionGrantRepo, PgApiKeyRepo, PgBoardRepo, PgDocumentRepo, PgPermissionGrantRepo,
        PgProjectRepo, PgSearchRepo, PgTaskRepo, ProjectRepo, TaskRepo, UserRepo,
    },
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn search_url(base: &str, ws: &str, q: &str) -> String {
    format!("{base}/v1/workspaces/{ws}/search?q={q}")
}

async fn get_search_with_token(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    ws: &str,
    q: &str,
) -> reqwest::Response {
    http.get(search_url(base, ws, q))
        .bearer_auth(token)
        .send()
        .await
        .expect("HTTP request")
}

async fn search_ids(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    ws_slug: &str,
    q: &str,
) -> Vec<Uuid> {
    let resp = get_search_with_token(http, token, base, ws_slug, q).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "expected 200 for search, got {:?}",
        resp.status()
    );
    let page: Page<SearchHitDto> = resp.json().await.expect("decode page");
    page.items.iter().map(|h| h.id).collect()
}

/// Seeds a workspace with an owner user and returns the workspace record and user.
async fn seed_workspace_with_member(
    db: &support::TestDb,
    username: &str,
) -> (
    atlas_server::persistence::repos::Workspace,
    atlas_server::persistence::repos::User,
) {
    support::seed_workspace(db, username).await
}

/// Grants an API key workspace-scope access.
async fn grant_ws_scope_for_key(
    db: &support::TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    key_id: atlas_domain::ids::ApiKeyId,
    grantor_id: atlas_domain::ids::UserId,
) {
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: None,
        api_key_id: Some(key_id),
        group_id: None,
        project_id: None,
        folder_id: None,
        document_id: None,
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: Some(grantor_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant ws-scope for key");
}

/// Grants an API key a per-document grant (document_id-scoped, all other resource ids NULL).
///
/// This is the document-level grant arm of the permission disjunction. A key with only
/// this grant must see the specific document and no other same-workspace document.
async fn grant_doc_for_key(
    db: &support::TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    key_id: atlas_domain::ids::ApiKeyId,
    doc_id: atlas_domain::ids::DocumentId,
    grantor_id: atlas_domain::ids::UserId,
) {
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: None,
        api_key_id: Some(key_id),
        group_id: None,
        project_id: None,
        folder_id: None,
        document_id: Some(doc_id),
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: Some(grantor_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant doc-scope for key");
}

/// Creates an API key for a workspace and returns (key_id, raw_token).
async fn create_api_key_for_ws(
    db: &support::TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    creator_id: atlas_domain::ids::UserId,
    name: &str,
) -> (atlas_domain::ids::ApiKeyId, String) {
    let raw_token = generate_api_key();
    let token_hash = hash_token(&raw_token);

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(creator_id));
    let key = PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewApiKey {
            name: name.to_string(),
            token_hash,
            type_: atlas_domain::entities::identity::ApiKeyType::Agent,
            expires_at: None,
            scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
        },
    )
    .await
    .expect("create api key");

    (key.id, raw_token)
}

async fn seed_document(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    title: &str,
    content: &str,
) -> atlas_domain::ids::DocumentId {
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let doc = repo
        .create(
            ctx,
            NewDocument {
                title: title.to_string(),
                slug: None,
                content: content.to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");
    doc.id
}

async fn seed_task_with_board(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    project_slug: &str,
    task_prefix: &str,
    title: &str,
    description: &str,
) -> (
    atlas_domain::ids::TaskId,
    atlas_domain::ids::BoardId,
    atlas_domain::ids::ProjectId,
) {
    let project_repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    let board_repo = PgBoardRepo::new(db.conn().clone());

    let project = project_repo
        .create(
            ctx,
            NewProject {
                name: format!("Project {project_slug}"),
                slug: project_slug.to_string(),
                task_prefix: task_prefix.to_string(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("seed project");

    let board = board_repo
        .create_board(
            ctx,
            NewBoard {
                project_id: project.id,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("seed board");

    let col = board_repo
        .add_column(
            ctx,
            board.id,
            "Backlog".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("seed column");

    let task_repo = PgTaskRepo::new(db.conn().clone());
    let task = task_repo
        .create(
            ctx,
            NewTask {
                column_id: col.id,
                board_id: board.id,
                project_id: project.id,
                title: title.to_string(),
                description: description.to_string(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("seed task");

    (task.id, board.id, project.id)
}

// ---------------------------------------------------------------------------
// API-key principal: no-grant → 404; with grant → sees rows
// ---------------------------------------------------------------------------

/// An API key with NO grants is rejected at the workspace gate (404).
///
/// `membership_clause` is FALSE for ApiKey principals, so the route's
/// `Authorized<WorkspaceRes, ViewerMin>` extractor finds no qualifying
/// membership or grant and returns 404 (workspace not visible — never 403, per
/// cross-tenant concealment policy). Guards the invariant that an unganted
/// api-key cannot bypass the outer authorization gate.
#[tokio::test]
async fn api_key_without_grant_gets_404() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (ws, owner) = seed_workspace_with_member(&db, "b1-nokey-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let unique = "apikeynogrnt9a";
    let _ = seed_document(&db, &ctx, &format!("Doc {unique}"), unique).await;

    // Create an API key for the same workspace — no grants given.
    let (_, raw_token) = create_api_key_for_ws(&db, ws.id, owner.id, "test-key-no-grant").await;

    let http = reqwest::Client::new();
    let resp = get_search_with_token(&http, &raw_token, server.base_url(), &ws.slug, unique).await;

    assert_eq!(
        resp.status().as_u16(),
        404,
        "api-key without any grant must get 404 (workspace not visible); got {:?}",
        resp.status()
    );

    db.teardown().await;
}

/// An API key with a workspace-scope grant sees resources it has been granted.
///
/// Guards the invariant that the grant arm of the SQL predicate fires correctly
/// for `Principal::ApiKey`, surfacing rows that have an explicit grant.
#[tokio::test]
async fn api_key_with_ws_scope_grant_sees_granted_resources() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (ws, owner) = seed_workspace_with_member(&db, "b1-wsgrant-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let unique = "apikeywsgrant9b";
    let doc_id = seed_document(&db, &ctx, &format!("Doc {unique}"), unique).await;

    let (key_id, raw_token) =
        create_api_key_for_ws(&db, ws.id, owner.id, "test-key-ws-grant").await;

    // Grant workspace-scope access to the API key.
    grant_ws_scope_for_key(&db, ws.id, key_id, owner.id).await;

    let http = reqwest::Client::new();
    let ids = search_ids(&http, &raw_token, server.base_url(), &ws.slug, unique).await;

    assert!(
        ids.contains(&doc_id.0),
        "api-key with ws-scope grant must see the document; got: {ids:?}"
    );

    db.teardown().await;
}

/// An API key with workspace-scope grant sees docs in its workspace but NOT those
/// in another workspace — even when searching the same query term.
///
/// Guards the invariant that the `workspace_id = $1` constraint in the SQL
/// permission predicate prevents cross-tenant document leakage for api-key
/// principals, whose only access path is through explicit grants.
#[tokio::test]
async fn api_key_ws_scope_grant_does_not_leak_cross_tenant() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (ws_a, owner_a) = seed_workspace_with_member(&db, "b1-xten-a").await;
    let ctx_a = support::ctx(&ws_a, &owner_a);

    let (ws_b, owner_b) = seed_workspace_with_member(&db, "b1-xten-b").await;
    let ctx_b = support::ctx(&ws_b, &owner_b);

    let unique = "apikey_xten9c";
    let doc_a_id = seed_document(&db, &ctx_a, &format!("Doc {unique}"), unique).await;
    let _ = seed_document(&db, &ctx_b, &format!("Doc {unique}"), unique).await;

    // Create a key in workspace A with ws-scope grant.
    let (key_id, raw_token) = create_api_key_for_ws(&db, ws_a.id, owner_a.id, "key-a").await;
    grant_ws_scope_for_key(&db, ws_a.id, key_id, owner_a.id).await;

    let http = reqwest::Client::new();
    let ids = search_ids(&http, &raw_token, server.base_url(), &ws_a.slug, unique).await;

    assert!(
        ids.contains(&doc_a_id.0),
        "api-key must see workspace A document; got: {ids:?}"
    );
    assert_eq!(
        ids.iter().filter(|&&id| id == doc_a_id.0).count(),
        ids.len(),
        "api-key must NOT see workspace B docs — extra ids are cross-tenant leaks; got: {ids:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Cross-tenant task isolation
// ---------------------------------------------------------------------------

/// A task in workspace B must NEVER appear in a search by workspace A's principal.
///
/// Guards the invariant that the tasks arm's `workspace_id = $1` constraint
/// prevents cross-tenant leakage for tasks, mirroring the same guard on the
/// documents arm.
#[tokio::test]
async fn cross_tenant_task_isolation() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (ws_a, alice) = seed_workspace_with_member(&db, "b3-alice").await;
    let ctx_a = support::ctx(&ws_a, &alice);

    let (ws_b, bob) = seed_workspace_with_member(&db, "b3-bob").await;
    let ctx_b = support::ctx(&ws_b, &bob);

    // Log in as alice to get a session token.
    let _ = support::login_user_with_workspace(&server, &db, "b3-alice-login").await;
    // seed_workspace inside login_user_with_workspace creates a NEW workspace, but we need alice's ws_a.
    // Use a separate login approach: log in alice directly via HTTP after seeding her with a real pw.
    let alice_token = {
        use atlas_api::dtos::LoginRequest;
        use atlas_server::auth::password;
        let pw = "TestPassword1!";
        let hash = password::hash(pw.to_string()).await.expect("hash");

        let user = db
            .user_repo()
            .create(NewUser {
                username: "b3-alice-auth".to_string(),
                display_name: "Alice".to_string(),
                email: None,
                password_hash: Some(hash),
                is_root: false,
                is_system_admin: false,
            })
            .await
            .expect("create alice");

        support::activate_user_in_db(&db, user.id.0).await;

        let member_ctx = WorkspaceCtx::new(ws_a.id, Actor::User(user.id));
        db.membership_repo()
            .add(&member_ctx, user.id, MemberRole::Owner)
            .await
            .expect("add membership");

        let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
        client
            .login(LoginRequest {
                username: "b3-alice-auth".to_string(),
                password: pw.to_string(),
            })
            .await
            .expect("login");
        client.token().expect("token").to_string()
    };

    let unique = "crosstenant_task_b3";

    // Bob's workspace has a task with this unique token.
    let _ = seed_task_with_board(
        &db,
        &ctx_b,
        "b3-proj-bob",
        "B3B",
        &format!("Task {unique}"),
        unique,
    )
    .await;

    // Alice's workspace also has a task — she should see only hers.
    let (alice_task_id, _, _) = seed_task_with_board(
        &db,
        &ctx_a,
        "b3-proj-alice",
        "B3A",
        &format!("Task {unique}"),
        unique,
    )
    .await;

    let http = reqwest::Client::new();
    let ids = search_ids(&http, &alice_token, server.base_url(), &ws_a.slug, unique).await;

    assert!(
        ids.contains(&alice_task_id.0),
        "alice must see her own task; got: {ids:?}"
    );
    assert_eq!(
        ids.iter().filter(|&&id| id == alice_task_id.0).count(),
        ids.len(),
        "alice must see ONLY her task — any extra id is a cross-tenant leak; got: {ids:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Additional permission scenarios (document + task, intra-workspace)
// ---------------------------------------------------------------------------

/// A workspace member (owner of the workspace) sees their own documents.
#[tokio::test]
async fn workspace_member_sees_own_documents() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "perm-member-own").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let token = client.token().expect("token");

    let unique = "perm_member_own9d";
    let doc_id = seed_document(&db, &ctx, &format!("Doc {unique}"), unique).await;

    let http = reqwest::Client::new();
    let ids = search_ids(&http, token, server.base_url(), &ws.slug, unique).await;

    assert!(
        ids.contains(&doc_id.0),
        "workspace member must see their own document; got: {ids:?}"
    );

    db.teardown().await;
}

/// A user who is NOT a member of a workspace and has no grant sees nothing.
#[tokio::test]
async fn non_member_without_grant_sees_nothing() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (ws, owner) = seed_workspace_with_member(&db, "perm-nomem-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let unique = "perm_nomem9e";
    let _ = seed_document(&db, &ctx, &format!("Doc {unique}"), unique).await;

    // Log in a completely different user who has their own workspace.
    let (stranger_client, _, _) =
        support::login_user_with_workspace(&server, &db, "perm-nomem-stranger").await;
    let token = stranger_client.token().expect("token");

    let http = reqwest::Client::new();
    // The stranger tries to search ws_a (the owner's workspace). They should get 404 (not a member).
    let resp = get_search_with_token(&http, token, server.base_url(), &ws.slug, unique).await;
    assert_eq!(
        resp.status().as_u16(),
        404,
        "non-member must get 404 for a workspace they are not in"
    );

    db.teardown().await;
}

/// A workspace owner sees tasks in their workspace via the membership clause.
///
/// The task permission disjunction includes `membership_clause` (for User principals),
/// which means any workspace member sees all tasks. This test confirms that an
/// authenticated workspace owner can search and see tasks, while a user from a
/// different workspace (no membership, no grant) gets 404.
#[tokio::test]
async fn task_visible_to_member_not_to_outsider() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    // The workspace owner: logged in so they can hit the authenticated route.
    let (owner_client, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "perm-board-owner").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    let owner_token = owner_client.token().expect("owner token").to_string();

    let unique = "perm_board9f";
    let (task_id, _, _) = seed_task_with_board(
        &db,
        &ctx,
        "perm-board-proj",
        "PBP",
        &format!("Task {unique}"),
        unique,
    )
    .await;

    // A completely unrelated user in a different workspace cannot access the search.
    let (outsider_client, _, _) =
        support::login_user_with_workspace(&server, &db, "perm-board-outsider").await;
    let outsider_token = outsider_client.token().expect("outsider token").to_string();

    let http = reqwest::Client::new();

    let member_ids = search_ids(&http, &owner_token, server.base_url(), &ws.slug, unique).await;
    assert!(
        member_ids.contains(&task_id.0),
        "workspace owner must see tasks via membership_clause; got: {member_ids:?}"
    );

    // The outsider is a member of their own workspace, not ws. They get 404.
    let outsider_resp =
        get_search_with_token(&http, &outsider_token, server.base_url(), &ws.slug, unique).await;
    assert_eq!(
        outsider_resp.status().as_u16(),
        404,
        "user not in workspace must get 404; got {:?}",
        outsider_resp.status()
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Discriminating: api-key `membership_clause` must be FALSE, not TRUE.
//
// This test calls PgSearchRepo::search() directly rather than via HTTP because
// the route gate blocks an api-key with no workspace-scope grant before the SQL
// ever runs. If we instead gave the key a ws-scope grant, that grant arm itself
// would surface every row, making `membership_clause` irrelevant and the test
// vacuous.
//
// With `membership_clause = FALSE` (correct) and no ws-scope grant, only the
// per-doc grant arm fires. If `membership_clause` were regressed to TRUE, the
// first arm of the disjunction would surface every row in the workspace.
// ---------------------------------------------------------------------------

/// An API key with no ws-scope grant sees only explicitly granted rows at the SQL level.
///
/// Guards the invariant that `membership_clause` for `Principal::ApiKey` in
/// `PgSearchRepo::search` is FALSE — any regression to TRUE would cause every
/// workspace row to surface regardless of grants.
#[tokio::test]
async fn api_key_no_grant_sees_no_rows_at_sql_level() {
    let db = support::TestDb::create().await.expect("TestDb");

    let (ws, owner) = seed_workspace_with_member(&db, "b1-disc-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let unique = "b1discrimtoken9h";

    // Seed two documents that both match the query term.
    let granted_doc_id = seed_document(&db, &ctx, &format!("Granted {unique}"), unique).await;
    let _ = seed_document(&db, &ctx, &format!("Ungranted {unique}"), unique).await;

    // Create an api-key in the workspace. Grant it a per-document grant on
    // granted_doc ONLY — no workspace-scope grant. At the route level this key
    // would get 404 (no ws-scope grant), so we call the repo directly.
    let (key_id, _raw_token) = create_api_key_for_ws(&db, ws.id, owner.id, "key-disc-b1").await;

    grant_doc_for_key(&db, ws.id, key_id, granted_doc_id, owner.id).await;

    // Call PgSearchRepo::search() directly. The api-key has a per-doc grant on
    // granted_doc and no ws-scope grant. With membership_clause = "FALSE":
    //   - FALSE (membership arm)
    //   - OR ws-scope grant arm: no ws-scope grant exists → FALSE
    //   - OR per-doc grant arm on granted_doc.id: fires for granted_doc → TRUE
    //   - ungranted_doc has no matching per-doc grant → FALSE
    //
    // Expected result: exactly {granted_doc_id}, NOT ungranted_doc.
    // If membership_clause were "TRUE", both docs would surface (TRUE OR ...).
    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::ApiKey(key_id);
    let query = SearchQuery {
        text: unique.to_string(),
        filters: vec![],
        sort: SearchSort::Relevance,
        type_filter: TypeSet::all(),
        warnings: vec![],
        prefix: false,
    };
    let hits = repo
        .search(&ctx, &principal, &query, 50, None, false)
        .await
        .expect("search");

    let ids: Vec<Uuid> = hits.iter().map(|h| h.id).collect();

    assert!(
        ids.contains(&granted_doc_id.0),
        "api-key with per-doc grant must see the granted document; got: {ids:?}"
    );
    assert_eq!(
        ids.len(),
        1,
        "api-key with per-doc grant must NOT see ungranted documents — \
         any extra id indicates membership_clause is over-broad (not FALSE); got: {ids:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Gate: any principal with workspace access can reach search.
//
// The search route admits any principal with access to the workspace: a member
// (any MemberRole) OR a principal holding at least one grant anywhere in the
// workspace. A principal with no membership and no grant gets 404 (concealment).
// ---------------------------------------------------------------------------

/// Grants a user a project-scope grant.
async fn grant_project_for_user(
    db: &support::TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    user_id: atlas_domain::ids::UserId,
    project_id: atlas_domain::ids::ProjectId,
    grantor_id: atlas_domain::ids::UserId,
) {
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: Some(user_id),
        api_key_id: None,
        group_id: None,
        project_id: Some(project_id),
        folder_id: None,
        document_id: None,
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: Some(grantor_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant project for user");
}

/// A user who is NOT a member but holds a project-scope grant can reach search
/// and sees ONLY that project's resources — nothing from other projects.
#[tokio::test]
async fn project_grant_only_user_can_search_and_is_scoped() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (ws, owner) = seed_workspace_with_member(&db, "gate-projgrant-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let unique = "gateprojgrant9k";

    // Granted project: a doc the grantee should see.
    let (granted_task_id, _granted_board, granted_project) = seed_task_with_board(
        &db,
        &ctx_owner,
        "gate-granted-proj",
        "GGP",
        &format!("Granted Task {unique}"),
        unique,
    )
    .await;

    // Other project: a task the grantee must NOT see.
    let (other_task_id, _other_board, _other_project) = seed_task_with_board(
        &db,
        &ctx_owner,
        "gate-other-proj",
        "GOP",
        &format!("Other Task {unique}"),
        unique,
    )
    .await;

    // A grantee who is not a member of the workspace, with a project grant only.
    let (grantee_client, grantee) =
        support::login_user(&server, &db, "gate-projgrant-grantee").await;
    grant_project_for_user(&db, ws.id, grantee.id, granted_project, owner.id).await;

    let token = grantee_client.token().expect("token");
    let http = reqwest::Client::new();
    let ids = search_ids(&http, token, server.base_url(), &ws.slug, unique).await;

    assert!(
        ids.contains(&granted_task_id.0),
        "project-grant-only user must see the granted project's task; got: {ids:?}"
    );
    assert!(
        !ids.contains(&other_task_id.0),
        "project-grant-only user must NOT see other projects' tasks; got: {ids:?}"
    );

    db.teardown().await;
}

/// A plain member (no workspace-scope grant, not Owner/Admin) can reach search.
#[tokio::test]
async fn plain_member_can_reach_search() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (ws, owner) = seed_workspace_with_member(&db, "gate-plainmember-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let unique = "gateplainmember9l";

    // A Workspace-visibility project task the plain member should see.
    let (task_id, _board, _project) = seed_task_with_board(
        &db,
        &ctx_owner,
        "gate-plainmember-proj",
        "GPM",
        &format!("Member Task {unique}"),
        unique,
    )
    .await;

    let (member_client, member) =
        support::login_user(&server, &db, "gate-plainmember-member").await;
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    db.membership_repo()
        .add(&ctx, member.id, MemberRole::Member)
        .await
        .expect("add plain member");

    let token = member_client.token().expect("token");
    let http = reqwest::Client::new();
    let resp = get_search_with_token(&http, token, server.base_url(), &ws.slug, unique).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "a plain member must be able to reach search; got {:?}",
        resp.status()
    );

    let ids = search_ids(&http, token, server.base_url(), &ws.slug, unique).await;
    assert!(
        ids.contains(&task_id.0),
        "a plain member must see a Workspace-visibility project task; got: {ids:?}"
    );

    db.teardown().await;
}

/// Cross-tenant document isolation via the HTTP route.
///
/// Proved at repo level in search_repo.rs; this test also covers the HTTP surface.
#[tokio::test]
async fn cross_tenant_document_isolation_via_http() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (alice_client, ws_a, user_a) =
        support::login_user_with_workspace(&server, &db, "perm-ctdoc-alice").await;
    let ctx_a = WorkspaceCtx::new(ws_a.id, Actor::User(user_a.id));

    let (_, ws_b, user_b) =
        support::login_user_with_workspace(&server, &db, "perm-ctdoc-bob").await;
    let ctx_b = WorkspaceCtx::new(ws_b.id, Actor::User(user_b.id));

    let unique = "perm_ctdoc9g";
    let alice_doc_id = seed_document(&db, &ctx_a, &format!("Doc {unique}"), unique).await;
    let _ = seed_document(&db, &ctx_b, &format!("Doc {unique}"), unique).await;

    let token = alice_client.token().expect("token");
    let http = reqwest::Client::new();
    let ids = search_ids(&http, token, server.base_url(), &ws_a.slug, unique).await;

    assert!(
        ids.contains(&alice_doc_id.0),
        "alice must see her own document; got: {ids:?}"
    );
    assert_eq!(
        ids.iter().filter(|&&id| id == alice_doc_id.0).count(),
        ids.len(),
        "alice must see ONLY her document — extra ids are cross-tenant leaks; got: {ids:?}"
    );

    db.teardown().await;
}
