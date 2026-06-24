#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateProjectRequest,
    boards_tasks::{CreateBoardRequest, CreateColumnRequest, CreateTaskRequest},
};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::MemberRole,
    entities::permissions::NewPermissionGrant,
    ids::{BoardId, ProjectId, WorkspaceId},
    permissions::ResourceRole,
};
use atlas_server::persistence::repos::{
    MembershipRepo, NewUser, PermissionGrantRepo, PgMembershipRepo, PgPermissionGrantRepo, UserRepo,
};

// ---------------------------------------------------------------------------
// Seed helpers
// ---------------------------------------------------------------------------

fn mk_project_req(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: None,
        visibility_role: None,
    }
}

fn mk_project_req_private(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: Some("private".to_string()),
        visibility_role: None,
    }
}

fn mk_project_req_workspace(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: Some("workspace".to_string()),
        visibility_role: Some("viewer".to_string()),
    }
}

async fn seed_task(
    client: &atlas_client::AtlasClient,
    ws: &str,
    board_id: uuid::Uuid,
    column_id: uuid::Uuid,
    title: &str,
) -> atlas_api::dtos::boards_tasks::TaskDto {
    client
        .create_task(
            ws,
            board_id,
            CreateTaskRequest {
                column_id,
                title: title.to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task")
}

async fn seed_board(
    client: &atlas_client::AtlasClient,
    ws: &str,
    project_slug: &str,
    name: &str,
) -> atlas_api::dtos::boards_tasks::BoardDto {
    client
        .create_board(
            ws,
            project_slug,
            CreateBoardRequest {
                name: name.to_string(),
            },
        )
        .await
        .expect("create board")
}

async fn seed_column(
    client: &atlas_client::AtlasClient,
    ws: &str,
    board_id: uuid::Uuid,
    name: &str,
) -> atlas_api::dtos::boards_tasks::ColumnDto {
    client
        .create_column(
            ws,
            board_id,
            CreateColumnRequest {
                name: name.to_string(),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column")
}

async fn add_plain_member(
    db: &support::TestDb,
    server: &support::TestServer,
    ws_id: WorkspaceId,
    username: &str,
) -> atlas_client::AtlasClient {
    let hash = atlas_server::auth::password::hash("TestPassword1!".to_string())
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
    PgMembershipRepo {
        conn: db.conn().clone(),
    }
    .add(&ctx, user.id, MemberRole::Member)
    .await
    .expect("add membership");

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(atlas_api::dtos::LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    client
}

async fn grant_board(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    board_id: BoardId,
    user_id: atlas_domain::ids::UserId,
    role: ResourceRole,
) {
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws_id,
            user_id: Some(user_id),
            api_key_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: Some(board_id),
            role,
            created_by_user_id: None,
            created_by_api_key_id: None,
        })
        .await
        .expect("grant board");
}

async fn grant_project(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    project_id: ProjectId,
    user_id: atlas_domain::ids::UserId,
    role: ResourceRole,
) {
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws_id,
            user_id: Some(user_id),
            api_key_id: None,
            project_id: Some(project_id),
            folder_id: None,
            document_id: None,
            board_id: None,
            role,
            created_by_user_id: None,
            created_by_api_key_id: None,
        })
        .await
        .expect("grant project");
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/activity
// ---------------------------------------------------------------------------

/// TEST 1 — Privacy: a plain Member without a grant to a private project does NOT
/// see that project's task activity in the workspace feed.
#[tokio::test]
async fn workspace_activity_private_project_hidden_without_grant() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-priv1").await;

    let project = owner_client
        .create_project(&ws.slug, mk_project_req_private("priv-proj1", "PP1"))
        .await
        .expect("create project");

    let board = seed_board(&owner_client, &ws.slug, &project.slug, "Board 1").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    seed_task(&owner_client, &ws.slug, board.id, col.id, "secret task").await;

    let member_client = add_plain_member(&db, &server, ws.id, "member-priv1").await;

    let page = member_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("list activity");

    assert_eq!(
        page.items.len(),
        0,
        "member must not see private project activity"
    );
}

/// TEST 2 — Board grant: a board-only grant to board X in a private project P
/// makes that board's task activity visible even without a project grant.
#[tokio::test]
async fn workspace_activity_board_only_grant_surfaces_board_tasks() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "owner-board1").await;

    let project = owner_client
        .create_project(&ws.slug, mk_project_req_private("priv-board-proj1", "PBP"))
        .await
        .expect("create project");

    let board = seed_board(&owner_client, &ws.slug, &project.slug, "Secret Board").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    seed_task(
        &owner_client,
        &ws.slug,
        board.id,
        col.id,
        "board-granted task",
    )
    .await;

    let hash = atlas_server::auth::password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");
    let member_user = db
        .user_repo()
        .create(NewUser {
            username: "member-board1".to_string(),
            display_name: "member-board1".to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");
    support::activate_user_in_db(&db, member_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    PgMembershipRepo {
        conn: db.conn().clone(),
    }
    .add(&ctx, member_user.id, MemberRole::Member)
    .await
    .expect("add membership");

    grant_board(
        &db,
        ws.id,
        BoardId(board.id),
        member_user.id,
        ResourceRole::Viewer,
    )
    .await;

    let mut member_client = atlas_client::AtlasClient::new(server.base_url().to_string());
    member_client
        .login(atlas_api::dtos::LoginRequest {
            username: "member-board1".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    let page = member_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("list activity");

    assert_eq!(
        page.items.len(),
        1,
        "member with board grant must see 1 activity entry for that board's tasks"
    );
}

/// TEST 3 — Project grant: a project-level grant surfaces all of that project's
/// task activity.
#[tokio::test]
async fn workspace_activity_project_grant_surfaces_project_tasks() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "owner-proj1").await;

    let project = owner_client
        .create_project(&ws.slug, mk_project_req_private("proj-grant-1", "PG1"))
        .await
        .expect("create project");

    let board = seed_board(&owner_client, &ws.slug, &project.slug, "Board PG1").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    seed_task(&owner_client, &ws.slug, board.id, col.id, "granted task").await;

    let hash = atlas_server::auth::password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");
    let member_user = db
        .user_repo()
        .create(NewUser {
            username: "member-proj1".to_string(),
            display_name: "member-proj1".to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");
    support::activate_user_in_db(&db, member_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    PgMembershipRepo {
        conn: db.conn().clone(),
    }
    .add(&ctx, member_user.id, MemberRole::Member)
    .await
    .expect("add membership");

    grant_project(
        &db,
        ws.id,
        ProjectId(project.id),
        member_user.id,
        ResourceRole::Viewer,
    )
    .await;

    let mut member_client = atlas_client::AtlasClient::new(server.base_url().to_string());
    member_client
        .login(atlas_api::dtos::LoginRequest {
            username: "member-proj1".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    let page = member_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("list activity");

    assert_eq!(
        page.items.len(),
        1,
        "member with project grant must see activity for that project's tasks"
    );
}

/// TEST 4 — Project visibility: a workspace-visible project is visible to all
/// members; a private project without a grant is not.
#[tokio::test]
async fn workspace_activity_workspace_visible_project_visible_to_all_members() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-vis1").await;

    let pub_project = owner_client
        .create_project(&ws.slug, mk_project_req_workspace("vis-pub-proj1", "VPP"))
        .await
        .expect("create project");

    let board = seed_board(&owner_client, &ws.slug, &pub_project.slug, "Visible Board").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    seed_task(&owner_client, &ws.slug, board.id, col.id, "visible task").await;

    let priv_project = owner_client
        .create_project(&ws.slug, mk_project_req_private("vis-priv-proj1", "VPR"))
        .await
        .expect("create private project");

    let priv_board = seed_board(&owner_client, &ws.slug, &priv_project.slug, "Private Board").await;
    let priv_col = seed_column(&owner_client, &ws.slug, priv_board.id, "Todo").await;
    seed_task(
        &owner_client,
        &ws.slug,
        priv_board.id,
        priv_col.id,
        "hidden task",
    )
    .await;

    let member_client = add_plain_member(&db, &server, ws.id, "member-vis1").await;

    let page = member_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("list activity");

    assert_eq!(
        page.items.len(),
        1,
        "member must see only the workspace-visible project's task activity"
    );
    assert_eq!(
        page.items[0].task_readable_id, "VPP-1",
        "visible task readable_id must be correct"
    );
}

/// TEST 5 — Owner/Admin sees ALL workspace activity (is_admin bypass).
#[tokio::test]
async fn workspace_activity_owner_sees_all() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-all1").await;

    let priv_project = owner_client
        .create_project(&ws.slug, mk_project_req_private("priv-all-1", "PA1"))
        .await
        .expect("create project");

    let board = seed_board(&owner_client, &ws.slug, &priv_project.slug, "Private Board").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    seed_task(&owner_client, &ws.slug, board.id, col.id, "private task").await;

    let page = owner_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("list activity");

    assert!(
        !page.items.is_empty(),
        "owner must see all workspace activity; got {}",
        page.items.len()
    );
}

/// TEST 6 — api_key caller: a key with workspace-scope viewer grant sees activity
/// for workspace-visible projects.
#[tokio::test]
async fn workspace_activity_api_key_sees_granted_scope() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-apikey1").await;

    let vis_project = owner_client
        .create_project(&ws.slug, mk_project_req_workspace("vis-ak-proj1", "VAK"))
        .await
        .expect("create project");

    let vis_board = seed_board(&owner_client, &ws.slug, &vis_project.slug, "Vis Board").await;
    let vis_col = seed_column(&owner_client, &ws.slug, vis_board.id, "Todo").await;
    seed_task(
        &owner_client,
        &ws.slug,
        vis_board.id,
        vis_col.id,
        "vis task",
    )
    .await;

    let key_created = owner_client
        .create_user_api_key(atlas_api::dtos::CreateUserApiKeyRequest {
            name: "agent-ak1".to_string(),
            r#type: Some("agent".to_string()),
            expires_at: None,
            initial_grant: Some(atlas_api::dtos::InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "viewer".to_string(),
            }),
        })
        .await
        .expect("create api key");

    let api_key_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret.clone());

    let page = api_key_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("list activity");

    assert!(
        !page.items.is_empty(),
        "api_key with workspace-scope grant must see accessible activity; got {}",
        page.items.len()
    );
}

/// TEST 7 — actor filter: actor=user filters out api_key activity; actor=api_key
/// filters out user activity.
#[tokio::test]
async fn workspace_activity_actor_type_filter_works() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-af1").await;

    let project = owner_client
        .create_project(&ws.slug, mk_project_req("pub-af1", "AF1"))
        .await
        .expect("project");

    let board = seed_board(&owner_client, &ws.slug, &project.slug, "AF Board").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;

    seed_task(&owner_client, &ws.slug, board.id, col.id, "user task").await;

    let key_created = owner_client
        .create_user_api_key(atlas_api::dtos::CreateUserApiKeyRequest {
            name: "agent-af1".to_string(),
            r#type: Some("agent".to_string()),
            expires_at: None,
            initial_grant: Some(atlas_api::dtos::InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
        })
        .await
        .expect("create api key");

    let key_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret.clone());

    seed_task(&key_client, &ws.slug, board.id, col.id, "agent task").await;

    let user_page = owner_client
        .list_workspace_activity(&ws.slug, Some("user"), None, None, None)
        .await
        .expect("user filter");

    assert!(
        user_page.items.iter().all(|e| e.actor.r#type == "user"),
        "actor=user filter must return only user activities"
    );
    assert!(
        !user_page.items.is_empty(),
        "at least one user activity must appear"
    );

    let key_page = owner_client
        .list_workspace_activity(&ws.slug, Some("api_key"), None, None, None)
        .await
        .expect("api_key filter");

    assert!(
        key_page.items.iter().all(|e| e.actor.r#type == "api_key"),
        "actor=api_key filter must return only api_key activities"
    );
    assert!(
        !key_page.items.is_empty(),
        "at least one api_key activity must appear"
    );
}

/// TEST 8 — Date range filter: from/to bound by a.created_at.
#[tokio::test]
async fn workspace_activity_date_range_filter() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-date1").await;

    let project = owner_client
        .create_project(&ws.slug, mk_project_req("date-proj1", "DT1"))
        .await
        .expect("project");

    let board = seed_board(&owner_client, &ws.slug, &project.slug, "Date Board").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    seed_task(&owner_client, &ws.slug, board.id, col.id, "task 1").await;

    let far_future = "2099-01-01T00:00:00Z";
    let page = owner_client
        .list_workspace_activity(&ws.slug, None, Some(far_future), None, None)
        .await
        .expect("list with from");

    assert_eq!(page.items.len(), 0, "from=far_future must return 0 items");

    let far_past = "2000-01-01T00:00:00Z";
    let page2 = owner_client
        .list_workspace_activity(&ws.slug, None, None, Some(far_past), None)
        .await
        .expect("list with to");

    assert_eq!(page2.items.len(), 0, "to=far_past must return 0 items");

    let page3 = owner_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("all entries");

    assert!(
        !page3.items.is_empty(),
        "unfiltered must return at least 1 item"
    );
}

/// TEST 9 — Keyset pagination under filtering.
#[tokio::test]
async fn workspace_activity_keyset_pagination_under_filtering() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-page1").await;

    let vis_project = owner_client
        .create_project(&ws.slug, mk_project_req_workspace("page-vis-1", "PGV"))
        .await
        .expect("vis project");

    let vis_board = seed_board(&owner_client, &ws.slug, &vis_project.slug, "Vis Board").await;
    let vis_col = seed_column(&owner_client, &ws.slug, vis_board.id, "Todo").await;
    seed_task(
        &owner_client,
        &ws.slug,
        vis_board.id,
        vis_col.id,
        "accessible task A",
    )
    .await;
    seed_task(
        &owner_client,
        &ws.slug,
        vis_board.id,
        vis_col.id,
        "accessible task B",
    )
    .await;

    let priv_project = owner_client
        .create_project(&ws.slug, mk_project_req_private("page-priv-1", "PGP"))
        .await
        .expect("priv project");

    let priv_board = seed_board(&owner_client, &ws.slug, &priv_project.slug, "Priv Board").await;
    let priv_col = seed_column(&owner_client, &ws.slug, priv_board.id, "Todo").await;
    seed_task(
        &owner_client,
        &ws.slug,
        priv_board.id,
        priv_col.id,
        "inaccessible task C",
    )
    .await;

    let member_client = add_plain_member(&db, &server, ws.id, "member-page1").await;

    let page1 = member_client
        .list_workspace_activity(&ws.slug, None, None, None, Some(1))
        .await
        .expect("page 1");

    assert_eq!(
        page1.items.len(),
        1,
        "page 1 must return exactly 1 accessible entry"
    );
    assert!(page1.has_more, "page 1 must indicate has_more=true");
    assert!(
        page1.next_cursor.is_some(),
        "page 1 must have a next_cursor"
    );

    let page2 = member_client
        .list_workspace_activity_with_cursor(
            &ws.slug,
            None,
            None,
            page1.next_cursor.as_deref(),
            Some(1),
        )
        .await
        .expect("page 2");

    assert_eq!(
        page2.items.len(),
        1,
        "page 2 must return exactly 1 accessible entry"
    );
    assert!(!page2.has_more, "page 2 must indicate has_more=false");

    assert_ne!(
        page1.items[0].id, page2.items[0].id,
        "pages must not contain duplicate entries"
    );

    let all_readable_ids: std::collections::HashSet<String> = page1
        .items
        .iter()
        .chain(page2.items.iter())
        .map(|e| e.task_readable_id.clone())
        .collect();

    assert!(
        all_readable_ids.iter().all(|rid| rid.starts_with("PGV-")),
        "all paginated entries must belong to the visible project (PGV-*)"
    );
}

/// TEST 10 — Actor enrichment: workspace feed actors carry display_name and
/// account_status.
#[tokio::test]
async fn workspace_activity_actors_are_enriched() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-enrich1").await;

    let project = owner_client
        .create_project(&ws.slug, mk_project_req("enrich-proj1", "EN1"))
        .await
        .expect("project");

    let board = seed_board(&owner_client, &ws.slug, &project.slug, "Enrich Board").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    seed_task(&owner_client, &ws.slug, board.id, col.id, "enriched task").await;

    let page = owner_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("list activity");

    assert!(!page.items.is_empty(), "must have at least one entry");
    for entry in &page.items {
        assert!(
            entry.actor.display_name.is_some(),
            "actor display_name must be populated; id={}",
            entry.actor.id
        );
        assert!(
            entry.actor.account_status.is_some(),
            "user actor must carry account_status; id={}",
            entry.actor.id
        );
    }
}

