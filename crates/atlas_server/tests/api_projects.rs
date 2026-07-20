#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateProjectRequest, UpdateProjectRequest,
    boards_tasks::{CreateBoardRequest, CreateColumnRequest, CreateTaskRequest},
};
use atlas_client::ClientError;
use support::{TestDb, TestServer, login_root_user, login_user_with_workspace};

fn project_req(name: &str, slug: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: name.to_string(),
        slug: slug.to_string(),
        task_prefix: "TST".to_string(),
        visibility: None,
        visibility_role: None,
    }
}

#[tokio::test]
async fn create_project_succeeds_for_workspace_member() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _) = login_user_with_workspace(&server, &db, "proj-owner").await;

    let project = client
        .create_project(&ws.slug, project_req("My Project", "my-project"))
        .await
        .expect("create project");

    assert_eq!(project.name, "My Project");
    assert_eq!(project.slug, "my-project");
    assert_eq!(project.workspace_id, ws.id.0);
    assert_eq!(project.visibility, "workspace");
}

#[tokio::test]
async fn list_projects_returns_created_project() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _) = login_user_with_workspace(&server, &db, "proj-owner2").await;

    client
        .create_project(&ws.slug, project_req("Listed Project", "listed-proj"))
        .await
        .expect("create project");

    let page = client
        .list_projects(&ws.slug, None, None)
        .await
        .expect("list projects");

    assert!(
        page.items.iter().any(|p| p.slug == "listed-proj"),
        "created project should appear in list"
    );
}

#[tokio::test]
async fn root_user_lists_private_project_without_membership() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_client, ws, _) = login_user_with_workspace(&server, &db, "proj-root-list").await;
    let root_client = login_root_user(&server, &db).await;

    owner_client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Private Project".to_string(),
                slug: "private-project".to_string(),
                task_prefix: "PVT".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create private project");

    let page = root_client
        .list_projects(&ws.slug, None, None)
        .await
        .expect("root list projects");

    assert!(
        page.items.iter().any(|p| p.slug == "private-project"),
        "root/system admin users must see private workspace projects without membership"
    );
}

#[tokio::test]
async fn get_project_returns_project_data() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _) = login_user_with_workspace(&server, &db, "proj-owner3").await;

    client
        .create_project(&ws.slug, project_req("Get Project", "get-proj"))
        .await
        .expect("create project");

    let project = client
        .get_project(&ws.slug, "get-proj")
        .await
        .expect("get project");

    assert_eq!(project.name, "Get Project");
    assert_eq!(project.slug, "get-proj");
}

#[tokio::test]
async fn update_project_changes_name() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _) = login_user_with_workspace(&server, &db, "proj-owner4").await;

    client
        .create_project(&ws.slug, project_req("Original Name", "upd-proj"))
        .await
        .expect("create project");

    let updated = client
        .update_project(
            &ws.slug,
            "upd-proj",
            UpdateProjectRequest {
                name: Some("Updated Name".to_string()),
                visibility: None,
                visibility_role: None,
                task_prefix: None,
            },
        )
        .await
        .expect("update project");

    assert_eq!(updated.name, "Updated Name");
}

#[tokio::test]
async fn delete_project_soft_deletes() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _) = login_user_with_workspace(&server, &db, "proj-owner5").await;

    client
        .create_project(&ws.slug, project_req("To Delete", "del-proj"))
        .await
        .expect("create project");

    client
        .delete_project(&ws.slug, "del-proj")
        .await
        .expect("delete project");

    // After soft delete, the project should not appear in the list.
    let page = client
        .list_projects(&ws.slug, None, None)
        .await
        .expect("list after delete");

    assert!(
        !page.items.iter().any(|p| p.slug == "del-proj"),
        "deleted project should not appear in list"
    );
}

