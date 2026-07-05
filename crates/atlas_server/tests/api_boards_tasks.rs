#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateProjectRequest, CreateUserApiKeyRequest, InitialGrantRequest,
    boards_tasks::{
        AddAssigneeRequest, CreateBoardRequest, CreateChecklistItemRequest, CreateColumnRequest,
        CreateReferenceRequest, CreateTaskRequest, MoveTaskRequest, PromoteChecklistItemRequest,
        UpdateBoardRequest, UpdateChecklistItemRequest, UpdateColumnRequest, UpdateTaskRequest,
    },
    documents::CreateDocumentRequest,
};
use atlas_client::ClientError;
use atlas_domain::{
    Actor, WorkspaceCtx, entities::identity::MemberRole, entities::permissions::NewPermissionGrant,
    ids::BoardId, ids::UserId, permissions::ResourceRole,
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

/// Returns the `(target_title, target_document_id)` rows of every document_link
/// whose source is the given task, ordered by title.
async fn task_links(
    db: &support::TestDb,
    task_id: uuid::Uuid,
) -> Vec<(String, Option<uuid::Uuid>)> {
    use sea_orm::{ConnectionTrait, Statement};

    let rows = db
        .conn()
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT target_title, target_document_id FROM document_links \
                 WHERE source_task_id = '{task_id}' ORDER BY target_title"
            ),
        ))
        .await
        .expect("query document_links");

    rows.into_iter()
        .map(|r| {
            let title: String = r.try_get("", "target_title").expect("target_title");
            let doc: Option<uuid::Uuid> = r
                .try_get("", "target_document_id")
                .expect("target_document_id");
            (title, doc)
        })
        .collect()
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
                color: None,
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
                color: None,
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
                color: None,
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
                color: None,
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

#[tokio::test]
async fn delete_column_with_live_task_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "col-del-busy").await;

    client
        .create_project(&ws.slug, project_req("col-busy-proj", "CB"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-busy-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let busy_col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Has Tasks".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create busy column");

    client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: busy_col.id,
                title: "Pinned Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let result = client.delete_column(&ws.slug, board.id, busy_col.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "deleting a column with live tasks must be rejected with 422, got: {result:?}"
    );

    let empty_col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Empty".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create empty column");

    client
        .delete_column(&ws.slug, board.id, empty_col.id)
        .await
        .expect("deleting an empty column must still succeed");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Task happy-path CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_task_returns_board_name_and_column_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-names-1").await;

    client
        .create_project(&ws.slug, project_req("task-names-proj", "TN"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-names-proj",
            CreateBoardRequest {
                name: "My Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "In Progress".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create column");

    let created = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Named Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let fetched = client
        .get_task(&ws.slug, &created.readable_id)
        .await
        .expect("get task");

    assert_eq!(
        fetched.board_name, "My Board",
        "get_task must return board_name matching the seeded board"
    );
    assert_eq!(
        fetched.column_name, "In Progress",
        "get_task must return column_name matching the seeded column"
    );

    db.teardown().await;
}

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
                color: None,
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
                color: None,
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

    for task in &page.items {
        assert_eq!(
            task.board_name, "Board",
            "board-scoped listing must carry board_name"
        );
        assert_eq!(
            task.column_name, "Todo",
            "board-scoped listing must carry column_name"
        );
        assert_eq!(
            task.board_id, board.id,
            "board-scoped listing must carry the board's id"
        );
    }

    db.teardown().await;
}

