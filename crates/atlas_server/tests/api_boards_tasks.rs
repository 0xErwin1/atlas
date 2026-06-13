#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateProjectRequest,
    boards_tasks::{
        AddAssigneeRequest, CreateBoardRequest, CreateChecklistItemRequest, CreateColumnRequest,
        CreateReferenceRequest, CreateTaskRequest, MoveTaskRequest, PromoteChecklistItemRequest,
        UpdateBoardRequest, UpdateColumnRequest, UpdateTaskRequest,
    },
};
use atlas_client::ClientError;
use atlas_domain::{
    Actor, WorkspaceCtx, entities::identity::MemberRole, entities::permissions::NewPermissionGrant,
    ids::BoardId, permissions::ResourceRole,
};
use atlas_server::persistence::repos::{
    MembershipRepo, NewUser, PermissionGrantRepo, PgPermissionGrantRepo, UserRepo,
};

fn project_req(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: None,
        visibility_role: None,
    }
}

/// Creates and logs in a second user with the given membership role in `ws`.
async fn add_member(
    db: &support::TestDb,
    server: &support::TestServer,
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

// ---------------------------------------------------------------------------
// Board happy-path CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_get_board_returns_201_and_board_data() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "board-crud-1").await;

    client
        .create_project(&ws.slug, project_req("board-proj-1", "BP1"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "board-proj-1",
            CreateBoardRequest {
                name: "Sprint 1".to_string(),
            },
        )
        .await
        .expect("create board");

    assert_eq!(board.name, "Sprint 1");
    assert_eq!(board.workspace_id, ws.id.0);

    let fetched = client
        .get_board(&ws.slug, board.id)
        .await
        .expect("get board");

    assert_eq!(fetched.id, board.id);
    assert_eq!(fetched.name, "Sprint 1");

    db.teardown().await;
}

#[tokio::test]
async fn list_boards_returns_created_boards() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "board-list-1").await;

    client
        .create_project(&ws.slug, project_req("board-list-proj", "BL"))
        .await
        .expect("create project");

    client
        .create_board(
            &ws.slug,
            "board-list-proj",
            CreateBoardRequest {
                name: "A".to_string(),
            },
        )
        .await
        .expect("create board A");
    client
        .create_board(
            &ws.slug,
            "board-list-proj",
            CreateBoardRequest {
                name: "B".to_string(),
            },
        )
        .await
        .expect("create board B");

    let page = client
        .list_boards(&ws.slug, "board-list-proj", None, None)
        .await
        .expect("list boards");

    assert_eq!(page.items.len(), 2, "must list 2 boards");

    db.teardown().await;
}

#[tokio::test]
async fn update_board_renames_it() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "board-upd-1").await;

    client
        .create_project(&ws.slug, project_req("board-upd-proj", "BU"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "board-upd-proj",
            CreateBoardRequest {
                name: "Old".to_string(),
            },
        )
        .await
        .expect("create board");

    let updated = client
        .update_board(
            &ws.slug,
            board.id,
            UpdateBoardRequest {
                name: Some("New".to_string()),
            },
        )
        .await
        .expect("update board");

    assert_eq!(updated.name, "New");

    db.teardown().await;
}

