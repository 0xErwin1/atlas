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
        UpdateTaskRequest,
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

/// Sets up a project, board, single column and one parent task; returns the
/// workspace slug, board id, and the created parent task.
async fn setup_parent(
    client: &AtlasClient,
    ws_slug: &str,
    proj: &str,
    prefix: &str,
) -> (uuid::Uuid, atlas_api::dtos::boards_tasks::TaskDto) {
    client
        .create_project(ws_slug, project_req(proj, prefix))
        .await
        .expect("create project");

    let board = client
        .create_board(
            ws_slug,
            proj,
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            ws_slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let parent = client
        .create_task(
            ws_slug,
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
        .expect("create parent task");

    (board.id, parent)
}

#[tokio::test]
async fn subtask_is_excluded_from_board_and_listed_under_parent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "subtask-1").await;

    let (board_id, parent) = setup_parent(&client, &ws.slug, "subtask-proj", "ST").await;

    let sub = client
        .create_subtask(
            &ws.slug,
            &parent.readable_id,
            CreateSubtaskRequest {
                title: "Child".to_string(),
            },
        )
        .await
        .expect("create subtask");

    assert_eq!(sub.parent_task_id, Some(parent.id));
    assert_eq!(sub.title, "Child");
    assert_eq!(sub.column_id, parent.column_id, "inherits parent column");

    // The board listing must NOT include the sub-task.
    let board_tasks = client
        .list_tasks(&ws.slug, board_id, None, None)
        .await
        .expect("list board tasks");
    let ids: Vec<uuid::Uuid> = board_tasks.items.iter().map(|t| t.id).collect();
    assert!(ids.contains(&parent.id), "parent stays on the board");
    assert!(!ids.contains(&sub.id), "sub-task is hidden from the board");

    // The sub-task list under the parent must include it.
    let subs = client
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
        .create_subtask(
            &ws.slug,
            &parent.readable_id,
            CreateSubtaskRequest {
                title: "Soon a task".to_string(),
            },
        )
        .await
        .expect("create subtask");

    let promoted = client
        .promote_subtask(&ws.slug, &sub.readable_id)
        .await
        .expect("promote subtask");
    assert_eq!(promoted.parent_task_id, None, "parent cleared on promote");

    let board_tasks = client
        .list_tasks(&ws.slug, board_id, None, None)
        .await
        .expect("list board tasks");
    let ids: Vec<uuid::Uuid> = board_tasks.items.iter().map(|t| t.id).collect();
    assert!(ids.contains(&sub.id), "promoted task now appears on the board");

    let subs = client
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
        .create_subtask(
            &ws.slug,
            &parent.readable_id,
            CreateSubtaskRequest {
                title: "Rich child".to_string(),
            },
        )
        .await
        .expect("create subtask");

    // A sub-task is a real task: it can be patched (description, estimate) exactly
    // like a board task, addressed by its own readable_id.
    let updated = client
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
        .list_subtasks(&ws.slug, &parent.readable_id)
        .await
        .expect("list subtasks");
    assert_eq!(subs[0].estimate, Some(5));

    db.teardown().await;
}
