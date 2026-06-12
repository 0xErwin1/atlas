#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{CreateProjectRequest, UpdateProjectRequest};
use support::{TestDb, TestServer, login_user_with_workspace};

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