/// TEST 11 — Per-task feed regression: the existing /activity endpoint returns
/// enriched actors and the two new DTO fields (task_id, task_readable_id).
#[tokio::test]
async fn per_task_activity_actors_are_enriched_and_dto_has_new_fields() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-reg1").await;

    let project = owner_client
        .create_project(&ws.slug, mk_project_req("reg-proj1", "RG1"))
        .await
        .expect("project");

    let board = seed_board(&owner_client, &ws.slug, &project.slug, "Reg Board").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    let task = seed_task(&owner_client, &ws.slug, board.id, col.id, "reg task").await;

    let page = owner_client
        .list_activity(&ws.slug, &task.readable_id)
        .await
        .expect("list per-task activity");

    assert!(
        !page.items.is_empty(),
        "per-task feed must have at least one entry"
    );
    for entry in &page.items {
        assert_eq!(
            entry.task_id, task.id,
            "per-task entry must have the task's id"
        );
        assert_eq!(
            entry.task_readable_id, task.readable_id,
            "per-task entry must have task_readable_id"
        );
        if entry.actor.r#type == "user" {
            assert!(
                entry.actor.display_name.is_some(),
                "user actor must have display_name"
            );
            assert!(
                entry.actor.account_status.is_some(),
                "user actor must have account_status"
            );
        }
    }
}

/// TEST 12 — DTO additive: task_readable_id and task_id are correct in the
/// workspace feed.
#[tokio::test]
async fn workspace_activity_dto_includes_task_readable_id() {
    let db = support::TestDb::create().await.expect("db");
    let server = support::TestServer::spawn(&db).await;

    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "owner-dto1").await;

    let project = owner_client
        .create_project(&ws.slug, mk_project_req("dto-proj1", "DTO"))
        .await
        .expect("project");

    let board = seed_board(&owner_client, &ws.slug, &project.slug, "DTO Board").await;
    let col = seed_column(&owner_client, &ws.slug, board.id, "Todo").await;
    let task = seed_task(&owner_client, &ws.slug, board.id, col.id, "dto task").await;

    let page = owner_client
        .list_workspace_activity(&ws.slug, None, None, None, None)
        .await
        .expect("list activity");

    assert!(!page.items.is_empty(), "must have at least one entry");
    let entry = &page.items[0];
    assert_eq!(
        entry.task_id, task.id,
        "workspace feed entry must carry the correct task_id"
    );
    assert_eq!(
        entry.task_readable_id, task.readable_id,
        "workspace feed entry must carry the correct task_readable_id"
    );
}
