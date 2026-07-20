#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::boards_tasks::{
    AddAssigneeRequest, CreateBoardRequest, CreateColumnRequest, CreateSubtaskRequest,
    CreateTaskRequest, WorkspaceTaskQueryParams,
};
use atlas_api::dtos::{
    ApiKeyScope, CreateProjectRequest, CreateUserApiKeyRequest, InitialGrantRequest,
};
use atlas_client::ClientError;

// ---------------------------------------------------------------------------
// Seed helpers
// ---------------------------------------------------------------------------

fn project_req(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: None,
        visibility_role: None,
    }
}

/// Seed data for workspace task listing tests.
#[allow(dead_code)]
struct WorkspaceSeed {
    ws_slug: String,
    user_client: atlas_client::AtlasClient,
    api_key_client: atlas_client::AtlasClient,
    api_key_id: uuid::Uuid,
    board1_id: uuid::Uuid,
    board2_id: uuid::Uuid,
    col1_id: uuid::Uuid,
    col2_id: uuid::Uuid,
    /// task ids created by the user
    user_task_ids: Vec<uuid::Uuid>,
    /// task ids created by the api key
    key_task_ids: Vec<uuid::Uuid>,
}

async fn seed_workspace(
    server: &support::TestServer,
    db: &support::TestDb,
    username: &str,
) -> WorkspaceSeed {
    let (user_client, ws, user) = support::login_user_with_workspace(server, db, username).await;

    let key_created = user_client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "test-agent".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![
                ApiKeyScope::TasksRead,
                ApiKeyScope::TasksCreate,
                ApiKeyScope::TasksUpdate,
                ApiKeyScope::TasksDelete,
            ]),
        })
        .await
        .expect("create api key with workspace grant");

    let api_key_id = key_created.id;
    let api_key_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(key_created.secret.clone());

    // Create a project + two boards + two columns each
    let project = user_client
        .create_project(&ws.slug, project_req(&format!("{username}-p1"), "TSK"))
        .await
        .expect("create project");

    let board1 = user_client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                folder_id: None,
                name: "Board 1".to_string(),
            },
        )
        .await
        .expect("create board 1");

    let board2 = user_client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                folder_id: None,
                name: "Board 2".to_string(),
            },
        )
        .await
        .expect("create board 2");

    // Create two columns on board1 and one column on board2
    let col1 = user_client
        .create_column(
            &ws.slug,
            board1.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create col1 board1");
    let col1_id = col1.id;

    let col2 = user_client
        .create_column(
            &ws.slug,
            board1.id,
            CreateColumnRequest {
                name: "In Progress".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create col2 board1");
    let col2_id = col2.id;

    let col3 = user_client
        .create_column(
            &ws.slug,
            board2.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create col3 board2");
    let col3_id = col3.id;

    // Create tasks as the user on board1
    let t1 = user_client
        .create_task(
            &ws.slug,
            board1.id,
            CreateTaskRequest {
                column_id: col1_id,
                title: "User task 1".to_string(),
                description: None,
                before: None,
                after: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    priority: Some("high".to_string()),
                    labels: vec!["alpha".to_string(), "beta".to_string()],
                    ..Default::default()
                }),
            },
        )
        .await
        .expect("create user task 1");

    let t2 = user_client
        .create_task(
            &ws.slug,
            board1.id,
            CreateTaskRequest {
                column_id: col2_id,
                title: "User task 2".to_string(),
                description: None,
                before: None,
                after: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    priority: Some("low".to_string()),
                    labels: vec!["alpha".to_string()],
                    ..Default::default()
                }),
            },
        )
        .await
        .expect("create user task 2");

    // Create tasks as the api key on board2
    let t3 = api_key_client
        .create_task(
            &ws.slug,
            board2.id,
            CreateTaskRequest {
                column_id: col3_id,
                title: "Agent task 1".to_string(),
                description: None,
                before: None,
                after: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    priority: Some("urgent".to_string()),
                    labels: vec!["gamma".to_string()],
                    ..Default::default()
                }),
            },
        )
        .await
        .expect("create agent task 1");

    let t4 = api_key_client
        .create_task(
            &ws.slug,
            board2.id,
            CreateTaskRequest {
                column_id: col3_id,
                title: "Agent task 2".to_string(),
                description: None,
                before: None,
                after: None,
                properties: None,
            },
        )
        .await
        .expect("create agent task 2");

    // Assign t1 to the user (user is assignee)
    user_client
        .add_assignee(
            &ws.slug,
            &t1.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: user.id.0,
            },
        )
        .await
        .expect("assign t1 to user");

    // Assign t3 to the api key
    user_client
        .add_assignee(
            &ws.slug,
            &t3.readable_id,
            AddAssigneeRequest {
                assignee_type: "api_key".to_string(),
                assignee_id: api_key_id,
            },
        )
        .await
        .expect("assign t3 to api key");

    WorkspaceSeed {
        ws_slug: ws.slug,
        user_client,
        api_key_client,
        api_key_id,
        board1_id: board1.id,
        board2_id: board2.id,
        col1_id,
        col2_id,
        user_task_ids: vec![t1.id, t2.id],
        key_task_ids: vec![t3.id, t4.id],
    }
}

