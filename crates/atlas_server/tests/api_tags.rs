#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    boards_tasks::{CreateBoardRequest, CreateColumnRequest, CreateTaskRequest},
    tags::{CreateTagRequest, UpdateTagRequest},
};
use atlas_client::ClientError;

// ---------------------------------------------------------------------------
// Create + list happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_tag_returns_201_and_appears_in_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-crud-1").await;

    let tag = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "Epic".to_string(),
            },
        )
        .await
        .expect("create tag");

    assert_eq!(tag.name, "Epic");
    assert_eq!(tag.workspace_id, ws.id.0);

    let listed = client.list_tags(&ws.slug).await.expect("list tags");

    assert_eq!(listed.len(), 1, "the created tag must appear in the list");
    assert_eq!(listed[0].id, tag.id);
    assert_eq!(listed[0].name, "Epic");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Idempotency by case-insensitive name
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_tag_is_idempotent_by_case_insensitive_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-idem-1").await;

    let first = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "Epic".to_string(),
            },
        )
        .await
        .expect("create tag");

    let same = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "Epic".to_string(),
            },
        )
        .await
        .expect("create same tag");

    let different_case = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "epic".to_string(),
            },
        )
        .await
        .expect("create different-case tag");

    assert_eq!(same.id, first.id, "same name must return the same tag");
    assert_eq!(
        different_case.id, first.id,
        "case-insensitive name must return the same tag"
    );

    let listed = client.list_tags(&ws.slug).await.expect("list tags");

    assert_eq!(listed.len(), 1, "idempotent creates must not duplicate");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Listing order
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_tags_is_sorted_by_name_ascending() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-sort-1").await;

    for name in ["Gamma", "alpha", "Beta"] {
        client
            .create_tag(
                &ws.slug,
                CreateTagRequest {
                    name: name.to_string(),
                },
            )
            .await
            .expect("create tag");
    }

    let listed = client.list_tags(&ws.slug).await.expect("list tags");

    let names: Vec<String> = listed.into_iter().map(|t| t.name).collect();
    assert_eq!(names, vec!["alpha", "Beta", "Gamma"]);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Cross-tenant isolation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tags_are_isolated_per_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "tag-tenant-a").await;
    let (client_b, ws_b, _) =
        support::login_user_with_workspace(&server, &db, "tag-tenant-b").await;

    client_a
        .create_tag(
            &ws_a.slug,
            CreateTagRequest {
                name: "OnlyInA".to_string(),
            },
        )
        .await
        .expect("create tag in A");

    let listed_b = client_b
        .list_tags(&ws_b.slug)
        .await
        .expect("list tags in B");

    assert!(
        listed_b.is_empty(),
        "workspace B must not see workspace A's tags"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_tag_rejects_blank_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-blank-1").await;

    let result = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "   ".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "blank name must be rejected as invalid input, got {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B2 — PATCH /v1/workspaces/{ws}/tags/{tag_id}: rename
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rename_tag_updates_name_and_backfills_task_labels() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tag-rename-backfill").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj".into(),
                task_prefix: "PR".into(),
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
            CreateBoardRequest { name: "B".into() },
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

    let tag = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "backend".into(),
            },
        )
        .await
        .expect("create tag");

    let task1 = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task 1".into(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    labels: vec!["backend".into()],
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task1");

    let task2 = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task 2".into(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    labels: vec!["backend".into(), "infra".into()],
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task2");

    let updated = client
        .update_tag(
            &ws.slug,
            tag.id,
            UpdateTagRequest {
                name: Some("platform".into()),
                color: None,
            },
        )
        .await
        .expect("rename tag");

    assert_eq!(updated.name, "platform");

    let t1 = client
        .get_task(&ws.slug, &task1.readable_id)
        .await
        .expect("get task1");
    let t2 = client
        .get_task(&ws.slug, &task2.readable_id)
        .await
        .expect("get task2");

    assert!(
        t1.labels.contains(&"platform".to_string()),
        "task1 label should have been backfilled: {:?}",
        t1.labels
    );
    assert!(
        !t1.labels.contains(&"backend".to_string()),
        "old label should be gone from task1: {:?}",
        t1.labels
    );
    assert!(
        t2.labels.contains(&"platform".to_string()),
        "task2 should have the new label: {:?}",
        t2.labels
    );
    assert!(
        t2.labels.contains(&"infra".to_string()),
        "task2 unrelated label should be preserved: {:?}",
        t2.labels
    );

    db.teardown().await;
}

