#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    ApiKeyScope, CreateProjectRequest, CreateUserApiKeyRequest, InitialGrantRequest,
    boards_tasks::{
        CreateBoardRequest, CreateColumnRequest, CreateTaskRequest, RenameTaskAttachmentRequest,
    },
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
/// file to a task, rename its metadata, list and download it (asserting the renamed
/// metadata, bytes, and content-type round trip), then soft-delete it and confirm it
/// disappears from the list.
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
                folder_id: None,
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

    let token = client.token().expect("authenticated token");
    let attachment_url = format!(
        "{}/api/workspaces/{}/tasks/{}/attachments/{}",
        server.base_url(),
        ws.slug,
        task.readable_id,
        uploaded.id
    );

    let renamed = client
        .rename_task_attachment(
            &ws.slug,
            &task.readable_id,
            uploaded.id,
            RenameTaskAttachmentRequest {
                file_name: "  meeting-notes.txt  ".to_string(),
            },
        )
        .await
        .expect("rename attachment");
    assert_eq!(renamed.id, uploaded.id);
    assert_eq!(renamed.file_name, "meeting-notes.txt");
    assert_eq!(renamed.content_type, uploaded.content_type);
    assert_eq!(renamed.size_bytes, uploaded.size_bytes);

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
    assert_eq!(listed[0].file_name, "meeting-notes.txt");

    let download_response = reqwest::Client::new()
        .get(format!("{attachment_url}/content"))
        .bearer_auth(token)
        .send()
        .await
        .expect("download renamed attachment metadata");
    assert_eq!(download_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        download_response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok()),
        Some("attachment; filename=\"meeting-notes.txt\"; filename*=UTF-8''meeting-notes.txt")
    );

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

    let invalid_response = reqwest::Client::new()
        .patch(&attachment_url)
        .bearer_auth(token)
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "file_name": "   " }))
        .send()
        .await
        .expect("invalid rename request");
    assert_eq!(
        invalid_response.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );

    let overlong_response = reqwest::Client::new()
        .patch(&attachment_url)
        .bearer_auth(token)
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "file_name": format!("{}x", "é".repeat(100)) }))
        .send()
        .await
        .expect("overlong rename request");
    assert_eq!(
        overlong_response.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );
    let problem: serde_json::Value = overlong_response
        .json()
        .await
        .expect("overlong-name RFC9457 response");
    assert_eq!(
        problem.get("detail").and_then(serde_json::Value::as_str),
        Some("file_name must be at most 200 bytes")
    );

    for blocked_name in ["payload.exe", "script.bat", "setup.ps1"] {
        let blocked_response = reqwest::Client::new()
            .patch(&attachment_url)
            .bearer_auth(token)
            .header("x-atlas-csrf", "1")
            .json(&serde_json::json!({ "file_name": format!("  {blocked_name}  ") }))
            .send()
            .await
            .expect("blocked-extension rename request");
        assert_eq!(
            blocked_response.status(),
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            "rename to {blocked_name} must be rejected"
        );
        let problem: serde_json::Value = blocked_response
            .json()
            .await
            .expect("blocked-extension RFC9457 response");
        let expected_hint = format!(
            "File extension '{}' is not allowed",
            blocked_name.rsplit_once('.').expect("extension").1
        );
        assert_eq!(
            problem.get("detail").and_then(serde_json::Value::as_str),
            Some(expected_hint.as_str())
        );
    }

    let second_task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Other task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create other task");
    let mismatch_response = reqwest::Client::new()
        .patch(format!(
            "{}/api/workspaces/{}/tasks/{}/attachments/{}",
            server.base_url(),
            ws.slug,
            second_task.readable_id,
            uploaded.id
        ))
        .bearer_auth(token)
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "file_name": "must-not-change.txt" }))
        .send()
        .await
        .expect("cross-task rename request");
    assert_eq!(mismatch_response.status(), reqwest::StatusCode::NOT_FOUND);

    let missing_response = reqwest::Client::new()
        .patch(format!(
            "{}/api/workspaces/{}/tasks/{}/attachments/{}",
            server.base_url(),
            ws.slug,
            task.readable_id,
            uuid::Uuid::now_v7()
        ))
        .bearer_auth(token)
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "file_name": "must-not-change.txt" }))
        .send()
        .await
        .expect("missing attachment rename request");
    assert_eq!(missing_response.status(), reqwest::StatusCode::NOT_FOUND);

    let (other_client, other_ws, _) =
        support::login_user_with_workspace(&server, &db, "task-attach-other-ws").await;
    other_client
        .create_project(&other_ws.slug, project_req("other-attach-proj", "OA"))
        .await
        .expect("create other workspace project");
    let other_board = other_client
        .create_board(
            &other_ws.slug,
            "other-attach-proj",
            CreateBoardRequest {
                folder_id: None,
                name: "Other board".to_string(),
            },
        )
        .await
        .expect("create other workspace board");
    let other_column = other_client
        .create_column(
            &other_ws.slug,
            other_board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create other workspace column");
    let other_task = other_client
        .create_task(
            &other_ws.slug,
            other_board.id,
            CreateTaskRequest {
                column_id: other_column.id,
                title: "Other workspace task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create other workspace task");
    let other_attachment = other_client
        .upload_task_attachment(
            &other_ws.slug,
            &other_task.readable_id,
            "foreign.txt",
            "text/plain",
            b"foreign".to_vec(),
        )
        .await
        .expect("upload other workspace attachment");

    let cross_workspace_response = reqwest::Client::new()
        .patch(format!(
            "{}/api/workspaces/{}/tasks/{}/attachments/{}",
            server.base_url(),
            ws.slug,
            task.readable_id,
            other_attachment.id
        ))
        .bearer_auth(token)
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "file_name": "must-not-change.txt" }))
        .send()
        .await
        .expect("cross-workspace rename request");
    assert_eq!(
        cross_workspace_response.status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let other_listed = other_client
        .list_task_attachments(&other_ws.slug, &other_task.readable_id)
        .await
        .expect("list other workspace attachments");
    assert_eq!(other_listed[0].file_name, "foreign.txt");

    let read_only_key = client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "task-attachment-read-only".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![ApiKeyScope::TasksRead]),
        })
        .await
        .expect("create read-only task API key");
    let read_only_client = atlas_client::AtlasClient::new(server.base_url().to_string())
        .with_token(read_only_key.secret);
    let capability_result = read_only_client
        .rename_task_attachment(
            &ws.slug,
            &task.readable_id,
            uploaded.id,
            RenameTaskAttachmentRequest {
                file_name: "must-not-change.txt".to_string(),
            },
        )
        .await;
    assert!(
        matches!(capability_result, Err(atlas_client::ClientError::Api(ref problem)) if problem.status == 403),
        "an API key without tasks:update must be denied: {capability_result:?}"
    );

    let unauthenticated_response = reqwest::Client::new()
        .patch(&attachment_url)
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "file_name": "must-not-change.txt" }))
        .send()
        .await
        .expect("unauthenticated rename request");
    assert_eq!(
        unauthenticated_response.status(),
        reqwest::StatusCode::UNAUTHORIZED
    );

    let after_rejections = client
        .list_task_attachments(&ws.slug, &task.readable_id)
        .await
        .expect("list attachments after rejected renames");
    assert_eq!(after_rejections[0].file_name, "meeting-notes.txt");

    client
        .delete_task_attachment(&ws.slug, &task.readable_id, uploaded.id)
        .await
        .expect("delete attachment");

    let deleted_at: Option<chrono::DateTime<chrono::Utc>> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM attachments WHERE id = $1",
            [uploaded.id.into()],
        ))
        .await
        .expect("load deleted attachment")
        .expect("deleted attachment row")
        .try_get("", "deleted_at")
        .expect("attachment tombstone");
    assert!(
        deleted_at.is_some(),
        "delete must retain the attachment row"
    );

    let cleanup_intents: i64 = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT COUNT(*) AS count FROM attachment_write_intents",
        ))
        .await
        .expect("count cleanup intents")
        .expect("cleanup intent count row")
        .try_get("", "count")
        .expect("cleanup intent count");
    assert_eq!(
        cleanup_intents, 0,
        "ordinary delete must not schedule blob cleanup"
    );

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

