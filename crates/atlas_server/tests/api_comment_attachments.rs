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
        CreateBoardRequest, CreateColumnRequest, CreateCommentRequest, CreateTaskRequest,
    },
    documents::CreateDocumentRequest,
};
use reqwest::Response;
use serde_json::Value;

fn project_req(slug: &str, prefix: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: format!("Project {slug}"),
        slug: slug.to_string(),
        task_prefix: prefix.to_string(),
        visibility: None,
        visibility_role: None,
    }
}

async fn upload(
    client: &atlas_client::AtlasClient,
    url: String,
    file_name: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Response {
    let boundary = "atlas-comment-attachment-test-boundary";
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(&bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    client
        .http_client()
        .post(url)
        .bearer_auth(client.token().expect("authenticated token"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .expect("send upload")
}

async fn upload_raw(
    client: &atlas_client::AtlasClient,
    url: String,
    file_name: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Response {
    client
        .http_client()
        .post(url)
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-file-name", file_name)
        .header("content-type", content_type)
        .body(bytes)
        .send()
        .await
        .expect("send raw upload")
}

async fn get(client: &atlas_client::AtlasClient, url: String) -> Response {
    client
        .http_client()
        .get(url)
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("send get")
}

async fn delete(client: &atlas_client::AtlasClient, url: String) -> Response {
    client
        .http_client()
        .delete(url)
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("send delete")
}

async fn request_without_credentials(method: reqwest::Method, url: String) -> Response {
    reqwest::Client::new()
        .request(method, url)
        .header("content-type", "text/plain")
        .body("untrusted bytes")
        .send()
        .await
        .expect("send unauthenticated request")
}

async fn assert_unauthorized_attachment_operations(
    attachment_url: &str,
    attachment_id: uuid::Uuid,
    secret: &str,
) {
    let operations = [
        (reqwest::Method::POST, attachment_url.to_string()),
        (reqwest::Method::GET, attachment_url.to_string()),
        (
            reqwest::Method::GET,
            format!("{attachment_url}/{attachment_id}/content"),
        ),
        (
            reqwest::Method::DELETE,
            format!("{attachment_url}/{attachment_id}"),
        ),
    ];

    for (method, url) in operations {
        let response = request_without_credentials(method, url).await;
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

        let body = response.text().await.expect("read unauthorized response");
        assert!(
            !body.contains(secret),
            "unauthorized responses must not disclose attachment metadata"
        );
    }
}

#[tokio::test]
async fn task_comment_attachment_routes_round_trip_raw_bytes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "task-comment-attachment").await;

    client
        .create_project(&ws.slug, project_req("comment-attachment", "CA"))
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            "comment-attachment",
            CreateBoardRequest {
                name: "Board".into(),
            },
        )
        .await
        .expect("create board");
    let column = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".into(),
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
                title: "Comment attachment task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");
    let comment = client
        .add_comment(
            &ws.slug,
            &task.readable_id,
            CreateCommentRequest {
                body: "Task comment".into(),
            },
        )
        .await
        .expect("create comment");

    let attachment_url = format!(
        "{}/api/workspaces/{}/tasks/{}/comments/{}/attachments",
        server.base_url(),
        ws.slug,
        task.readable_id,
        comment.id
    );
    let payload = b"task comment bytes".to_vec();
    let upload_response = upload(
        &client,
        attachment_url.clone(),
        "task.txt",
        "text/plain",
        payload.clone(),
    )
    .await;
    assert_eq!(upload_response.status(), reqwest::StatusCode::CREATED);
    let attachment: Value = upload_response.json().await.expect("decode attachment");
    assert_eq!(attachment["comment_id"], comment.id.to_string());
    assert_eq!(attachment["file_name"], "task.txt");
    assert_eq!(attachment["size_bytes"], payload.len() as i64);
    let attachment_id = attachment["id"]
        .as_str()
        .expect("attachment id")
        .parse::<uuid::Uuid>()
        .expect("UUID attachment id");

    assert_unauthorized_attachment_operations(&attachment_url, attachment_id, "task.txt").await;

    let list_response = get(&client, attachment_url.clone()).await;
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    let listed: Vec<Value> = list_response.json().await.expect("decode attachment list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], attachment_id.to_string());

    let content_response = get(&client, format!("{attachment_url}/{attachment_id}/content")).await;
    assert_eq!(content_response.status(), reqwest::StatusCode::OK);
    assert_eq!(content_response.headers()["content-type"], "text/plain");
    assert_eq!(
        content_response.bytes().await.expect("read attachment"),
        payload
    );

    let mismatched_comment_content = get(
        &client,
        format!(
            "{}/api/workspaces/{}/tasks/{}/comments/{}/attachments/{attachment_id}/content",
            server.base_url(),
            ws.slug,
            task.readable_id,
            uuid::Uuid::now_v7()
        ),
    )
    .await;
    assert_eq!(
        mismatched_comment_content.status(),
        reqwest::StatusCode::NOT_FOUND,
        "an attachment must not disclose itself through a different comment owner chain"
    );

    let delete_response = delete(&client, format!("{attachment_url}/{attachment_id}")).await;
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);
    let listed_after_delete = get(&client, attachment_url).await;
    assert_eq!(listed_after_delete.status(), reqwest::StatusCode::OK);
    assert!(
        listed_after_delete
            .json::<Vec<Value>>()
            .await
            .expect("decode empty attachment list")
            .is_empty()
    );

    db.teardown().await;
}

#[tokio::test]
async fn document_comment_attachment_routes_round_trip_raw_bytes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-comment-attachment").await;

    client
        .create_project(&ws.slug, project_req("document-comment-attachment", "DA"))
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-comment-attachment",
            CreateDocumentRequest {
                title: "Document comment attachment".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");
    let comment = client
        .add_document_comment(
            &ws.slug,
            &slug,
            CreateCommentRequest {
                body: "Document comment".into(),
            },
        )
        .await
        .expect("create comment");

    let attachment_url = format!(
        "{}/api/workspaces/{}/documents/{}/comments/{}/attachments",
        server.base_url(),
        ws.slug,
        slug,
        comment.id
    );
    let payload = b"document comment bytes".to_vec();
    let upload_response = upload_raw(
        &client,
        attachment_url.clone(),
        "document.txt",
        "text/plain",
        payload.clone(),
    )
    .await;
    assert_eq!(upload_response.status(), reqwest::StatusCode::CREATED);
    let attachment: Value = upload_response.json().await.expect("decode attachment");
    let attachment_id = attachment["id"]
        .as_str()
        .expect("attachment id")
        .parse::<uuid::Uuid>()
        .expect("UUID attachment id");
    assert_eq!(attachment["comment_id"], comment.id.to_string());

    assert_unauthorized_attachment_operations(&attachment_url, attachment_id, "document.txt").await;

    let mismatched_comment_content = get(
        &client,
        format!(
            "{}/api/workspaces/{}/documents/{}/comments/{}/attachments/{attachment_id}",
            server.base_url(),
            ws.slug,
            slug,
            uuid::Uuid::now_v7()
        ),
    )
    .await;
    assert_eq!(
        mismatched_comment_content.status(),
        reqwest::StatusCode::NOT_FOUND,
        "an attachment must not disclose itself through a different document comment owner chain"
    );

    let content_response = get(&client, format!("{attachment_url}/{attachment_id}")).await;
    assert_eq!(content_response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        content_response.bytes().await.expect("read attachment"),
        payload
    );

    let delete_response = delete(&client, format!("{attachment_url}/{attachment_id}")).await;
    assert_eq!(delete_response.status(), reqwest::StatusCode::NO_CONTENT);
    let list_response = get(&client, attachment_url).await;
    assert_eq!(list_response.status(), reqwest::StatusCode::OK);
    assert!(
        list_response
            .json::<Vec<Value>>()
            .await
            .expect("decode empty attachment list")
            .is_empty()
    );

    db.teardown().await;
}
