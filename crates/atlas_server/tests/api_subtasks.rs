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
        CreateBoardRequest, CreateColumnRequest, CreateSubtaskRequest, CreateTaskRequest,
        SetTaskParentRequest, TaskPropertiesDto, UpdateTaskRequest,
    },
};
use atlas_client::AtlasClient;

fn project_req(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: None,
        visibility_role: None,
    }
}

/// Creates a board with a single `Todo` column in an existing project; returns
/// the board id and that column's id.
async fn board_with_column(
    client: &AtlasClient,
    ws_slug: &str,
    proj: &str,
    name: &str,
) -> (uuid::Uuid, uuid::Uuid) {
    let board = client
        .acta()
        .create_board(
            ws_slug,
            proj,
            CreateBoardRequest {
                folder_id: None,
                name: name.to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .acta()
        .create_column(
            ws_slug,
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

    (board.id, col.id)
}

/// Sets up a project, board, single column and one parent task; returns the
/// workspace slug, board id, and the created parent task.
async fn setup_parent(
    client: &AtlasClient,
    ws_slug: &str,
    proj: &str,
    prefix: &str,
) -> (uuid::Uuid, atlas_api::dtos::boards_tasks::TaskDto) {
    client
        .acta()
        .create_project(ws_slug, project_req(proj, prefix))
        .await
        .expect("create project");

    let (board_id, column_id) = board_with_column(client, ws_slug, proj, "Board").await;

    let parent = create_task(client, ws_slug, board_id, column_id, "Parent").await;

    (board_id, parent)
}

/// Creates a plain top-level task with only a title.
async fn create_task(
    client: &AtlasClient,
    ws_slug: &str,
    board_id: uuid::Uuid,
    column_id: uuid::Uuid,
    title: &str,
) -> atlas_api::dtos::boards_tasks::TaskDto {
    client
        .acta()
        .create_task(
            ws_slug,
            board_id,
            CreateTaskRequest {
                references: vec![],
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

#[tokio::test]
async fn subtask_is_excluded_from_board_and_listed_under_parent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "subtask-1").await;

    let (board_id, parent) = setup_parent(&client, &ws.slug, "subtask-proj", "ST").await;

    let sub = client
        .acta()
        .create_subtask(
            &ws.slug,
            &parent.readable_id,
            CreateSubtaskRequest::titled("Child"),
        )
        .await
        .expect("create subtask")
        .task;

    assert_eq!(sub.parent_task_id, Some(parent.id));
    assert_eq!(sub.title, "Child");
    assert_eq!(sub.column_id, parent.column_id, "inherits parent column");

    // The board listing must NOT include the sub-task.
    let board_tasks = client
        .acta()
        .list_tasks(&ws.slug, board_id, None, None)
        .await
        .expect("list board tasks");
    let ids: Vec<uuid::Uuid> = board_tasks.items.iter().map(|t| t.id).collect();
    assert!(ids.contains(&parent.id), "parent stays on the board");
    assert!(!ids.contains(&sub.id), "sub-task is hidden from the board");

    // The sub-task list under the parent must include it.
    let subs = client
        .acta()
        .list_subtasks(&ws.slug, &parent.readable_id)
        .await
        .expect("list subtasks");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].id, sub.id);
    assert_eq!(subs[0].title, "Child");

    db.teardown().await;
}

#[tokio::test]
async fn promote_subtask_moves_it_onto_the_board() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "subtask-2").await;

    let (board_id, parent) = setup_parent(&client, &ws.slug, "promote-proj", "PR").await;

    let sub = client
        .acta()
        .create_subtask(
            &ws.slug,
            &parent.readable_id,
            CreateSubtaskRequest::titled("Soon a task"),
        )
        .await
        .expect("create subtask")
        .task;

    let promoted = client
        .acta()
        .promote_subtask(&ws.slug, &sub.readable_id)
        .await
        .expect("promote subtask");
    assert_eq!(promoted.parent_task_id, None, "parent cleared on promote");

    let board_tasks = client
        .acta()
        .list_tasks(&ws.slug, board_id, None, None)
        .await
        .expect("list board tasks");
    let ids: Vec<uuid::Uuid> = board_tasks.items.iter().map(|t| t.id).collect();
    assert!(
        ids.contains(&sub.id),
        "promoted task now appears on the board"
    );

    let subs = client
        .acta()
        .list_subtasks(&ws.slug, &parent.readable_id)
        .await
        .expect("list subtasks");
    assert!(subs.is_empty(), "no longer a sub-task of the parent");

    db.teardown().await;
}

