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
use sea_orm::{ConnectionTrait, Statement};

fn project_req(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: None,
        visibility_role: None,
    }
}

/// End-to-end task-attachment lifecycle on the default disk backend: upload a small
/// file to a task, list it, download it (asserting the bytes and content-type round
/// trip), then soft-delete it and confirm it disappears from the list.
#[tokio::test]
async fn task_attachment_upload_list_download_delete_roundtrip() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "task-attach-1").await;

    client
        .create_project(&ws.slug, project_req("attach-proj", "AT"))
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "attach-proj",
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
                title: "Task with attachment".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let payload = b"hello attachment".to_vec();

    let uploaded = client
        .upload_task_attachment(
            &ws.slug,
            &task.readable_id,
            "notes.txt",
            "text/plain",
            payload.clone(),
        )
        .await
        .expect("upload attachment");

    assert_eq!(uploaded.file_name, "notes.txt");
    assert_eq!(uploaded.content_type, "text/plain");
    assert_eq!(uploaded.size_bytes, payload.len() as i64);

    let intent_count: i64 = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT COUNT(*) AS count FROM attachment_write_intents",
        ))
        .await
        .expect("query write intents")
        .expect("write intent count row")
        .try_get("", "count")
        .expect("read write intent count");
    assert_eq!(
        intent_count, 0,
        "multipart upload must atomically finalize its attachment row and remove the intent"
    );

    let listed = client
        .list_task_attachments(&ws.slug, &task.readable_id)
        .await
        .expect("list attachments");

    assert_eq!(listed.len(), 1, "task must have exactly one attachment");
    assert_eq!(listed[0].id, uploaded.id);

    let (bytes, content_type) = client
        .download_task_attachment(&ws.slug, &task.readable_id, uploaded.id)
        .await
        .expect("download attachment");

    assert_eq!(bytes, payload, "downloaded bytes must round-trip");
    assert_eq!(
        content_type.as_deref(),
        Some("text/plain"),
        "content-type must round-trip"
    );

    client
        .delete_task_attachment(&ws.slug, &task.readable_id, uploaded.id)
        .await
        .expect("delete attachment");

    let after_delete = client
        .list_task_attachments(&ws.slug, &task.readable_id)
        .await
        .expect("list attachments after delete");

    assert!(
        after_delete.is_empty(),
        "soft-deleted attachment must not appear in the list"
    );

    db.teardown().await;
}