#[tokio::test]
async fn delete_board_makes_it_invisible() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "board-del-1").await;

    client
        .create_project(&ws.slug, project_req("board-del-proj", "BD"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "board-del-proj",
            CreateBoardRequest {
                name: "Del".to_string(),
            },
        )
        .await
        .expect("create board");

    client
        .delete_board(&ws.slug, board.id)
        .await
        .expect("delete board");

    let result = client.get_board(&ws.slug, board.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "deleted board must return 404, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Column happy-path CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_list_columns() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "col-crud-1").await;

    client
        .create_project(&ws.slug, project_req("col-proj-1", "CP"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-proj-1",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    assert_eq!(col.name, "Todo");

    let cols = client
        .list_columns(&ws.slug, board.id)
        .await
        .expect("list columns");

    assert_eq!(cols.len(), 1, "board must have one column");
    assert_eq!(cols[0].id, col.id);

    db.teardown().await;
}

#[tokio::test]
async fn update_column_renames_it() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "col-upd-1").await;

    client
        .create_project(&ws.slug, project_req("col-upd-proj", "CU"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-upd-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Backlog".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let updated = client
        .update_column(
            &ws.slug,
            board.id,
            col.id,
            UpdateColumnRequest {
                name: Some("In Progress".to_string()),
                before: None,
                after: None,
            },
        )
        .await
        .expect("update column");

    assert_eq!(updated.name, "In Progress");

    db.teardown().await;
}

#[tokio::test]
async fn delete_column_removes_it_from_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "col-del-1").await;

    client
        .create_project(&ws.slug, project_req("col-del-proj", "CD"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-del-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Bye".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    client
        .delete_column(&ws.slug, board.id, col.id)
        .await
        .expect("delete column");

    let cols = client
        .list_columns(&ws.slug, board.id)
        .await
        .expect("list columns after delete");

    assert!(
        !cols.iter().any(|c| c.id == col.id),
        "deleted column must not appear in list"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Task happy-path CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_get_task_returns_201_and_task_data() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-crud-1").await;

    client
        .create_project(&ws.slug, project_req("task-proj-1", "TK"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-proj-1",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "First Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    assert_eq!(task.title, "First Task");
    assert!(
        task.readable_id.starts_with("TK-"),
        "readable_id must use prefix"
    );

    let fetched = client
        .get_task(&ws.slug, &task.readable_id)
        .await
        .expect("get task");

    assert_eq!(fetched.id, task.id);
    assert_eq!(fetched.title, "First Task");

    db.teardown().await;
}

#[tokio::test]
async fn list_tasks_returns_tasks_for_board() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-list-1").await;

    client
        .create_project(&ws.slug, project_req("task-list-proj", "TL"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-list-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "T1".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task 1");

    client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "T2".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task 2");

    let page = client
        .list_tasks(&ws.slug, board.id, None, None)
        .await
        .expect("list tasks");

    assert_eq!(page.items.len(), 2, "board must have 2 tasks");

    db.teardown().await;
}

#[tokio::test]
async fn update_task_changes_title() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-upd-1").await;

    client
        .create_project(&ws.slug, project_req("task-upd-proj", "TU"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-upd-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Original".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let updated = client
        .update_task(
            &ws.slug,
            &task.readable_id,
            UpdateTaskRequest {
                title: Some("Renamed".to_string()),
                description: None,
                priority: None,
                due_date: None,
                estimate: None,
                labels: None,
                properties: None,
            },
        )
        .await
        .expect("update task");

    assert_eq!(updated.title, "Renamed");

    db.teardown().await;
}

#[tokio::test]
async fn delete_task_returns_404_on_subsequent_get() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-del-1").await;

    client
        .create_project(&ws.slug, project_req("task-del-proj", "TD"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-del-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Delete Me".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    client
        .delete_task(&ws.slug, &task.readable_id)
        .await
        .expect("delete task");

    let result = client.get_task(&ws.slug, &task.readable_id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "deleted task must return 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn move_task_changes_column() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-move-1").await;

    client
        .create_project(&ws.slug, project_req("task-move-proj", "MV"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-move-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col_a = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create col A");

    let col_b = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Done".to_string(),
                before: Some(col_a.position_key.clone()),
                after: None,
            },
        )
        .await
        .expect("create col B");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col_a.id,
                title: "Move Me".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let moved = client
        .move_task(
            &ws.slug,
            &task.readable_id,
            MoveTaskRequest {
                column_id: col_b.id,
                before: None,
                after: None,
            },
        )
        .await
        .expect("move task");

    assert_eq!(
        moved.column_id, col_b.id,
        "task must be in col_b after move"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Assignees
// ---------------------------------------------------------------------------

#[tokio::test]
async fn add_and_list_assignees() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "assignee-1").await;

    client
        .create_project(&ws.slug, project_req("assign-proj", "AS"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "assign-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Assign Me".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let assignee = client
        .add_assignee(
            &ws.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: user.id.0,
            },
        )
        .await
        .expect("add assignee");

    assert_eq!(assignee.assignee.id, user.id.0);

    let list = client
        .list_assignees(&ws.slug, &task.readable_id)
        .await
        .expect("list assignees");

    assert_eq!(list.len(), 1, "one assignee");

    db.teardown().await;
}

#[tokio::test]
async fn add_duplicate_assignee_returns_409() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "dup-assignee-1").await;

    client
        .create_project(&ws.slug, project_req("dup-assign-proj", "DA"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "dup-assign-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Dup Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    client
        .add_assignee(
            &ws.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: user.id.0,
            },
        )
        .await
        .expect("first add");

    let result = client
        .add_assignee(
            &ws.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: user.id.0,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "duplicate assignee must return 409, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn remove_assignee_unassigns_user() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "rm-assignee-1").await;

    client
        .create_project(&ws.slug, project_req("rm-assign-proj", "RA"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "rm-assign-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Remove Assignee".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    client
        .add_assignee(
            &ws.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: user.id.0,
            },
        )
        .await
        .expect("add assignee");

    client
        .remove_assignee(&ws.slug, &task.readable_id, &format!("user:{}", user.id.0))
        .await
        .expect("remove assignee");

    let list = client
        .list_assignees(&ws.slug, &task.readable_id)
        .await
        .expect("list assignees after remove");

    assert!(list.is_empty(), "assignee must be removed");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_list_references() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ref-1").await;

    client
        .create_project(&ws.slug, project_req("ref-proj", "RF"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "ref-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task_a = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Source".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task A");

    let task_b = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Target".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task B");

    let reference = client
        .create_reference(
            &ws.slug,
            &task_a.readable_id,
            CreateReferenceRequest {
                kind: "relates".to_string(),
                target_task_readable_id: Some(task_b.readable_id.clone()),
                target_document_id: None,
            },
        )
        .await
        .expect("create reference");

    assert_eq!(reference.kind, "relates");

    let refs = client
        .list_references(&ws.slug, &task_a.readable_id)
        .await
        .expect("list references");

    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].id, reference.id);

    db.teardown().await;
}

#[tokio::test]
async fn backlinks_surface_pending_references() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "backlink-1").await;

    client
        .create_project(&ws.slug, project_req("backlink-proj", "BK"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "backlink-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task_a = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Source".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task A");

    let task_b = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Target".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task B");

    client
        .create_reference(
            &ws.slug,
            &task_a.readable_id,
            CreateReferenceRequest {
                kind: "blocks".to_string(),
                target_task_readable_id: Some(task_b.readable_id.clone()),
                target_document_id: None,
            },
        )
        .await
        .expect("create reference");

    let backlinks = client
        .list_task_backlinks(&ws.slug, &task_b.readable_id)
        .await
        .expect("list backlinks on target");

    assert_eq!(
        backlinks.items.len(),
        1,
        "target must see one backlink from source"
    );
    assert_eq!(backlinks.items[0].source_readable_id, task_a.readable_id);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Checklist
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_list_checklist_items() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "checklist-1").await;

    client
        .create_project(&ws.slug, project_req("checklist-proj", "CL"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "checklist-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Checklist Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let item = client
        .create_checklist_item(
            &ws.slug,
            &task.readable_id,
            CreateChecklistItemRequest {
                title: "Step 1".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create checklist item");

    assert_eq!(item.title, "Step 1");
    assert!(!item.checked);

    let list = client
        .list_checklist(&ws.slug, &task.readable_id)
        .await
        .expect("list checklist");

    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, item.id);

    db.teardown().await;
}

#[tokio::test]
async fn promote_checklist_item_returns_409_on_second_promote() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "promote-dup-1").await;

    client
        .create_project(&ws.slug, project_req("promote-proj", "PR"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "promote-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Parent".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let item = client
        .create_checklist_item(
            &ws.slug,
            &task.readable_id,
            CreateChecklistItemRequest {
                title: "Sub".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create checklist item");

    client
        .promote_checklist_item(
            &ws.slug,
            &task.readable_id,
            item.id,
            PromoteChecklistItemRequest {
                board_id: board.id,
                column_id: col.id,
            },
        )
        .await
        .expect("first promote");

    let result = client
        .promote_checklist_item(
            &ws.slug,
            &task.readable_id,
            item.id,
            PromoteChecklistItemRequest {
                board_id: board.id,
                column_id: col.id,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "second promote must return 409, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Activity
// ---------------------------------------------------------------------------

#[tokio::test]
async fn activity_is_recorded_on_task_create() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "activity-1").await;

    client
        .create_project(&ws.slug, project_req("activity-proj", "AC"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "activity-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Tracked".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let activity = client
        .list_activity(&ws.slug, &task.readable_id)
        .await
        .expect("list activity");

    assert!(
        !activity.items.is_empty(),
        "must have at least one activity entry (created)"
    );
    assert_eq!(activity.items[0].kind, "created");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Cross-tenant isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cross_tenant_board_access_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client_a, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "ct-board-alice").await;
    let (_client_b, ws_b, _) =
        support::login_user_with_workspace(&server, &db, "ct-board-bob").await;

    client_a
        .create_project(&ws_a.slug, project_req("ct-proj-a", "CTA"))
        .await
        .expect("create project");

    let board = client_a
        .create_board(
            &ws_a.slug,
            "ct-proj-a",
            CreateBoardRequest {
                name: "Board A".to_string(),
            },
        )
        .await
        .expect("create board");

    let result = client_a.get_board(&ws_b.slug, board.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant board get must return 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_task_access_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client_a, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "ct-task-alice").await;
    let (_client_b, ws_b, _) =
        support::login_user_with_workspace(&server, &db, "ct-task-bob").await;

    client_a
        .create_project(&ws_a.slug, project_req("ct-task-proj-a", "CTT"))
        .await
        .expect("create project");

    let board = client_a
        .create_board(
            &ws_a.slug,
            "ct-task-proj-a",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client_a
        .create_column(
            &ws_a.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = client_a
        .create_task(
            &ws_a.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Secret".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let result = client_a.get_task(&ws_b.slug, &task.readable_id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant task get must return 404, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Permission gate: viewer cannot mutate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn viewer_cannot_create_board() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "board-authz-owner").await;

    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Board Authz Project".to_string(),
                slug: "board-authz-proj".to_string(),
                task_prefix: "BAZ".to_string(),
                visibility: Some("workspace".to_string()),
                visibility_role: Some("viewer".to_string()),
            },
        )
        .await
        .expect("create project");

    let (viewer, _) = add_member(
        &db,
        &server,
        ws.id,
        "board-authz-viewer",
        MemberRole::Member,
    )
    .await;

    let result = viewer
        .create_board(
            &ws.slug,
            "board-authz-proj",
            CreateBoardRequest {
                name: "Unauthorized".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "viewer must get 403 on board create, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn viewer_cannot_create_task() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "task-authz-owner").await;

    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Task Authz Project".to_string(),
                slug: "task-authz-proj".to_string(),
                task_prefix: "TAZ".to_string(),
                visibility: Some("workspace".to_string()),
                visibility_role: Some("viewer".to_string()),
            },
        )
        .await
        .expect("create project");

    let board = owner
        .create_board(
            &ws.slug,
            "task-authz-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = owner
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let (viewer, _) =
        add_member(&db, &server, ws.id, "task-authz-viewer", MemberRole::Member).await;

    let result = viewer
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Unauthorized".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "viewer must get 403 on task create, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Invalid input: 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_task_with_invalid_priority_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-422-1").await;

    client
        .create_project(&ws.slug, project_req("task-422-proj", "T4"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-422-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let result = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Bad Priority".to_string(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    priority: Some("invalid_priority".to_string()),
                    due_date: None,
                    estimate: None,
                    labels: vec![],
                    custom: None,
                }),
                before: None,
                after: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "invalid priority must return 422, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn create_task_with_negative_estimate_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-neg-est-1").await;

    client
        .create_project(&ws.slug, project_req("task-neg-proj", "NE"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-neg-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let result = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Neg Estimate".to_string(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    priority: None,
                    due_date: None,
                    estimate: Some(-1),
                    labels: vec![],
                    custom: None,
                }),
                before: None,
                after: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "negative estimate must return 422, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Board-level grant resolution (Fix 1: build_board_chain Board segment)
// ---------------------------------------------------------------------------

/// A board-scoped grant (permission_grants.board_id = board.id) must be honored by
/// the permission engine: the grantee can read the board at the granted role.
#[tokio::test]
async fn board_scoped_grant_is_honored_by_resolution() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "bgrant-owner").await;

    // Private project: no membership-derived visibility, so a non-owner needs an
    // explicit grant to access the board.
    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Grant Test Project".to_string(),
                slug: "bgrant-proj".to_string(),
                task_prefix: "BGT".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = owner
        .create_board(
            &ws.slug,
            "bgrant-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let (grantee, grantee_user) =
        add_member(&db, &server, ws.id, "bgrant-grantee", MemberRole::Member).await;

    // Before any grant: grantee cannot access the board (private project, no grants).
    // Private resources use existence concealment, so the response is 404, not 403.
    let before = grantee.get_board(&ws.slug, board.id).await;
    assert!(
        before.is_err(),
        "grantee must be denied before any grant, got: {before:?}"
    );

    // Insert a board-scoped grant directly — no HTTP route exists for board grants yet.
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(grantee_user.id),
            api_key_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: Some(BoardId(board.id)),
            role: ResourceRole::Viewer,
            created_by_user_id: None,
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert board grant");

    // After the board grant: the grantee can read the board.
    let after = grantee.get_board(&ws.slug, board.id).await;
    assert!(
        after.is_ok(),
        "board-scoped grant must be honored, got: {after:?}"
    );

    db.teardown().await;
}

/// A board-scoped grant also grants access to tasks on that board (TaskRes resolves
/// through the same build_board_chain).
#[tokio::test]
async fn board_scoped_grant_grants_task_access() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "bgrant-task-owner").await;

    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Task Grant Project".to_string(),
                slug: "bgrant-task-proj".to_string(),
                task_prefix: "BGK".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = owner
        .create_board(
            &ws.slug,
            "bgrant-task-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = owner
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task = owner
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let (grantee, grantee_user) = add_member(
        &db,
        &server,
        ws.id,
        "bgrant-task-grantee",
        MemberRole::Member,
    )
    .await;

    // No grant yet: task is inaccessible on the private project (existence concealment → 404).
    let before = grantee.get_task(&ws.slug, &task.readable_id).await;
    assert!(
        before.is_err(),
        "task must be denied before board grant, got: {before:?}"
    );

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(grantee_user.id),
            api_key_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: Some(BoardId(board.id)),
            role: ResourceRole::Viewer,
            created_by_user_id: None,
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert board grant");

    // After the board grant: the task on that board becomes accessible.
    let after = grantee.get_task(&ws.slug, &task.readable_id).await;
    assert!(
        after.is_ok(),
        "board-scoped grant must propagate to tasks, got: {after:?}"
    );

    db.teardown().await;
}

/// Existing behavior is unchanged: a member without any grant on a private board
/// is still denied (regression guard).
#[tokio::test]
async fn no_board_grant_remains_denied() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "no-bgrant-owner").await;

    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "No Grant Project".to_string(),
                slug: "no-bgrant-proj".to_string(),
                task_prefix: "NBG".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = owner
        .create_board(
            &ws.slug,
            "no-bgrant-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let (non_grantee, _) =
        add_member(&db, &server, ws.id, "no-bgrant-member", MemberRole::Member).await;

    // Private project: existence concealment → 404 for members without any grant.
    let result = non_grantee.get_board(&ws.slug, board.id).await;
    assert!(
        result.is_err(),
        "member without any grant must be denied on private board, got: {result:?}"
    );

    db.teardown().await;
}