// ---------------------------------------------------------------------------
// TW01: no params → all workspace top-level tasks across boards (default updated_at_desc)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_no_params_returns_all_toplevel_tasks_across_boards() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw01-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(&seed.ws_slug, &Default::default())
        .await
        .expect("list workspace tasks");

    let all_ids: Vec<uuid::Uuid> = page.items.iter().map(|t| t.id).collect();
    assert_eq!(all_ids.len(), 4, "all 4 tasks must appear");

    for id in seed.user_task_ids.iter().chain(seed.key_task_ids.iter()) {
        assert!(all_ids.contains(id), "task {id} must be in the listing");
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW02: assignee=me → only tasks assigned to caller (My tasks view)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_assignee_me_returns_only_assigned_tasks() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw02-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                assignee: Some("me".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks assignee=me");

    assert_eq!(
        page.items.len(),
        1,
        "only tasks assigned to user must appear"
    );
    assert_eq!(page.items[0].id, seed.user_task_ids[0]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW03: actor=api_key → only api_key-created tasks (Agent activity view)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_actor_api_key_returns_only_agent_created_tasks() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw03-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                actor: Some("api_key".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks actor=api_key");

    let ids: Vec<uuid::Uuid> = page.items.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 2, "only api_key-created tasks must appear");
    for id in &seed.key_task_ids {
        assert!(ids.contains(id));
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW04: sort=updated_at_desc orders correctly across boards (Recently updated)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_sort_updated_desc_orders_correctly_across_boards() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw04-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                sort: Some("updated_at_desc".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks sort=updated_at_desc");

    assert_eq!(page.items.len(), 4);

    let times: Vec<_> = page.items.iter().map(|t| t.updated_at).collect();
    let sorted_desc = times.windows(2).all(|w| w[0] >= w[1]);
    assert!(sorted_desc, "items must be in descending updated_at order");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW05: board_id scopes to single board
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_board_id_scopes_to_single_board() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw05-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                board_id: Some(seed.board1_id.to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks board_id filter");

    let ids: Vec<uuid::Uuid> = page.items.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 2, "only board1 tasks must appear");
    for id in &seed.user_task_ids {
        assert!(ids.contains(id));
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW06: column_id filter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_column_id_filter() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw06-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                column_ids: vec![seed.col1_id.to_string()],
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks column_id filter");

    let ids: Vec<uuid::Uuid> = page.items.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), 1);
    assert_eq!(ids[0], seed.user_task_ids[0]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW07: priority filter (repeated param)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_priority_filter_repeated_param() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw07-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                priorities: vec!["high".to_string(), "urgent".to_string()],
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks priority filter");

    let ids: Vec<uuid::Uuid> = page.items.iter().map(|t| t.id).collect();
    assert_eq!(
        ids.len(),
        2,
        "high + urgent tasks: user_task_ids[0] and key_task_ids[0]"
    );
    assert!(ids.contains(&seed.user_task_ids[0]));
    assert!(ids.contains(&seed.key_task_ids[0]));

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW08: label filter (array-contains ALL labels)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_label_filter_array_contains_all() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw08-user").await;

    // Only user_task_ids[0] has BOTH "alpha" AND "beta"
    let page = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                labels: vec!["alpha".to_string(), "beta".to_string()],
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks label filter");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, seed.user_task_ids[0]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW09: combined filters are ANDed together
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_combined_filters_and_together() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw09-user").await;

    // board1 AND priority=high → only user_task_ids[0]
    let page = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                board_id: Some(seed.board1_id.to_string()),
                priorities: vec!["high".to_string()],
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks combined filters");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, seed.user_task_ids[0]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW10: invalid sort → 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_invalid_sort_returns_400() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw10-user").await;

    let result = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                sort: Some("'; DROP TABLE tasks; --".to_string()),
                ..Default::default()
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 400),
        "invalid sort must return 400, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW11: pagination — no overlap between pages, has_more correct
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_pagination_no_overlap_and_has_more_correct() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw11-user").await;

    // First page: limit=2
    let page1 = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                limit: Some(2),
                sort: Some("updated_at_desc".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("page 1");

    assert_eq!(page1.items.len(), 2);
    assert!(
        page1.has_more,
        "has_more must be true when more pages remain"
    );
    assert!(page1.next_cursor.is_some(), "next_cursor must be set");

    let ids_page1: Vec<uuid::Uuid> = page1.items.iter().map(|t| t.id).collect();

    // Second page
    let page2 = seed
        .user_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                limit: Some(2),
                sort: Some("updated_at_desc".to_string()),
                cursor: page1.next_cursor.clone(),
                ..Default::default()
            },
        )
        .await
        .expect("page 2");

    assert_eq!(page2.items.len(), 2);
    assert!(!page2.has_more, "has_more must be false on last page");

    let ids_page2: Vec<uuid::Uuid> = page2.items.iter().map(|t| t.id).collect();

    // No overlap
    for id in &ids_page1 {
        assert!(!ids_page2.contains(id), "task {id} appeared on both pages");
    }

    // Together they cover all 4 tasks
    let mut all: Vec<uuid::Uuid> = ids_page1.iter().chain(ids_page2.iter()).copied().collect();
    all.sort_unstable();
    let mut expected: Vec<uuid::Uuid> = seed
        .user_task_ids
        .iter()
        .chain(seed.key_task_ids.iter())
        .copied()
        .collect();
    expected.sort_unstable();
    assert_eq!(
        all, expected,
        "the two pages must cover all tasks exactly once"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW12: sub-tasks (parent_task_id IS NOT NULL) are excluded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_excludes_subtasks() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw12-user").await;

    // Create a subtask under user_task_ids[0]
    let parent_rid = {
        let page = seed
            .user_client
            .list_workspace_tasks(&seed.ws_slug, &Default::default())
            .await
            .expect("list");
        page.items
            .iter()
            .find(|t| t.id == seed.user_task_ids[0])
            .map(|t| t.readable_id.clone())
            .expect("parent readable_id")
    };

    seed.user_client
        .create_subtask(
            &seed.ws_slug,
            &parent_rid,
            CreateSubtaskRequest {
                title: "A subtask".to_string(),
            },
        )
        .await
        .expect("create subtask");

    let page = seed
        .user_client
        .list_workspace_tasks(&seed.ws_slug, &Default::default())
        .await
        .expect("list after subtask");

    assert_eq!(
        page.items.len(),
        4,
        "subtask must not appear in workspace listing"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW13: soft-deleted tasks are excluded
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_excludes_soft_deleted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw13-user").await;

    let page_before = seed
        .user_client
        .list_workspace_tasks(&seed.ws_slug, &Default::default())
        .await
        .expect("list before delete");
    assert_eq!(page_before.items.len(), 4);

    // Delete user_task_ids[1]
    let rid = page_before
        .items
        .iter()
        .find(|t| t.id == seed.user_task_ids[1])
        .map(|t| t.readable_id.clone())
        .expect("task readable_id");

    seed.user_client
        .delete_task(&seed.ws_slug, &rid)
        .await
        .expect("delete task");

    let page_after = seed
        .user_client
        .list_workspace_tasks(&seed.ws_slug, &Default::default())
        .await
        .expect("list after delete");

    assert_eq!(
        page_after.items.len(),
        3,
        "soft-deleted task must not appear"
    );
    let ids: Vec<uuid::Uuid> = page_after.items.iter().map(|t| t.id).collect();
    assert!(!ids.contains(&seed.user_task_ids[1]));

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW14: cross-workspace isolation — tasks from another workspace never appear
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_cross_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let seed_a = seed_workspace(&server, &db, "tw14-user-a").await;
    let seed_b = seed_workspace(&server, &db, "tw14-user-b").await;

    let page_a = seed_a
        .user_client
        .list_workspace_tasks(&seed_a.ws_slug, &Default::default())
        .await
        .expect("list ws_a tasks as user_a");

    let page_b = seed_b
        .user_client
        .list_workspace_tasks(&seed_b.ws_slug, &Default::default())
        .await
        .expect("list ws_b tasks as user_b");

    let ids_a: Vec<uuid::Uuid> = page_a.items.iter().map(|t| t.id).collect();
    let ids_b: Vec<uuid::Uuid> = page_b.items.iter().map(|t| t.id).collect();

    // No id from ws_b should appear in ws_a and vice versa
    for id in &ids_b {
        assert!(
            !ids_a.contains(id),
            "ws_b task {id} appeared in ws_a listing"
        );
    }
    for id in &ids_a {
        assert!(
            !ids_b.contains(id),
            "ws_a task {id} appeared in ws_b listing"
        );
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW15: unauthenticated → 401
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_rejects_unauthenticated() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw15-user").await;

    let anon = atlas_client::AtlasClient::new(server.base_url().to_string());
    let result = anon
        .list_workspace_tasks(&seed.ws_slug, &Default::default())
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated must return 401, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW16: non-member → 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_returns_404_for_non_member() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let seed = seed_workspace(&server, &db, "tw16-user").await;

    // Create a second user who is NOT a member of the workspace
    let (outsider, _) = support::login_user(&server, &db, "tw16-outsider").await;

    let result = outsider
        .list_workspace_tasks(&seed.ws_slug, &Default::default())
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-member must return 404, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW17: assignee=me as an api_key principal resolves against assignee_api_key_id
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_assignee_me_as_api_key_resolves_against_assignee_api_key_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw17-user").await;

    // t3 (key_task_ids[0]) is assigned to the api key
    let page = seed
        .api_key_client
        .list_workspace_tasks(
            &seed.ws_slug,
            &WorkspaceTaskQueryParams {
                assignee: Some("me".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("list workspace tasks assignee=me as api key");

    assert_eq!(
        page.items.len(),
        1,
        "only the task assigned to the api key must appear"
    );
    assert_eq!(page.items[0].id, seed.key_task_ids[0]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW18: summaries carry board_name and column_name; cross-board rows are distinct
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_summaries_carry_board_name_and_column_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw18-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(&seed.ws_slug, &Default::default())
        .await
        .expect("list workspace tasks");

    assert_eq!(page.items.len(), 4);

    // user_task_ids[0] → Board 1 / col1 (Todo)
    let t1 = page
        .items
        .iter()
        .find(|t| t.id == seed.user_task_ids[0])
        .expect("user_task_ids[0] must appear");
    assert_eq!(t1.board_name, "Board 1");
    assert_eq!(t1.column_name, "Todo");

    // user_task_ids[1] → Board 1 / col2 (In Progress)
    let t2 = page
        .items
        .iter()
        .find(|t| t.id == seed.user_task_ids[1])
        .expect("user_task_ids[1] must appear");
    assert_eq!(t2.board_name, "Board 1");
    assert_eq!(t2.column_name, "In Progress");

    // key_task_ids[0] and [1] → Board 2 / Todo
    let t3 = page
        .items
        .iter()
        .find(|t| t.id == seed.key_task_ids[0])
        .expect("key_task_ids[0] must appear");
    assert_eq!(t3.board_name, "Board 2");
    assert_eq!(t3.column_name, "Todo");

    // Cross-board: t1 and t3 have distinct board_names
    assert_ne!(
        t1.board_name, t3.board_name,
        "tasks on different boards must have distinct board_names"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// TW19: summaries carry board_id; cross-board rows have DISTINCT board_ids
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_workspace_tasks_summaries_carry_board_id_and_cross_board_ids_are_distinct() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let seed = seed_workspace(&server, &db, "tw19-user").await;

    let page = seed
        .user_client
        .list_workspace_tasks(&seed.ws_slug, &Default::default())
        .await
        .expect("list workspace tasks");

    assert_eq!(page.items.len(), 4);

    let t1 = page
        .items
        .iter()
        .find(|t| t.id == seed.user_task_ids[0])
        .expect("user_task_ids[0] must appear");

    let t3 = page
        .items
        .iter()
        .find(|t| t.id == seed.key_task_ids[0])
        .expect("key_task_ids[0] must appear");

    assert_eq!(
        t1.board_id, seed.board1_id,
        "user task must carry board1_id"
    );
    assert_eq!(
        t3.board_id, seed.board2_id,
        "agent task must carry board2_id"
    );
    assert_ne!(
        t1.board_id, t3.board_id,
        "tasks on different boards must have distinct board_ids"
    );

    db.teardown().await;
}