#[tokio::test]
async fn list_tasks_includes_labels() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-labels-1").await;

    client
        .create_project(&ws.slug, project_req("task-labels-proj", "TLB"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-labels-proj",
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
                color: None,
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
                title: "Labeled".to_string(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    labels: vec!["shell".to_string(), "M1".to_string()],
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let page = client
        .list_tasks(&ws.slug, board.id, None, None)
        .await
        .expect("list tasks");

    let summary = page.items.first().expect("one task in the board");
    assert_eq!(
        summary.labels,
        vec!["shell".to_string(), "M1".to_string()],
        "the kanban summary must carry the task labels so cards can render them"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_tasks_includes_assignees_with_names() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "task-assignees-1").await;

    client
        .create_project(&ws.slug, project_req("task-asg-proj", "TAS"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-asg-proj",
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
                color: None,
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
                title: "Assigned".to_string(),
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

    let page = client
        .list_tasks(&ws.slug, board.id, None, None)
        .await
        .expect("list tasks");

    let summary = page.items.first().expect("one task in the board");
    let assignee = summary
        .assignees
        .first()
        .expect("the kanban summary must carry the task's assignees");

    assert_eq!(assignee.id, user.id.0, "summary assignee id must match");
    assert_eq!(assignee.r#type, "user");
    assert_eq!(
        assignee.display_name.as_deref(),
        Some(user.display_name.as_str()),
        "the summary assignee must carry the resolved display name, not a generic fallback"
    );

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
                color: None,
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
                color: None,
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
                color: None,
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
                color: None,
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

// A move anchored to a neighbour TASK ID (what clients send — they never see the
// internal fractional keys) must resolve and succeed, not fail as an invalid key.
#[tokio::test]
async fn move_task_with_task_id_anchor_succeeds() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-anchor-1").await;

    client
        .create_project(&ws.slug, project_req("anchor-proj", "AN"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "anchor-proj",
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
                color: None,
            },
        )
        .await
        .expect("create column");

    let make = |title: &str| CreateTaskRequest {
        column_id: col.id,
        title: title.to_string(),
        description: None,
        properties: None,
        before: None,
        after: None,
    };

    let t1 = client
        .create_task(&ws.slug, board.id, make("T1"))
        .await
        .expect("t1");
    let t2 = client
        .create_task(&ws.slug, board.id, make("T2"))
        .await
        .expect("t2");

    // Anchor the move to T1 by its task id; before the fix this was treated as a
    // fractional key, was invalid, and returned 409 PositionExhausted.
    let moved = client
        .move_task(
            &ws.slug,
            &t2.readable_id,
            MoveTaskRequest {
                column_id: col.id,
                before: Some(t1.id.to_string()),
                after: None,
            },
        )
        .await
        .expect("move anchored by task id should succeed");

    assert_eq!(moved.column_id, col.id);

    db.teardown().await;
}

#[tokio::test]
async fn move_task_across_boards_succeeds() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-xboard-1").await;

    // Two projects, each with its own board and column, exercises updating both
    // board_id and project_id on the moved task.
    client
        .create_project(&ws.slug, project_req("proj-a", "PA"))
        .await
        .expect("create project a");
    client
        .create_project(&ws.slug, project_req("proj-b", "PB"))
        .await
        .expect("create project b");

    let board_a = client
        .create_board(
            &ws.slug,
            "proj-a",
            CreateBoardRequest {
                name: "A".to_string(),
            },
        )
        .await
        .expect("create board a");
    let board_b = client
        .create_board(
            &ws.slug,
            "proj-b",
            CreateBoardRequest {
                name: "B".to_string(),
            },
        )
        .await
        .expect("create board b");

    let col_a = client
        .create_column(
            &ws.slug,
            board_a.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create column a");
    let col_b = client
        .create_column(
            &ws.slug,
            board_b.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create column b");

    let task = client
        .create_task(
            &ws.slug,
            board_a.id,
            CreateTaskRequest {
                column_id: col_a.id,
                title: "Cross".to_string(),
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
        .expect("cross-board move should succeed");

    assert_eq!(moved.column_id, col_b.id, "task lands in the target column");
    assert_eq!(moved.board_id, board_b.id, "task adopts the target board");
    assert_eq!(
        moved.project_id, board_b.project_id,
        "task adopts the target board's project"
    );
    assert_eq!(
        moved.readable_id, task.readable_id,
        "readable id is immutable identity"
    );

    let in_board_b = client
        .list_tasks(&ws.slug, board_b.id, None, None)
        .await
        .expect("list board b tasks");
    assert!(
        in_board_b.items.iter().any(|t| t.id == task.id),
        "moved task must appear in the target board's task list"
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
                color: None,
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
    assert_eq!(
        assignee.assignee.display_name.as_deref(),
        Some(user.display_name.as_str()),
        "add must return the assignee's resolved display name, not a generic fallback"
    );

    let list = client
        .list_assignees(&ws.slug, &task.readable_id)
        .await
        .expect("list assignees");

    assert_eq!(list.len(), 1, "one assignee");
    assert_eq!(
        list[0].assignee.display_name.as_deref(),
        Some(user.display_name.as_str()),
        "list must return the assignee's resolved display name"
    );

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
                color: None,
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
                color: None,
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
                color: None,
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
                color: None,
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
                color: None,
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
                color: None,
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

/// A successful promotion must always materialize the parent reference. The
/// reference insert is part of the promotion transaction, so a `None`
/// `parent_reference` would mean the insert silently failed yet the task and
/// promoted-mark still committed — exactly the torn state FIX 1 forbids.
#[tokio::test]
async fn promote_checklist_item_always_persists_parent_reference() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "promote-ref-1").await;

    client
        .create_project(&ws.slug, project_req("promref-proj", "PRF"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "promref-proj",
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
                color: None,
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

    let promotion = client
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
        .expect("promote");

    let parent_ref = promotion
        .parent_reference
        .expect("promotion must always carry a parent reference");
    assert_eq!(parent_ref.kind, "parent");
    assert_eq!(
        parent_ref.target_task_id,
        Some(task.id),
        "parent reference must point at the originating task"
    );

    // The promoted child task must land on the parent's board and project.
    assert_eq!(
        promotion.task.board_id, task.board_id,
        "promoted child must share the parent task's board_id"
    );
    assert_eq!(
        promotion.task.project_id, task.project_id,
        "promoted child must share the parent task's project_id"
    );

    // The reference is durable: it surfaces as an inbound backlink on the parent.
    let backlinks = client
        .list_task_backlinks(&ws.slug, &task.readable_id)
        .await
        .expect("list backlinks");
    assert!(
        backlinks
            .items
            .iter()
            .any(|b| b.source_task_id == promotion.task.id && b.kind == "parent"),
        "promoted task must back-link to its parent, got: {:?}",
        backlinks.items
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Task description wikilinks (FIX 3)
// ---------------------------------------------------------------------------

/// A `[[Existing Doc]]` wikilink in a task description must be persisted as a
/// document_link whose source is the task and whose target resolves to the doc.
#[tokio::test]
async fn task_description_wikilink_persists_resolved_link() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "wikilink-1").await;

    client
        .create_project(&ws.slug, project_req("wiki-proj", "WK"))
        .await
        .expect("create project");

    let doc = client
        .create_document(
            &ws.slug,
            "wiki-proj",
            CreateDocumentRequest {
                title: "Existing Doc".to_string(),
                folder_id: None,
                content: Some("body".to_string()),
            },
        )
        .await
        .expect("create document");

    let board = client
        .create_board(
            &ws.slug,
            "wiki-proj",
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
                color: None,
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
                title: "Task".to_string(),
                description: Some("see [[Existing Doc]] for context".to_string()),
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let links = task_links(&db, task.id).await;
    assert_eq!(
        links,
        vec![("Existing Doc".to_string(), Some(doc.id))],
        "task description wikilink must persist as a resolved document_link"
    );

    db.teardown().await;
}

/// An id-bound `[[<uuid>|Title]]` wikilink in a task description resolves to the
/// target document by its stable id, independent of the display title text.
#[tokio::test]
async fn task_description_id_bound_wikilink_resolves_by_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "wikilink-id-1").await;

    client
        .create_project(&ws.slug, project_req("wiki-id-proj", "WI"))
        .await
        .expect("create project");

    let doc = client
        .create_document(
            &ws.slug,
            "wiki-id-proj",
            CreateDocumentRequest {
                title: "Existing Doc".to_string(),
                folder_id: None,
                content: Some("body".to_string()),
            },
        )
        .await
        .expect("create document");

    let board = client
        .create_board(
            &ws.slug,
            "wiki-id-proj",
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
                color: None,
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
                title: "Task".to_string(),
                description: Some(format!("see [[{}|Renamed Label]] for context", doc.id)),
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let links = task_links(&db, task.id).await;
    assert_eq!(
        links,
        vec![("Renamed Label".to_string(), Some(doc.id))],
        "id-bound task wikilink must resolve to the target id with its display title"
    );

    db.teardown().await;
}

/// A `[[Nonexistent]]` wikilink is stored as a pending link (target NULL),
/// consistent with E04 document behavior — not dropped, not an error.
#[tokio::test]
async fn task_description_wikilink_to_missing_doc_is_pending() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "wikilink-2").await;

    client
        .create_project(&ws.slug, project_req("wiki2-proj", "WB"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "wiki2-proj",
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
                color: None,
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
                title: "Task".to_string(),
                description: Some("links to [[Nonexistent]]".to_string()),
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let links = task_links(&db, task.id).await;
    assert_eq!(
        links,
        vec![("Nonexistent".to_string(), None)],
        "unresolved wikilink must persist as a pending document_link"
    );

    db.teardown().await;
}

/// Patching the description replaces the task's link set: old links go, new
/// links arrive, all in the same write.
#[tokio::test]
async fn task_description_patch_replaces_wikilinks() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "wikilink-3").await;

    client
        .create_project(&ws.slug, project_req("wiki3-proj", "WC"))
        .await
        .expect("create project");

    let alpha = client
        .create_document(
            &ws.slug,
            "wiki3-proj",
            CreateDocumentRequest {
                title: "Alpha".to_string(),
                folder_id: None,
                content: Some("a".to_string()),
            },
        )
        .await
        .expect("create alpha");

    let beta = client
        .create_document(
            &ws.slug,
            "wiki3-proj",
            CreateDocumentRequest {
                title: "Beta".to_string(),
                folder_id: None,
                content: Some("b".to_string()),
            },
        )
        .await
        .expect("create beta");

    let board = client
        .create_board(
            &ws.slug,
            "wiki3-proj",
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
                color: None,
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
                title: "Task".to_string(),
                description: Some("[[Alpha]]".to_string()),
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    assert_eq!(
        task_links(&db, task.id).await,
        vec![("Alpha".to_string(), Some(alpha.id))]
    );

    client
        .update_task(
            &ws.slug,
            &task.readable_id,
            UpdateTaskRequest {
                description: Some("now [[Beta]]".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("patch description");

    assert_eq!(
        task_links(&db, task.id).await,
        vec![("Beta".to_string(), Some(beta.id))],
        "patching the description must replace the old link set"
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
                color: None,
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

#[tokio::test]
async fn description_edits_are_suppressed_and_other_fields_coalesce() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "activity-coalesce").await;

    client
        .create_project(&ws.slug, project_req("coalesce-proj", "CO"))
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            "coalesce-proj",
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
                color: None,
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

    // Description autosaves must never reach the activity feed, no matter how many.
    for draft in ["First draft", "Second draft", "Third draft"] {
        client
            .update_task(
                &ws.slug,
                &task.readable_id,
                UpdateTaskRequest {
                    title: None,
                    description: Some(draft.to_string()),
                    priority: None,
                    due_date: None,
                    estimate: None,
                    labels: None,
                    properties: None,
                },
            )
            .await
            .expect("update description");
    }

    assert_eq!(
        count_activity_of_kind(&db, task.id, "field_changed").await,
        0,
        "editing the description never records an activity entry"
    );

    // Other fields ARE recorded, and consecutive same-field edits by the same actor
    // coalesce into a single entry.
    for renamed in ["Renamed A", "Renamed B"] {
        client
            .update_task(
                &ws.slug,
                &task.readable_id,
                UpdateTaskRequest {
                    title: Some(renamed.to_string()),
                    description: None,
                    priority: None,
                    due_date: None,
                    estimate: None,
                    labels: None,
                    properties: None,
                },
            )
            .await
            .expect("update title");
    }

    assert_eq!(
        count_activity_of_kind(&db, task.id, "field_changed").await,
        1,
        "consecutive title edits coalesce into one entry"
    );

    let payload = latest_field_changed_payload(&db, task.id).await;
    assert_eq!(payload["field_changed"]["field"].as_str(), Some("title"));
    assert_eq!(
        payload["field_changed"]["new_value"].as_str(),
        Some("Renamed B"),
        "the coalesced entry reflects the latest edit"
    );
    assert_eq!(
        payload["field_changed"]["old_value"].as_str(),
        Some("Tracked"),
        "the coalesced entry keeps the burst's original (pre-edit) old_value"
    );

    // A change to a different field is a distinct entry — coalescing is field-scoped.
    client
        .update_task(
            &ws.slug,
            &task.readable_id,
            UpdateTaskRequest {
                title: None,
                description: None,
                priority: Some(serde_json::json!("high")),
                due_date: None,
                estimate: None,
                labels: None,
                properties: None,
            },
        )
        .await
        .expect("update priority");

    assert_eq!(
        count_activity_of_kind(&db, task.id, "field_changed").await,
        2,
        "a change to a different field is not merged into the title entry"
    );

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
                color: None,
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
                color: None,
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
                color: None,
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
                color: None,
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
// Intra-workspace cross-resource authorization (FIX 2 — IDOR)
//
// A principal with an editor grant on board/task A and NO access to board/task B
// (same workspace) must NOT mutate B's sub-resources by routing the request
// through A's URL. The owning parent is now constrained in every sub-resource
// op, so a mismatch resolves to 404 — never a silent cross-resource mutation.
// ---------------------------------------------------------------------------

/// Grants an editor board-scoped grant to `user` on `board_id`.
async fn grant_board_editor(
    db: &support::TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    user_id: UserId,
    board_id: uuid::Uuid,
) {
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws_id,
            user_id: Some(user_id),
            api_key_id: None,
            group_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: Some(BoardId(board_id)),
            role: ResourceRole::Editor,
            created_by_user_id: None,
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert board grant");
}

struct IdorFixture {
    ws: atlas_domain::entities::identity::Workspace,
    owner: atlas_client::AtlasClient,
    attacker: atlas_client::AtlasClient,
    board_a: uuid::Uuid,
    col_a: uuid::Uuid,
    task_a_readable: String,
    item_a: uuid::Uuid,
    task_b_readable: String,
    item_b: uuid::Uuid,
    ref_b: uuid::Uuid,
    board_b: uuid::Uuid,
    col_b: uuid::Uuid,
}

/// Builds two private boards A and B in one workspace, each with a column, a
/// task, a checklist item and a reference. The attacker holds an editor grant on
/// board A only (no access to B).
async fn setup_idor(
    db: &support::TestDb,
    server: &support::TestServer,
    prefix: &str,
) -> IdorFixture {
    let (owner, ws, _) =
        support::login_user_with_workspace(server, db, &format!("{prefix}-owner")).await;

    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "IDOR Project".to_string(),
                slug: format!("{prefix}-proj"),
                task_prefix: "IDR".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let make_board = |name: &'static str| {
        let owner = &owner;
        let ws = &ws;
        let proj_slug = format!("{prefix}-proj");
        async move {
            let board = owner
                .create_board(
                    &ws.slug,
                    &proj_slug,
                    CreateBoardRequest {
                        name: name.to_string(),
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
                        color: None,
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
                        title: format!("{name} Task"),
                        description: None,
                        properties: None,
                        before: None,
                        after: None,
                    },
                )
                .await
                .expect("create task");
            let item = owner
                .create_checklist_item(
                    &ws.slug,
                    &task.readable_id,
                    CreateChecklistItemRequest {
                        title: "Item".to_string(),
                        before: None,
                        after: None,
                    },
                )
                .await
                .expect("create checklist item");
            let other_task = owner
                .create_task(
                    &ws.slug,
                    board.id,
                    CreateTaskRequest {
                        column_id: col.id,
                        title: format!("{name} Other Task"),
                        description: None,
                        properties: None,
                        before: None,
                        after: None,
                    },
                )
                .await
                .expect("create other task");
            let reference = owner
                .create_reference(
                    &ws.slug,
                    &task.readable_id,
                    CreateReferenceRequest {
                        kind: "relates".to_string(),
                        target_task_readable_id: Some(other_task.readable_id.clone()),
                        target_document_id: None,
                    },
                )
                .await
                .expect("create reference");
            (board, col, task, item, reference)
        }
    };

    let (board_a, col_a, task_a, item_a, _ref_a) = make_board("Board A").await;
    let (board_b, col_b, task_b, item_b, ref_b) = make_board("Board B").await;

    let (attacker, attacker_user) = add_member(
        db,
        server,
        ws.id,
        &format!("{prefix}-attacker"),
        MemberRole::Member,
    )
    .await;

    // Attacker gets editor on board A only — nothing on board B.
    grant_board_editor(db, ws.id, attacker_user.id, board_a.id).await;

    IdorFixture {
        ws,
        owner,
        attacker,
        board_a: board_a.id,
        col_a: col_a.id,
        task_a_readable: task_a.readable_id,
        item_a: item_a.id,
        task_b_readable: task_b.readable_id,
        item_b: item_b.id,
        ref_b: ref_b.id,
        board_b: board_b.id,
        col_b: col_b.id,
    }
}

fn is_404<T: std::fmt::Debug>(result: &Result<T, ClientError>) -> bool {
    matches!(result, Err(ClientError::Api(p)) if p.status == 404)
}

/// Patching/deleting B's checklist item through A's authorized task URL → 404,
/// while the same op on A's own item still works.
#[tokio::test]
async fn idor_checklist_item_cross_task_is_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let f = setup_idor(&db, &server, "idor-cl").await;

    // Attacker authorizes task A (has editor on board A) but targets B's item.
    let patch = f
        .attacker
        .update_checklist_item(
            &f.ws.slug,
            &f.task_a_readable,
            f.item_b,
            UpdateChecklistItemRequest {
                title: Some("hijacked".to_string()),
                ..Default::default()
            },
        )
        .await;
    assert!(
        is_404(&patch),
        "cross-task checklist patch must be 404, got: {patch:?}"
    );

    let delete = f
        .attacker
        .delete_checklist_item(&f.ws.slug, &f.task_a_readable, f.item_b)
        .await;
    assert!(
        is_404(&delete),
        "cross-task checklist delete must be 404, got: {delete:?}"
    );

    // Legitimate same-task op still works: create an item on A, then patch it.
    let own_item = f
        .attacker
        .create_checklist_item(
            &f.ws.slug,
            &f.task_a_readable,
            CreateChecklistItemRequest {
                title: "mine".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create own item");
    let ok = f
        .attacker
        .update_checklist_item(
            &f.ws.slug,
            &f.task_a_readable,
            own_item.id,
            UpdateChecklistItemRequest {
                checked: Some(true),
                ..Default::default()
            },
        )
        .await;
    assert!(
        ok.is_ok(),
        "same-task checklist patch must succeed, got: {ok:?}"
    );

    db.teardown().await;
}

/// Promoting B's checklist item through A's task URL → 404.
#[tokio::test]
async fn idor_promote_cross_task_is_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let f = setup_idor(&db, &server, "idor-prom").await;

    let promote = f
        .attacker
        .promote_checklist_item(
            &f.ws.slug,
            &f.task_a_readable,
            f.item_b,
            PromoteChecklistItemRequest {
                board_id: f.board_a,
                column_id: f.col_a,
            },
        )
        .await;
    assert!(
        is_404(&promote),
        "cross-task promote must be 404, got: {promote:?}"
    );

    db.teardown().await;
}

/// Deleting B's reference through A's task URL → 404.
#[tokio::test]
async fn idor_reference_cross_task_is_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let f = setup_idor(&db, &server, "idor-ref").await;

    let delete = f
        .attacker
        .delete_reference(&f.ws.slug, &f.task_a_readable, f.ref_b)
        .await;
    assert!(
        is_404(&delete),
        "cross-task reference delete must be 404, got: {delete:?}"
    );

    db.teardown().await;
}

/// Patching/moving/deleting B's column through A's board URL → 404, while the
/// same op on A's own column still works.
#[tokio::test]
async fn idor_column_cross_board_is_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let f = setup_idor(&db, &server, "idor-col").await;

    let patch = f
        .attacker
        .update_column(
            &f.ws.slug,
            f.board_a,
            f.col_b,
            UpdateColumnRequest {
                name: Some("hijacked".to_string()),
                before: None,
                after: None,
                color: None,
            },
        )
        .await;
    assert!(
        is_404(&patch),
        "cross-board column patch must be 404, got: {patch:?}"
    );

    let delete = f
        .attacker
        .delete_column(&f.ws.slug, f.board_a, f.col_b)
        .await;
    assert!(
        is_404(&delete),
        "cross-board column delete must be 404, got: {delete:?}"
    );

    // Legitimate same-board op still works.
    let ok = f
        .attacker
        .update_column(
            &f.ws.slug,
            f.board_a,
            f.col_a,
            UpdateColumnRequest {
                name: Some("renamed".to_string()),
                before: None,
                after: None,
                color: None,
            },
        )
        .await;
    assert!(
        ok.is_ok(),
        "same-board column patch must succeed, got: {ok:?}"
    );

    db.teardown().await;
}

/// Creating a task into B's column through A's board URL → rejected (not written
/// into another board's column).
#[tokio::test]
async fn idor_create_task_into_foreign_column_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let f = setup_idor(&db, &server, "idor-create").await;

    let result = f
        .attacker
        .create_task(
            &f.ws.slug,
            f.board_a,
            CreateTaskRequest {
                column_id: f.col_b,
                title: "smuggled".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404 || p.status == 422),
        "creating a task into another board's column must be rejected, got: {result:?}"
    );

    db.teardown().await;
}

/// Moving A's task into B's column → rejected (a task cannot jump boards).
#[tokio::test]
async fn idor_move_task_into_foreign_column_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let f = setup_idor(&db, &server, "idor-move").await;

    let result = f
        .attacker
        .move_task(
            &f.ws.slug,
            &f.task_a_readable,
            MoveTaskRequest {
                column_id: f.col_b,
                before: None,
                after: None,
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404 || p.status == 422),
        "moving a task into another board's column must be rejected, got: {result:?}"
    );

    // Sanity: B's task is untouched and out of reach for the attacker.
    let b = f.attacker.get_task(&f.ws.slug, &f.task_b_readable).await;
    assert!(
        is_404(&b),
        "attacker must not see task B at all, got: {b:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Helper: count task_activity rows for a task
// ---------------------------------------------------------------------------

async fn count_activity_of_kind(db: &support::TestDb, task_id: uuid::Uuid, kind: &str) -> i64 {
    use sea_orm::{ConnectionTrait, Statement};

    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT COUNT(*) AS cnt FROM task_activity \
                 WHERE task_id = '{task_id}' AND kind = '{kind}'"
            ),
        ))
        .await
        .expect("count_activity query")
        .expect("count_activity row");

    row.try_get::<i64>("", "cnt").expect("cnt")
}

// Reads the payload of the most recent field_changed activity row for a task.
async fn latest_field_changed_payload(
    db: &support::TestDb,
    task_id: uuid::Uuid,
) -> serde_json::Value {
    use sea_orm::{ConnectionTrait, Statement};

    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT payload FROM task_activity \
                 WHERE task_id = '{task_id}' AND kind = 'field_changed' \
                 ORDER BY created_at DESC LIMIT 1"
            ),
        ))
        .await
        .expect("query field_changed payload")
        .expect("field_changed row");

    row.try_get("", "payload").expect("payload")
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
            group_id: None,
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
                color: None,
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
            group_id: None,
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

// ---------------------------------------------------------------------------
// Finding #5 — Duplicate reference → 409
// ---------------------------------------------------------------------------

/// Creating the same reference twice must return 409 on the second attempt.
/// The `task_references_dedup_uidx` unique constraint triggers a 23505 which
/// must be classified as Conflict, not Internal.
#[tokio::test]
async fn duplicate_reference_returns_409() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "dup-ref-409-1").await;

    client
        .create_project(&ws.slug, project_req("dup-ref-proj", "DR"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "dup-ref-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task_a = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "A".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task A");

    let task_b = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "B".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task B");

    client
        .create_reference(
            &ws.slug,
            &task_a.readable_id,
            CreateReferenceRequest {
                kind: "relates".into(),
                target_task_readable_id: Some(task_b.readable_id.clone()),
                target_document_id: None,
            },
        )
        .await
        .expect("first reference");

    let result = client
        .create_reference(
            &ws.slug,
            &task_a.readable_id,
            CreateReferenceRequest {
                kind: "relates".into(),
                target_task_readable_id: Some(task_b.readable_id.clone()),
                target_document_id: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "duplicate reference must return 409, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Finding #6 — delete_reference records the REAL kind, not hardcoded relates
// ---------------------------------------------------------------------------

/// Deleting a `blocks` reference must record `blocks` in the activity payload,
/// not the previously hardcoded `relates`.
#[tokio::test]
async fn delete_blocks_reference_records_correct_kind_in_activity() {
    use sea_orm::{ConnectionTrait, Statement};

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "del-ref-kind-1").await;

    client
        .create_project(&ws.slug, project_req("del-ref-proj", "DRK"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "del-ref-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task_a = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "A".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task A");

    let task_b = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "B".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task B");

    let reference = client
        .create_reference(
            &ws.slug,
            &task_a.readable_id,
            CreateReferenceRequest {
                kind: "blocks".into(),
                target_task_readable_id: Some(task_b.readable_id.clone()),
                target_document_id: None,
            },
        )
        .await
        .expect("create blocks reference");

    client
        .delete_reference(&ws.slug, &task_a.readable_id, reference.id)
        .await
        .expect("delete reference");

    // Read the ReferenceRemoved activity row directly and assert the kind is blocks.
    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT payload FROM task_activity \
                 WHERE task_id = '{}' AND kind = 'reference_removed'",
                task_a.id
            ),
        ))
        .await
        .expect("query activity")
        .expect("activity row");

    let payload: serde_json::Value = row.try_get("", "payload").expect("payload");
    let kind = payload["reference_removed"]["kind"]
        .as_str()
        .expect("kind in payload");
    assert_eq!(
        kind, "blocks",
        "activity must record the real kind 'blocks', got: {kind}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Finding #7 — Reference target validation
// ---------------------------------------------------------------------------

/// Providing neither target nor both targets must return 422.
#[tokio::test]
async fn create_reference_without_target_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ref-notarget-1").await;

    client
        .create_project(&ws.slug, project_req("ref-notarget-proj", "RNT"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "ref-notarget-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "T".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task");

    let result = client
        .create_reference(
            &ws.slug,
            &task.readable_id,
            CreateReferenceRequest {
                kind: "relates".into(),
                target_task_readable_id: None,
                target_document_id: None,
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "no target must return 422, got: {result:?}"
    );

    db.teardown().await;
}

/// Providing both a task and a document target must return 422.
#[tokio::test]
async fn create_reference_with_both_targets_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "ref-bothtarget-1").await;

    client
        .create_project(&ws.slug, project_req("ref-bothtarget-proj", "RBT"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "ref-bothtarget-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task_a = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "A".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task A");

    let task_b = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "B".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task B");

    let doc = client
        .create_document(
            &ws.slug,
            "ref-bothtarget-proj",
            CreateDocumentRequest {
                title: "Doc".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("doc");

    let result = client
        .create_reference(
            &ws.slug,
            &task_a.readable_id,
            CreateReferenceRequest {
                kind: "relates".into(),
                target_task_readable_id: Some(task_b.readable_id.clone()),
                target_document_id: Some(doc.id),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "both targets must return 422, got: {result:?}"
    );

    db.teardown().await;
}

/// A target_task_readable_id that resolves to nothing must return 404 and
/// must NOT write any reference row.
#[tokio::test]
async fn create_reference_unknown_task_target_returns_404_and_no_row() {
    use sea_orm::{ConnectionTrait, Statement};

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ref-notask-1").await;

    client
        .create_project(&ws.slug, project_req("ref-notask-proj", "RNK"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "ref-notask-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Source".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task");

    let result = client
        .create_reference(
            &ws.slug,
            &task.readable_id,
            CreateReferenceRequest {
                kind: "relates".into(),
                target_task_readable_id: Some("NONEXISTENT-9999".into()),
                target_document_id: None,
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "unknown task target must return 404, got: {result:?}"
    );

    let count_row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT COUNT(*) AS cnt FROM task_references WHERE source_task_id = '{}'",
                task.id
            ),
        ))
        .await
        .expect("query")
        .expect("row");
    let cnt: i64 = count_row.try_get("", "cnt").expect("cnt");
    assert_eq!(cnt, 0, "no reference row must be written on 404");

    db.teardown().await;
}

/// A target_document_id that belongs to another workspace must return 404 and
/// must NOT write any reference row (cross-tenant document target guard).
#[tokio::test]
async fn create_reference_cross_tenant_document_target_returns_404() {
    use sea_orm::{ConnectionTrait, Statement};

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client_a, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "ref-ctdoc-alice").await;
    let (client_b, ws_b, _) =
        support::login_user_with_workspace(&server, &db, "ref-ctdoc-bob").await;

    client_a
        .create_project(&ws_a.slug, project_req("ref-ctdoc-proj-a", "RCA"))
        .await
        .expect("create project A");

    client_b
        .create_project(&ws_b.slug, project_req("ref-ctdoc-proj-b", "RCB"))
        .await
        .expect("create project B");

    let board_a = client_a
        .create_board(
            &ws_a.slug,
            "ref-ctdoc-proj-a",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board A");

    let col_a = client_a
        .create_column(
            &ws_a.slug,
            board_a.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col A");

    let task_a = client_a
        .create_task(
            &ws_a.slug,
            board_a.id,
            CreateTaskRequest {
                column_id: col_a.id,
                title: "Source".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task A");

    // Bob creates a document in workspace B.
    let doc_b = client_b
        .create_document(
            &ws_b.slug,
            "ref-ctdoc-proj-b",
            CreateDocumentRequest {
                title: "Bob Doc".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("doc B");

    // Alice tries to reference Bob's document — must be rejected.
    let result = client_a
        .create_reference(
            &ws_a.slug,
            &task_a.readable_id,
            CreateReferenceRequest {
                kind: "spec".into(),
                target_task_readable_id: None,
                target_document_id: Some(doc_b.id),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant document target must return 404, got: {result:?}"
    );

    let count_row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT COUNT(*) AS cnt FROM task_references WHERE source_task_id = '{}'",
                task_a.id
            ),
        ))
        .await
        .expect("query")
        .expect("row");
    let cnt: i64 = count_row.try_get("", "cnt").expect("cnt");
    assert_eq!(
        cnt, 0,
        "no reference row must be written on cross-tenant 404"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Finding #8 — Assignee: unknown principal → 404; duplicate → 409
// ---------------------------------------------------------------------------

/// Adding a non-existent user ID as assignee must return 404 (FK violation,
/// SQLSTATE 23503, classified as NotFound).
#[tokio::test]
async fn add_unknown_assignee_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "assignee-404-1").await;

    client
        .create_project(&ws.slug, project_req("assignee-404-proj", "A4"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "assignee-404-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "T".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task");

    let ghost_id = uuid::Uuid::new_v4();
    let result = client
        .add_assignee(
            &ws.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".into(),
                assignee_id: ghost_id,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "unknown user assignee must return 404, got: {result:?}"
    );

    db.teardown().await;
}

/// Adding the same assignee twice must return 409 (SQLSTATE 23505 unique violation).
/// This test is more explicit than the existing `add_duplicate_assignee_returns_409`
/// — it asserts the exact status without accepting any other value.
#[tokio::test]
async fn add_duplicate_assignee_returns_409_explicit() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "dup-assign-explicit-1").await;

    client
        .create_project(&ws.slug, project_req("dup-assign-expl-proj", "DAE"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "dup-assign-expl-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "T".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task");

    client
        .add_assignee(
            &ws.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".into(),
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
                assignee_type: "user".into(),
                assignee_id: user.id.0,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "duplicate assignee must return exactly 409, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Finding #9 — Unassigning a non-existent assignment → 404, no activity
// ---------------------------------------------------------------------------

/// Unassigning a user who was never assigned must return 404 AND must NOT
/// write any `unassigned` activity row for the task.
#[tokio::test]
async fn unassign_non_existent_returns_404_and_no_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "unassign-404-1").await;

    client
        .create_project(&ws.slug, project_req("unassign-404-proj", "UA"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "unassign-404-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "T".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task");

    let result = client
        .remove_assignee(&ws.slug, &task.readable_id, &format!("user:{}", user.id.0))
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "unassign of non-existent assignment must return 404, got: {result:?}"
    );

    let unassigned_count = count_activity_of_kind(&db, task.id, "unassigned").await;
    assert_eq!(
        unassigned_count, 0,
        "no unassigned activity must be written when the assignment did not exist"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Cross-board promote IDOR (write-side escalation guard)
// ---------------------------------------------------------------------------

/// A promote request that names a destination board different from the parent
/// task's board must be rejected with 422 and must write nothing: no task row
/// on the target board and the checklist item remains unpromoted.
#[tokio::test]
async fn promote_into_foreign_board_same_workspace_is_rejected_and_writes_no_task() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let f = setup_idor(&db, &server, "idor-fprom").await;

    // Board B's task count before the attempted promotion.
    let before = f
        .owner
        .list_tasks(&f.ws.slug, f.board_b, None, None)
        .await
        .expect("list board B tasks before");
    let count_before = before.items.len();

    // Attacker holds editor on board A (task_a lives there) but submits
    // board B as the destination — this is the write-side IDOR escalation.
    let result = f
        .attacker
        .promote_checklist_item(
            &f.ws.slug,
            &f.task_a_readable,
            f.item_a,
            PromoteChecklistItemRequest {
                board_id: f.board_b,
                column_id: f.col_b,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "promote targeting a foreign board must return 422, got: {result:?}"
    );

    // Board B must be untouched: no task was written there.
    let after = f
        .owner
        .list_tasks(&f.ws.slug, f.board_b, None, None)
        .await
        .expect("list board B tasks after");

    assert_eq!(
        after.items.len(),
        count_before,
        "board B task count must be unchanged after the rejected promote, got: {:?}",
        after.items
    );

    // The checklist item on task A must still be unpromoted.
    let checklist = f
        .attacker
        .list_checklist(&f.ws.slug, &f.task_a_readable)
        .await
        .expect("list checklist");

    let item = checklist
        .iter()
        .find(|i| i.id == f.item_a)
        .expect("item_a must still be in the checklist");

    assert!(
        item.promoted_task_id.is_none(),
        "checklist item must remain unpromoted after the rejected call, got: {:?}",
        item.promoted_task_id
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Fix 1 — target_resolved must reflect live target existence (Req 6)
// ---------------------------------------------------------------------------

/// Creating a reference to a live task must return target_resolved=true and
/// populate target_readable_id. After soft-deleting the target task,
/// list_references must return target_resolved=false for that reference.
#[tokio::test]
async fn reference_target_resolved_false_after_target_task_soft_deleted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "ref-resolved-1").await;

    client
        .create_project(&ws.slug, project_req("ref-res-proj", "RR"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "ref-res-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let source = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Source".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("source task");

    let target = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Target".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("target task");

    let created_ref = client
        .create_reference(
            &ws.slug,
            &source.readable_id,
            CreateReferenceRequest {
                kind: "blocks".into(),
                target_task_readable_id: Some(target.readable_id.clone()),
                target_document_id: None,
            },
        )
        .await
        .expect("create reference");

    assert!(
        created_ref.target_resolved,
        "create_reference must return target_resolved=true for a live target, got: {}",
        created_ref.target_resolved
    );
    assert_eq!(
        created_ref.target_readable_id.as_deref(),
        Some(target.readable_id.as_str()),
        "create_reference must populate target_readable_id with the live target's readable_id"
    );

    let refs_before = client
        .list_references(&ws.slug, &source.readable_id)
        .await
        .expect("list references before delete");

    let r = refs_before
        .iter()
        .find(|r| r.id == created_ref.id)
        .expect("reference must appear in list");
    assert!(
        r.target_resolved,
        "list_references must return target_resolved=true while target is live"
    );
    assert_eq!(
        r.target_readable_id.as_deref(),
        Some(target.readable_id.as_str()),
        "list_references must populate target_readable_id for a live task target"
    );

    client
        .delete_task(&ws.slug, &target.readable_id)
        .await
        .expect("delete target task");

    let refs_after = client
        .list_references(&ws.slug, &source.readable_id)
        .await
        .expect("list references after delete");

    let r_after = refs_after
        .iter()
        .find(|r| r.id == created_ref.id)
        .expect("reference must still appear in list after target is deleted");
    assert!(
        !r_after.target_resolved,
        "list_references must return target_resolved=false after target task is soft-deleted, got: {}",
        r_after.target_resolved
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Fix 2 — Assignee must be a member of the workspace
// ---------------------------------------------------------------------------

/// Assigning a user who exists globally but is NOT a member of this workspace
/// must return 404 and must NOT write any task_assignees row or activity.
#[tokio::test]
async fn assign_non_member_principal_returns_404() {
    use sea_orm::{ConnectionTrait, Statement};

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client_a, ws_a, _) = support::login_user_with_workspace(&server, &db, "assign-nm-a").await;

    let (_client_b, ws_b, user_b) =
        support::login_user_with_workspace(&server, &db, "assign-nm-b").await;

    assert!(
        ws_a.id != ws_b.id,
        "workspaces must be distinct for this test"
    );

    client_a
        .create_project(&ws_a.slug, project_req("assign-nm-proj", "ANM"))
        .await
        .expect("create project");

    let board = client_a
        .create_board(
            &ws_a.slug,
            "assign-nm-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client_a
        .create_column(
            &ws_a.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task = client_a
        .create_task(
            &ws_a.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "T".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task");

    let result = client_a
        .add_assignee(
            &ws_a.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".into(),
                assignee_id: user_b.id.0,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "assigning a non-member user must return 404, got: {result:?}"
    );

    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT COUNT(*) AS cnt FROM task_assignees \
                 WHERE task_id = '{}' AND assignee_user_id = '{}'",
                task.id, user_b.id.0
            ),
        ))
        .await
        .expect("query")
        .expect("row");
    let cnt: i64 = row.try_get("", "cnt").expect("cnt");
    assert_eq!(cnt, 0, "no task_assignees row must be written");

    let assigned_count = count_activity_of_kind(&db, task.id, "assigned").await;
    assert_eq!(
        assigned_count, 0,
        "no assigned activity must be written for the rejected non-member assign"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Free-text field cap tests (Fix 3)
// ---------------------------------------------------------------------------

/// Verifies that a task title longer than 200 characters is rejected with 422.
#[tokio::test]
async fn create_task_with_title_over_200_chars_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "cap-title-1").await;

    client
        .create_project(&ws.slug, project_req("cap-proj-1", "CP1"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "cap-proj-1",
            CreateBoardRequest {
                name: "B".to_string(),
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
                color: None,
            },
        )
        .await
        .expect("create column");

    let long_title = "x".repeat(201);
    let result = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: long_title,
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "title over 200 chars must return 422, got: {result:?}"
    );

    db.teardown().await;
}

/// Verifies that a whitespace-only task title is rejected with 422.
#[tokio::test]
async fn create_task_with_empty_title_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "cap-empty-1").await;

    client
        .create_project(&ws.slug, project_req("cap-empty-proj", "CE"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "cap-empty-proj",
            CreateBoardRequest {
                name: "B".to_string(),
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
                color: None,
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
                title: "   ".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "whitespace-only title must return 422, got: {result:?}"
    );

    db.teardown().await;
}

/// Verifies that too many task labels (> 50) are rejected with 422.
#[tokio::test]
async fn create_task_with_too_many_labels_returns_422() {
    use atlas_api::dtos::boards_tasks::TaskPropertiesDto;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "cap-labels-1").await;

    client
        .create_project(&ws.slug, project_req("cap-labels-proj", "CL"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "cap-labels-proj",
            CreateBoardRequest {
                name: "B".to_string(),
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
                color: None,
            },
        )
        .await
        .expect("create column");

    let labels: Vec<String> = (0..51).map(|i| format!("label-{i}")).collect();
    let result = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task".to_string(),
                description: None,
                properties: Some(TaskPropertiesDto {
                    labels,
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "more than 50 labels must return 422, got: {result:?}"
    );

    db.teardown().await;
}

/// Verifies that a document title longer than 200 characters is rejected with 422.
#[tokio::test]
async fn create_document_with_title_over_200_chars_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "cap-doc-1").await;

    client
        .create_project(&ws.slug, project_req("cap-doc-proj", "CD"))
        .await
        .expect("create project");

    let long_title = "d".repeat(201);
    let result = client
        .create_document(
            &ws.slug,
            "cap-doc-proj",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: long_title,
                folder_id: None,
                content: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "document title over 200 chars must return 422, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn update_task_clears_estimate_with_explicit_null() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-clr-est").await;

    client
        .create_project(&ws.slug, project_req("task-clr-proj", "TC"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "task-clr-proj",
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
                color: None,
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
                title: "Has estimate".to_string(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    estimate: Some(5),
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");
    assert_eq!(task.estimate, Some(5));

    // An explicit JSON null must clear the estimate (an absent field would leave it).
    let updated = client
        .update_task(
            &ws.slug,
            &task.readable_id,
            UpdateTaskRequest {
                estimate: Some(serde_json::Value::Null),
                ..Default::default()
            },
        )
        .await
        .expect("update task");

    assert_eq!(
        updated.estimate, None,
        "explicit null must clear the estimate"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Column color (B1)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_column_with_valid_color_returns_color_in_response_and_listing() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "col-color-create-1").await;

    client
        .create_project(&ws.slug, project_req("col-color-proj-1", "CC1"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-color-proj-1",
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
                name: "In Progress".to_string(),
                color: Some("blue".to_string()),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column with color");

    assert_eq!(col.color.as_deref(), Some("blue"));

    let cols = client
        .list_columns(&ws.slug, board.id)
        .await
        .expect("list columns");

    assert_eq!(cols.len(), 1);
    assert_eq!(cols[0].id, col.id);
    assert_eq!(
        cols[0].color.as_deref(),
        Some("blue"),
        "color must round-trip through listing"
    );

    db.teardown().await;
}

#[tokio::test]
async fn create_column_without_color_omits_color_field() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "col-color-absent-1").await;

    client
        .create_project(&ws.slug, project_req("col-color-proj-2", "CC2"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-color-proj-2",
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
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column without color");

    assert_eq!(col.color, None, "no color set must yield null/absent color");

    db.teardown().await;
}

#[tokio::test]
async fn patch_column_sets_color() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "col-color-patch-1").await;

    client
        .create_project(&ws.slug, project_req("col-color-proj-3", "CC3"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-color-proj-3",
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
                name: "Done".to_string(),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    assert_eq!(col.color, None);

    let updated = client
        .update_column(
            &ws.slug,
            board.id,
            col.id,
            UpdateColumnRequest {
                name: None,
                color: Some(serde_json::Value::String("green".to_string())),
                before: None,
                after: None,
            },
        )
        .await
        .expect("patch color");

    assert_eq!(
        updated.color.as_deref(),
        Some("green"),
        "color must be set after PATCH"
    );

    let cols = client
        .list_columns(&ws.slug, board.id)
        .await
        .expect("list columns");

    assert_eq!(
        cols[0].color.as_deref(),
        Some("green"),
        "color persists in listing"
    );

    db.teardown().await;
}

#[tokio::test]
async fn patch_column_clears_color_with_explicit_null() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "col-color-clear-1").await;

    client
        .create_project(&ws.slug, project_req("col-color-proj-4", "CC4"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-color-proj-4",
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
                name: "Review".to_string(),
                color: Some("amber".to_string()),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column with color");

    assert_eq!(col.color.as_deref(), Some("amber"));

    let cleared = client
        .update_column(
            &ws.slug,
            board.id,
            col.id,
            UpdateColumnRequest {
                name: None,
                color: Some(serde_json::Value::Null),
                before: None,
                after: None,
            },
        )
        .await
        .expect("clear color with explicit null");

    assert_eq!(cleared.color, None, "explicit null must clear the color");

    let cols = client
        .list_columns(&ws.slug, board.id)
        .await
        .expect("list columns");

    assert_eq!(
        cols[0].color, None,
        "cleared color must be absent in listing"
    );

    db.teardown().await;
}

#[tokio::test]
async fn patch_column_absent_color_leaves_color_unchanged() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "col-color-unchanged-1").await;

    client
        .create_project(&ws.slug, project_req("col-color-proj-5", "CC5"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-color-proj-5",
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
                name: "Staging".to_string(),
                color: Some("cyan".to_string()),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column with color");

    let renamed = client
        .update_column(
            &ws.slug,
            board.id,
            col.id,
            UpdateColumnRequest {
                name: Some("Staging (renamed)".to_string()),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("rename without touching color");

    assert_eq!(renamed.name, "Staging (renamed)");
    assert_eq!(
        renamed.color.as_deref(),
        Some("cyan"),
        "absent color field must leave color unchanged"
    );

    db.teardown().await;
}

#[tokio::test]
async fn create_column_with_invalid_swatch_id_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "col-color-invalid-create-1").await;

    client
        .create_project(&ws.slug, project_req("col-color-proj-6", "CC6"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-color-proj-6",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let result = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Bad Color".to_string(),
                color: Some("hotpink".to_string()),
                before: None,
                after: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "invalid swatch id must return 422, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn patch_column_with_invalid_swatch_id_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "col-color-invalid-patch-1").await;

    client
        .create_project(&ws.slug, project_req("col-color-proj-7", "CC7"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "col-color-proj-7",
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
                name: "Valid".to_string(),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let result = client
        .update_column(
            &ws.slug,
            board.id,
            col.id,
            UpdateColumnRequest {
                name: None,
                color: Some(serde_json::Value::String("hotpink".to_string())),
                before: None,
                after: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "invalid swatch id on PATCH must return 422, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Revoked api_key assignee visibility (bug fix)
// ---------------------------------------------------------------------------

/// A revoked api_key's task assignment must NOT appear in list_assignees after
/// revocation, and the task_assignees row must be deleted atomically with the
/// revoke operation.
#[tokio::test]
async fn revoked_api_key_assignee_is_hidden_after_revoke() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "rvk-key-asgn-1").await;

    client
        .create_project(&ws.slug, project_req("rvk-proj", "RVK"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "rvk-proj",
            CreateBoardRequest { name: "B".into() },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "C".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task with api key assignee".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task");

    let api_key = client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "agent-to-revoke".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: None,
        })
        .await
        .expect("create api key");

    client
        .add_assignee(
            &ws.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "api_key".into(),
                assignee_id: api_key.id,
            },
        )
        .await
        .expect("add api key assignee");

    // Verify the assignee is present before revoke.
    let before = client
        .list_assignees(&ws.slug, &task.readable_id)
        .await
        .expect("list assignees before revoke");
    assert_eq!(before.len(), 1, "one assignee before revoke");
    assert_eq!(before[0].assignee.id, api_key.id);

    // Revoke the key — this should atomically delete its task_assignees rows.
    client
        .revoke_user_api_key(api_key.id)
        .await
        .expect("revoke api key");

    // Part A: list_assignees must return empty (read-side filter).
    let after = client
        .list_assignees(&ws.slug, &task.readable_id)
        .await
        .expect("list assignees after revoke");
    assert!(
        after.is_empty(),
        "revoked key assignee must not appear in list_assignees, got: {after:?}"
    );

    // Part A: list_tasks (board) must not include the revoked assignee.
    let page = client
        .list_tasks(&ws.slug, board.id, None, None)
        .await
        .expect("list tasks after revoke");
    let summary = page.items.first().expect("one task");
    assert!(
        summary.assignees.is_empty(),
        "revoked key assignee must not appear in list_tasks summary, got: {:?}",
        summary.assignees
    );

    // Part B: the task_assignees row must be physically deleted by revoke.
    use sea_orm::{ConnectionTrait, Statement};
    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT COUNT(*) AS cnt FROM task_assignees \
                 WHERE task_id = '{}' AND assignee_api_key_id = '{}'",
                task.id, api_key.id
            ),
        ))
        .await
        .expect("query")
        .expect("row");
    let cnt: i64 = row.try_get("", "cnt").expect("cnt");
    assert_eq!(cnt, 0, "task_assignees row must be deleted on revoke");

    db.teardown().await;
}