#[tokio::test]
async fn get_workspace_returns_workspace_info() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _) = login_user_with_workspace(&server, &db, "ws-owner").await;

    let ws_dto = client.get_workspace(&ws.slug).await.expect("get workspace");

    assert_eq!(ws_dto.id, ws.id.0);
    assert_eq!(ws_dto.slug, ws.slug);
}

#[tokio::test]
async fn non_member_cannot_create_project() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_, ws, _) = login_user_with_workspace(&server, &db, "proj-owner6").await;
    let (stranger, _, _) = login_user_with_workspace(&server, &db, "stranger-proj").await;

    let err = stranger
        .create_project(&ws.slug, project_req("Intruder Project", "intruder"))
        .await;

    assert!(err.is_err(), "non-member should not create project");
}

// ---------------------------------------------------------------------------
// B-PREFIX: task_prefix update
// ---------------------------------------------------------------------------

#[tokio::test]
async fn update_project_task_prefix_persists() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "prefix-upd-1").await;

    client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "PrefixProj".into(),
                slug: "prefix-proj".into(),
                task_prefix: "OLD".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let updated = client
        .update_project(
            &ws.slug,
            "prefix-proj",
            UpdateProjectRequest {
                name: None,
                visibility: None,
                visibility_role: None,
                task_prefix: Some("NEW2".into()),
            },
        )
        .await
        .expect("update project prefix");

    assert_eq!(updated.task_prefix, "NEW2");

    let fetched = client
        .get_project(&ws.slug, "prefix-proj")
        .await
        .expect("get project");

    assert_eq!(fetched.task_prefix, "NEW2", "prefix must persist");

    db.teardown().await;
}

#[tokio::test]
async fn update_project_task_prefix_new_tasks_use_new_prefix() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "prefix-tasks-1").await;

    let project = client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "PrefixTasks".into(),
                slug: "prefix-tasks-proj".into(),
                task_prefix: "OL2".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                folder_id: None,
                name: "B".into(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".into(),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");

    let task_before = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Before rename".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task before prefix change");

    client
        .update_project(
            &ws.slug,
            "prefix-tasks-proj",
            UpdateProjectRequest {
                name: None,
                visibility: None,
                visibility_role: None,
                task_prefix: Some("NW3".into()),
            },
        )
        .await
        .expect("update prefix");

    let task_after = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "After rename".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task after prefix change");

    assert!(
        task_before.readable_id.starts_with("OL2-"),
        "task created before prefix change must keep old prefix: {}",
        task_before.readable_id
    );
    assert!(
        task_after.readable_id.starts_with("NW3-"),
        "task created after prefix change must use new prefix: {}",
        task_after.readable_id
    );

    db.teardown().await;
}

#[tokio::test]
async fn update_project_task_prefix_invalid_format_returns_422() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "prefix-422-1").await;

    client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Proj422".into(),
                slug: "proj-422".into(),
                task_prefix: "OK1".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    for bad in &["a", "AB-CD", "1ABC", "toolongprefix", ""] {
        let result = client
            .update_project(
                &ws.slug,
                "proj-422",
                UpdateProjectRequest {
                    name: None,
                    visibility: None,
                    visibility_role: None,
                    task_prefix: Some(bad.to_string()),
                },
            )
            .await;

        assert!(
            matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
            "invalid prefix '{bad}' must return 422, got {result:?}"
        );
    }

    db.teardown().await;
}

#[tokio::test]
async fn update_project_task_prefix_duplicate_in_workspace_returns_409() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "prefix-409-1").await;

    client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "ProjA".into(),
                slug: "proj-a".into(),
                task_prefix: "AAA".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project A");

    client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "ProjB".into(),
                slug: "proj-b".into(),
                task_prefix: "BBB".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project B");

    let result = client
        .update_project(
            &ws.slug,
            "proj-b",
            UpdateProjectRequest {
                name: None,
                visibility: None,
                visibility_role: None,
                task_prefix: Some("AAA".into()),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "duplicate prefix in same workspace must return 409, got {result:?}"
    );

    db.teardown().await;
}