#[tokio::test]
async fn subtask_behaves_like_a_full_task() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "subtask-3").await;

    let (_board_id, parent) = setup_parent(&client, &ws.slug, "full-proj", "FT").await;

    let sub = client
        .acta()
        .create_subtask(
            &ws.slug,
            &parent.readable_id,
            CreateSubtaskRequest::titled("Rich child"),
        )
        .await
        .expect("create subtask")
        .task;

    // A sub-task is a real task: it can be patched (description, estimate) exactly
    // like a board task, addressed by its own readable_id.
    let updated = client
        .acta()
        .update_task(
            &ws.slug,
            &sub.readable_id,
            UpdateTaskRequest {
                description: Some("A detailed sub-task".to_string()),
                estimate: Some(serde_json::json!(5)),
                ..Default::default()
            },
        )
        .await
        .expect("patch subtask");

    assert_eq!(updated.description, "A detailed sub-task");
    assert_eq!(updated.estimate, Some(5));
    assert_eq!(updated.parent_task_id, Some(parent.id), "stays a sub-task");

    // The inline sub-task summary surfaces the estimate.
    let subs = client
        .acta()
        .list_subtasks(&ws.slug, &parent.readable_id)
        .await
        .expect("list subtasks");
    assert_eq!(subs[0].estimate, Some(5));

    db.teardown().await;
}

#[tokio::test]
async fn subtask_is_created_with_the_same_details_as_a_task() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "subtask-4").await;

    let (board_id, parent) = setup_parent(&client, &ws.slug, "detail-proj", "DT").await;

    let doing = client
        .acta()
        .create_column(
            &ws.slug,
            board_id,
            CreateColumnRequest {
                name: "Doing".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create second column");

    let sibling = create_task(&client, &ws.slug, board_id, doing.id, "Sibling").await;

    let created = client
        .acta()
        .create_subtask(
            &ws.slug,
            &parent.readable_id,
            CreateSubtaskRequest {
                title: "Detailed child".to_string(),
                column_id: Some(doing.id),
                description: Some("Body of the child".to_string()),
                properties: Some(TaskPropertiesDto {
                    priority: Some("high".to_string()),
                    due_date: None,
                    estimate: Some(8),
                    labels: vec!["backend".to_string()],
                    custom: None,
                }),
                before: None,
                after: None,
                references: vec![atlas_api::dtos::boards_tasks::CreateReferenceRequest {
                    kind: "relates".to_string(),
                    target_task_readable_id: Some(sibling.readable_id.clone()),
                    target_document_id: None,
                }],
            },
        )
        .await
        .expect("create detailed subtask");

    let sub = created.task;
    assert_eq!(sub.parent_task_id, Some(parent.id));
    assert_eq!(sub.description, "Body of the child");
    assert_eq!(sub.priority.as_deref(), Some("high"));
    assert_eq!(sub.estimate, Some(8));
    assert_eq!(sub.labels, vec!["backend".to_string()]);
    assert_eq!(
        sub.column_id, doing.id,
        "explicit column wins over the parent's"
    );

    assert_eq!(created.references.len(), 1);
    assert_eq!(
        created.references[0].target_readable_id.as_deref(),
        Some(sibling.readable_id.as_str())
    );

    db.teardown().await;
}

#[tokio::test]
async fn existing_task_is_converted_into_a_subtask_and_back() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "subtask-5").await;

    let (board_id, parent) = setup_parent(&client, &ws.slug, "convert-proj", "CV").await;

    let board_tasks = client
        .acta()
        .list_tasks(&ws.slug, board_id, None, None)
        .await
        .expect("list board tasks");
    let column_id = board_tasks.items[0].column_id;

    let standalone = create_task(&client, &ws.slug, board_id, column_id, "Standalone").await;

    let converted = client
        .acta()
        .set_task_parent(
            &ws.slug,
            &standalone.readable_id,
            SetTaskParentRequest {
                parent_readable_id: parent.readable_id.clone(),
            },
        )
        .await
        .expect("convert to subtask");

    assert_eq!(converted.parent_task_id, Some(parent.id));
    assert_eq!(
        converted.column_id, standalone.column_id,
        "conversion keeps the task's own column"
    );

    let subs = client
        .acta()
        .list_subtasks(&ws.slug, &parent.readable_id)
        .await
        .expect("list subtasks");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].id, standalone.id);

    let ids: Vec<uuid::Uuid> = client
        .acta()
        .list_tasks(&ws.slug, board_id, None, None)
        .await
        .expect("list board tasks")
        .items
        .iter()
        .map(|t| t.id)
        .collect();
    assert!(
        !ids.contains(&standalone.id),
        "converted task leaves the board listing"
    );

    // Re-applying the same parent is a no-op rather than an error.
    client
        .acta()
        .set_task_parent(
            &ws.slug,
            &standalone.readable_id,
            SetTaskParentRequest {
                parent_readable_id: parent.readable_id.clone(),
            },
        )
        .await
        .expect("re-parenting to the same parent is idempotent");

    let promoted = client
        .acta()
        .promote_subtask(&ws.slug, &standalone.readable_id)
        .await
        .expect("promote back");
    assert_eq!(promoted.parent_task_id, None);

    db.teardown().await;
}