#[tokio::test]
async fn task_attachment_rename_respects_configured_extension_allowlist() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_upload_allowed_extensions(["txt"]);
    let server = support::TestServer::spawn_with_state(state).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "task-attach-allowlist").await;

    client
        .create_project(&ws.slug, project_req("allowlist-proj", "AL"))
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            "allowlist-proj",
            CreateBoardRequest {
                folder_id: None,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");
    let column = client
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
                column_id: column.id,
                title: "Allowlist rename".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");
    let attachment = client
        .upload_task_attachment(
            &ws.slug,
            &task.readable_id,
            "notes.txt",
            "text/plain",
            b"notes".to_vec(),
        )
        .await
        .expect("upload allowed attachment");

    let response = reqwest::Client::new()
        .patch(format!(
            "{}/api/workspaces/{}/tasks/{}/attachments/{}",
            server.base_url(),
            ws.slug,
            task.readable_id,
            attachment.id
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-atlas-csrf", "1")
        .json(&serde_json::json!({ "file_name": "  notes.pdf  " }))
        .send()
        .await
        .expect("allowlist rename request");

    assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    let problem: serde_json::Value = response.json().await.expect("RFC9457 response");
    assert_eq!(
        problem.get("detail").and_then(serde_json::Value::as_str),
        Some("File extension 'pdf' is not allowed")
    );

    let listed = client
        .list_task_attachments(&ws.slug, &task.readable_id)
        .await
        .expect("list attachment after rejected rename");
    assert_eq!(listed[0].file_name, "notes.txt");

    db.teardown().await;
}