#[tokio::test]
async fn rename_tag_dedup_when_new_name_already_present_in_task() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "tag-rename-dedup").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj".into(),
                task_prefix: "PR".into(),
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
            CreateBoardRequest { name: "B".into() },
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

    let old_tag = client
        .create_tag(&ws.slug, CreateTagRequest { name: "v1".into() })
        .await
        .expect("create old tag");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task with both labels".into(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    labels: vec!["v1".into(), "v2".into()],
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    client
        .update_tag(
            &ws.slug,
            old_tag.id,
            UpdateTagRequest {
                name: Some("v2".into()),
                color: None,
            },
        )
        .await
        .expect("rename tag v1 -> v2");

    let t = client
        .get_task(&ws.slug, &task.readable_id)
        .await
        .expect("get task");

    let v2_count = t.labels.iter().filter(|l| l.as_str() == "v2").count();
    assert_eq!(
        v2_count, 1,
        "v2 must appear exactly once after dedup, got: {:?}",
        t.labels
    );

    db.teardown().await;
}

#[tokio::test]
async fn rename_tag_to_existing_name_returns_409() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-rename-409").await;

    client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "alpha".into(),
            },
        )
        .await
        .expect("create alpha");

    let beta = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "beta".into(),
            },
        )
        .await
        .expect("create beta");

    let result = client
        .update_tag(
            &ws.slug,
            beta.id,
            UpdateTagRequest {
                name: Some("alpha".into()),
                color: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "renaming to an existing name must return 409, got {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn recolor_tag_sets_color_without_backfill() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-recolor").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj".into(),
                task_prefix: "PR".into(),
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
            CreateBoardRequest { name: "B".into() },
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

    let tag = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "feature".into(),
            },
        )
        .await
        .expect("create tag");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task".into(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    labels: vec!["feature".into()],
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let updated = client
        .update_tag(
            &ws.slug,
            tag.id,
            UpdateTagRequest {
                name: None,
                color: Some("blue".into()),
            },
        )
        .await
        .expect("recolor tag");

    assert_eq!(updated.color.as_deref(), Some("blue"));
    assert_eq!(
        updated.name, "feature",
        "name must not change on color-only patch"
    );

    let t = client
        .get_task(&ws.slug, &task.readable_id)
        .await
        .expect("get task");

    assert!(
        t.labels.contains(&"feature".to_string()),
        "task label must remain 'feature' after color-only patch: {:?}",
        t.labels
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B2 — DELETE /v1/workspaces/{ws}/tags/{tag_id}: soft delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn soft_delete_tag_removes_it_from_list_but_keeps_task_labels() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-delete-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj".into(),
                task_prefix: "PR".into(),
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
            CreateBoardRequest { name: "B".into() },
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

    let tag = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "deprecated".into(),
            },
        )
        .await
        .expect("create tag");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task".into(),
                description: None,
                properties: Some(atlas_api::dtos::boards_tasks::TaskPropertiesDto {
                    labels: vec!["deprecated".into()],
                    ..Default::default()
                }),
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    client
        .delete_tag(&ws.slug, tag.id)
        .await
        .expect("delete tag");

    let listed = client.list_tags(&ws.slug).await.expect("list tags");

    assert!(
        listed.iter().all(|t| t.id != tag.id),
        "deleted tag must not appear in list"
    );

    let t = client
        .get_task(&ws.slug, &task.readable_id)
        .await
        .expect("get task after delete");

    assert!(
        t.labels.contains(&"deprecated".to_string()),
        "task label string must be preserved after tag soft-delete: {:?}",
        t.labels
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B2 — validation: invalid swatch + cross-workspace 404
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_tag_invalid_swatch_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "tag-inv-swatch").await;

    let tag = client
        .create_tag(
            &ws.slug,
            CreateTagRequest {
                name: "mytag".into(),
            },
        )
        .await
        .expect("create tag");

    let result = client
        .update_tag(
            &ws.slug,
            tag.id,
            UpdateTagRequest {
                name: None,
                color: Some("hotpink".into()),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "invalid swatch must return 422, got {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn patch_tag_from_other_workspace_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws_a, _) = support::login_user_with_workspace(&server, &db, "tag-xws-a").await;
    let (client_b, ws_b, _) = support::login_user_with_workspace(&server, &db, "tag-xws-b").await;

    let tag_a = client_a
        .create_tag(
            &ws_a.slug,
            CreateTagRequest {
                name: "tag-in-a".into(),
            },
        )
        .await
        .expect("create tag in A");

    let result = client_b
        .update_tag(
            &ws_b.slug,
            tag_a.id,
            UpdateTagRequest {
                name: Some("hijack".into()),
                color: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "PATCH tag from other workspace must return 404, got {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn delete_tag_from_other_workspace_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client_a, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "tag-del-xws-a").await;
    let (client_b, ws_b, _) =
        support::login_user_with_workspace(&server, &db, "tag-del-xws-b").await;

    let tag_a = client_a
        .create_tag(
            &ws_a.slug,
            CreateTagRequest {
                name: "tag-in-a".into(),
            },
        )
        .await
        .expect("create tag in A");

    let result = client_b.delete_tag(&ws_b.slug, tag_a.id).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "DELETE tag from other workspace must return 404, got {result:?}"
    );

    db.teardown().await;
}