#[tokio::test]
async fn conversion_allows_a_parent_on_another_board() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "subtask-6").await;

    let (_board_id, parent) = setup_parent(&client, &ws.slug, "cross-proj", "CB").await;

    let (other_board, other_column) =
        board_with_column(&client, &ws.slug, "cross-proj", "Other board").await;
    let elsewhere = create_task(&client, &ws.slug, other_board, other_column, "Elsewhere").await;

    let converted = client
        .acta()
        .set_task_parent(
            &ws.slug,
            &elsewhere.readable_id,
            SetTaskParentRequest {
                parent_readable_id: parent.readable_id.clone(),
            },
        )
        .await
        .expect("convert across boards");

    assert_eq!(converted.parent_task_id, Some(parent.id));
    assert_eq!(
        converted.board_id, other_board,
        "the sub-task keeps its own board"
    );

    let subs = client
        .acta()
        .list_subtasks(&ws.slug, &parent.readable_id)
        .await
        .expect("list subtasks");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].board_id, other_board);

    db.teardown().await;
}

#[tokio::test]
async fn conversion_rejects_self_parenting_and_cycles() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "subtask-7").await;

    let (_board_id, parent) = setup_parent(&client, &ws.slug, "cycle-proj", "CY").await;

    let child = client
        .acta()
        .create_subtask(
            &ws.slug,
            &parent.readable_id,
            CreateSubtaskRequest::titled("Child"),
        )
        .await
        .expect("create subtask")
        .task;

    let self_parent = client
        .acta()
        .set_task_parent(
            &ws.slug,
            &parent.readable_id,
            SetTaskParentRequest {
                parent_readable_id: parent.readable_id.clone(),
            },
        )
        .await;
    assert!(self_parent.is_err(), "a task cannot be its own parent");

    let cycle = client
        .acta()
        .set_task_parent(
            &ws.slug,
            &parent.readable_id,
            SetTaskParentRequest {
                parent_readable_id: child.readable_id.clone(),
            },
        )
        .await;
    assert!(cycle.is_err(), "cannot re-parent under a own descendant");

    // The rejected attempts left the hierarchy untouched.
    let subs = client
        .acta()
        .list_subtasks(&ws.slug, &parent.readable_id)
        .await
        .expect("list subtasks");
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].id, child.id);

    db.teardown().await;
}
