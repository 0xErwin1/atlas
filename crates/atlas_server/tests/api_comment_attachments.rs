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
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, UserRepo};
use reqwest::Response;
use sea_orm::{ConnectionTrait, Statement, TransactionTrait};
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

async fn upload_draft(
    client: &atlas_client::AtlasClient,
    url: String,
    upload_token: uuid::Uuid,
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
        .header("x-upload-token", upload_token.to_string())
        .body(body)
        .send()
        .await
        .expect("send draft upload")
}

async fn add_member(
    db: &support::TestDb,
    server: &support::TestServer,
    workspace_id: atlas_domain::ids::WorkspaceId,
    username: &str,
) -> atlas_client::AtlasClient {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let password = "TestPassword1!";
    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(
                password::hash(password.to_string())
                    .await
                    .expect("hash password"),
            ),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    support::activate_user_in_db(db, user.id.0).await;

    db.membership_repo()
        .add(
            &WorkspaceCtx::new(workspace_id, Actor::User(user.id)),
            user.id,
            MemberRole::Owner,
        )
        .await
        .expect("add workspace owner");

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        })
        .await
        .expect("login workspace owner");

    client
}

async fn assert_no_task_draft_upload_residue(db: &support::TestDb, draft_id: uuid::Uuid) {
    let row = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT \
                (SELECT count(*)::bigint FROM attachments WHERE draft_id = $1) AS attachments, \
                (SELECT count(*)::bigint FROM comment_attachment_draft_uploads WHERE draft_id = $1) AS uploads, \
                (SELECT count(*)::bigint FROM attachment_write_intents) AS intents",
            [draft_id.into()],
        ))
        .await
        .expect("count rejected task draft upload residue")
        .expect("rejected task draft upload residue row");

    assert_eq!(
        row.try_get::<i64>("", "attachments")
            .expect("attachment count"),
        0,
        "a concealed task draft upload must not create an attachment or blob owner"
    );
    assert_eq!(
        row.try_get::<i64>("", "uploads")
            .expect("upload ledger count"),
        0,
        "a concealed task draft upload must not create an upload ledger row"
    );
    assert_eq!(
        row.try_get::<i64>("", "intents")
            .expect("write intent count"),
        0,
        "a concealed task draft upload must not create a blob cleanup ledger row"
    );
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

async fn upload_draft_raw(
    client: &atlas_client::AtlasClient,
    url: String,
    upload_token: uuid::Uuid,
    file_name: &str,
    content_type: &str,
    bytes: Vec<u8>,
) -> Response {
    client
        .http_client()
        .post(url)
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-upload-token", upload_token.to_string())
        .header("x-file-name", file_name)
        .header("content-type", content_type)
        .body(bytes)
        .send()
        .await
        .expect("send raw draft upload")
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

async fn wait_for_advisory_waiter(db: &support::TestDb, lock_key: i32) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let row = db
                .conn()
                .query_one_raw(Statement::from_string(
                    sea_orm::DatabaseBackend::Postgres,
                    format!(
                        "SELECT EXISTS(\
                         SELECT 1 FROM pg_locks \
                         WHERE locktype = 'advisory' AND objid = {lock_key} AND NOT granted\
                         ) AS waiting"
                    ),
                ))
                .await
                .expect("query advisory waiter")
                .expect("advisory waiter row");

            if row
                .try_get::<bool>("", "waiting")
                .expect("decode advisory waiter")
            {
                return;
            }

            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("finalization must reach the coordinated advisory gate");
}

async fn wait_for_draft_lock_waiter(db: &support::TestDb) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let row = db
                .conn()
                .query_one_raw(Statement::from_string(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT EXISTS(\
                     SELECT 1 FROM pg_locks lock \
                     JOIN pg_stat_activity activity ON activity.pid = lock.pid \
                     WHERE lock.locktype = 'transactionid' \
                       AND NOT lock.granted \
                       AND activity.datname = current_database()\
                     ) AS waiting",
                ))
                .await
                .expect("query draft lock waiter")
                .expect("draft lock waiter row");

            if row
                .try_get::<bool>("", "waiting")
                .expect("decode draft lock waiter")
            {
                return;
            }

            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("draft operation must wait for finalization's lifecycle lock");
}

async fn install_finalization_gate(
    db: &support::TestDb,
    draft_id: &str,
) -> (sea_orm::DatabaseTransaction, i32) {
    let lock_key = (u32::from_be_bytes(
        uuid::Uuid::now_v7().as_bytes()[..4]
            .try_into()
            .expect("four UUID bytes"),
    ) & 0x7fff_ffff) as i32;
    let function_name = format!("pause_comment_draft_finalization_{draft_id}").replace('-', "_");
    let trigger_name =
        format!("pause_comment_draft_finalization_trigger_{draft_id}").replace('-', "_");

    db.conn()
        .execute_unprepared(&format!(
            "CREATE FUNCTION {function_name}() RETURNS trigger LANGUAGE plpgsql AS $$ \
             BEGIN PERFORM pg_advisory_xact_lock({lock_key}); RETURN NEW; END; $$; \
             CREATE TRIGGER {trigger_name} BEFORE UPDATE OF state ON comment_attachment_drafts \
             FOR EACH ROW WHEN (NEW.state = 'finalized') EXECUTE FUNCTION {function_name}()"
        ))
        .await
        .expect("install finalization gate");

    let gate = db.conn().begin().await.expect("begin advisory gate");
    gate.execute_unprepared(&format!("SELECT pg_advisory_xact_lock({lock_key})"))
        .await
        .expect("acquire advisory gate");

    (gate, lock_key)
}

async fn assert_no_losing_upload_residue(db: &support::TestDb, draft_id: &str) {
    let attachment_count = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT COUNT(*) AS count FROM attachments \
                 WHERE draft_id = '{draft_id}' OR comment_id = '{draft_id}'"
            ),
        ))
        .await
        .expect("count published or provisional attachments")
        .expect("attachment count row")
        .try_get::<i64>("", "count")
        .expect("decode attachment count");
    let ledger_count = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT COUNT(*) AS count FROM comment_attachment_draft_uploads \
                 WHERE draft_id = '{draft_id}'"
            ),
        ))
        .await
        .expect("count upload ledger rows")
        .expect("ledger count row")
        .try_get::<i64>("", "count")
        .expect("decode ledger count");

    assert_eq!(
        attachment_count, 0,
        "a finalization-winning upload must not leave a provisional or published attachment"
    );
    assert_eq!(
        ledger_count, 0,
        "a finalization-winning upload must not leave an upload ledger row"
    );
}

async fn comment_graph_counts(db: &support::TestDb, comment_id: &str) -> (i64, i64) {
    let row = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT \
                (SELECT count(*)::bigint FROM comment_links WHERE comment_id = $1) AS links, \
                (SELECT count(*)::bigint FROM comment_link_events WHERE comment_id = $1) AS events",
            [comment_id
                .parse::<uuid::Uuid>()
                .expect("comment UUID")
                .into()],
        ))
        .await
        .expect("count comment graph rows")
        .expect("comment graph count row");

    (
        row.try_get("", "links").expect("link count"),
        row.try_get("", "events").expect("event count"),
    )
}

async fn assert_canonical_operation_loses_to_finalization(
    db: &support::TestDb,
    client: &atlas_client::AtlasClient,
    drafts_url: &str,
    comment_url: &str,
    is_document: bool,
    operation: &str,
) {
    let draft = client
        .http_client()
        .post(drafts_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create draft");
    assert_eq!(draft.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft.json().await.expect("decode draft");
    let draft_id = draft["id"].as_str().expect("draft id").to_owned();
    let upload_url = format!("{drafts_url}/{draft_id}/attachments");
    let upload = if is_document {
        upload_draft_raw(
            client,
            upload_url,
            uuid::Uuid::now_v7(),
            "race.txt",
            "text/plain",
            b"canonical race attachment".to_vec(),
        )
        .await
    } else {
        upload_draft(
            client,
            upload_url,
            uuid::Uuid::now_v7(),
            "race.txt",
            "text/plain",
            b"canonical race attachment".to_vec(),
        )
        .await
    };
    assert_eq!(upload.status(), reqwest::StatusCode::CREATED);
    let attachment: Value = upload.json().await.expect("decode attachment");
    let attachment_id = attachment["id"].as_str().expect("attachment id");
    let attachment_url = format!("{comment_url}/{draft_id}/attachments/{attachment_id}");
    let (gate, lock_key) = install_finalization_gate(db, &draft_id).await;
    let http = client.http_client().clone();
    let token = client.token().expect("authenticated token").to_owned();
    let finalizer_url = comment_url.to_owned();
    let finalization_draft_id = draft_id.clone();
    let finalizer = tokio::spawn(async move {
        http.post(finalizer_url)
            .bearer_auth(&token)
            .json(&serde_json::json!({ "body": "finalized", "draft_id": finalization_draft_id }))
            .send()
            .await
            .expect("send finalization")
    });

    wait_for_advisory_waiter(db, lock_key).await;

    let (method, operation_url) = match operation {
        "list" => (
            reqwest::Method::GET,
            format!("{comment_url}/{draft_id}/attachments"),
        ),
        "download" if is_document => (reqwest::Method::GET, attachment_url),
        "download" => (reqwest::Method::GET, format!("{attachment_url}/content")),
        "delete" => (reqwest::Method::DELETE, attachment_url),
        _ => panic!("unsupported canonical draft operation"),
    };
    let http = reqwest::Client::new();
    let token = client.token().expect("authenticated token").to_owned();
    let operation_request = tokio::spawn(async move {
        http.request(method, operation_url)
            .bearer_auth(token)
            .send()
            .await
            .expect("send canonical operation")
    });

    wait_for_draft_lock_waiter(db).await;
    gate.commit().await.expect("release finalization gate");

    let finalized = finalizer.await.expect("finalizer joined");
    let operation_response = operation_request.await.expect("operation joined");
    assert_eq!(finalized.status(), reqwest::StatusCode::CREATED);
    assert_eq!(
        operation_response.status(),
        reqwest::StatusCode::CONFLICT,
        "{operation} must freeze at 409 after losing the draft lifecycle lock"
    );

    let published = get(client, format!("{comment_url}/{draft_id}/attachments")).await;
    assert_eq!(published.status(), reqwest::StatusCode::OK);
    let published: Vec<Value> = published
        .json()
        .await
        .expect("decode published attachments");
    assert_eq!(
        published.len(),
        1,
        "a losing {operation} must not hide, delete, or mutate the transferred attachment"
    );
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
            CreateCommentRequest::published("Task comment"),
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
    let repeated_delete = delete(&client, format!("{attachment_url}/{attachment_id}")).await;
    assert_eq!(repeated_delete.status(), reqwest::StatusCode::NO_CONTENT);
    assert_attachment_row_removed(&db, attachment_id).await;
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
            CreateCommentRequest::published("Document comment"),
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
    let repeated_delete = delete(&client, format!("{attachment_url}/{attachment_id}")).await;
    assert_eq!(repeated_delete.status(), reqwest::StatusCode::NO_CONTENT);
    assert_attachment_row_removed(&db, attachment_id).await;
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

async fn assert_attachment_row_removed(db: &support::TestDb, attachment_id: uuid::Uuid) {
    let count: i64 = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM attachments WHERE id = $1",
            [attachment_id.into()],
        ))
        .await
        .expect("count attachment rows")
        .expect("attachment count row")
        .try_get("", "count")
        .expect("attachment count");

    assert_eq!(
        count, 0,
        "explicit deletion must permanently remove the row"
    );
}

#[tokio::test]
async fn task_comment_draft_create_returns_reserved_comment_identity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "task-comment-draft").await;

    client
        .create_project(&ws.slug, project_req("comment-draft", "CD"))
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            "comment-draft",
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
                title: "Comment draft task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let create_token = uuid::Uuid::now_v7();
    let response = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts",
            server.base_url(),
            ws.slug,
            task.readable_id
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", create_token.to_string())
        .send()
        .await
        .expect("create comment draft");

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let draft: Value = response.json().await.expect("decode comment draft");
    assert!(draft["id"].as_str().is_some());
    assert!(draft["expires_at"].as_str().is_some());

    let replay = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts",
            server.base_url(),
            ws.slug,
            task.readable_id
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", create_token.to_string())
        .send()
        .await
        .expect("replay comment draft creation");

    assert_eq!(replay.status(), reqwest::StatusCode::OK);
    let replayed: Value = replay.json().await.expect("decode replayed comment draft");
    assert_eq!(replayed["id"], draft["id"]);

    let draft_id = draft["id"].as_str().expect("draft id");
    let attachment_url = format!(
        "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}/attachments",
        server.base_url(),
        ws.slug,
        task.readable_id
    );
    let upload_token = uuid::Uuid::now_v7();
    let first_upload = upload_draft(
        &client,
        attachment_url.clone(),
        upload_token,
        "draft.txt",
        "text/plain",
        b"draft attachment".to_vec(),
    )
    .await;

    assert_eq!(first_upload.status(), reqwest::StatusCode::CREATED);
    let uploaded: Value = first_upload.json().await.expect("decode draft attachment");
    assert_eq!(uploaded["file_name"], "draft.txt");
    let uploaded_url = uploaded["url"].as_str().expect("draft attachment URL");
    assert_eq!(uploaded["markdown"], format!("[draft.txt]({uploaded_url})"));

    let attachment_id = uploaded["id"]
        .as_str()
        .expect("draft attachment id")
        .parse::<uuid::Uuid>()
        .expect("UUID draft attachment id");
    let canonical_url = format!(
        "{}/api/workspaces/{}/tasks/{}/comments/{draft_id}/attachments",
        server.base_url(),
        ws.slug,
        task.readable_id
    );

    let listed = get(&client, canonical_url.clone()).await;
    assert_eq!(listed.status(), reqwest::StatusCode::OK);
    let attachments: Vec<Value> = listed.json().await.expect("decode draft attachments");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["id"], attachment_id.to_string());
    assert_eq!(attachments[0]["url"], uploaded["url"]);
    assert_eq!(attachments[0]["markdown"], uploaded["markdown"]);

    let content = get(&client, format!("{canonical_url}/{attachment_id}/content")).await;
    assert_eq!(content.status(), reqwest::StatusCode::OK);
    assert_eq!(content.headers()["content-type"], "text/plain");
    assert_eq!(
        content
            .bytes()
            .await
            .expect("read draft attachment")
            .as_ref(),
        b"draft attachment"
    );

    let replay = upload_draft(
        &client,
        attachment_url,
        upload_token,
        "draft.txt",
        "text/plain",
        b"draft attachment".to_vec(),
    )
    .await;
    assert_eq!(replay.status(), reqwest::StatusCode::OK);

    let deleted = delete(&client, format!("{canonical_url}/{attachment_id}")).await;
    assert_eq!(deleted.status(), reqwest::StatusCode::NO_CONTENT);

    let deleted_download = get(&client, format!("{canonical_url}/{attachment_id}/content")).await;
    assert_eq!(deleted_download.status(), reqwest::StatusCode::GONE);

    let repeated_delete = delete(&client, format!("{canonical_url}/{attachment_id}")).await;
    assert_eq!(repeated_delete.status(), reqwest::StatusCode::GONE);

    let cancelled = delete(
        &client,
        format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}",
            server.base_url(),
            ws.slug,
            task.readable_id
        ),
    )
    .await;
    assert_eq!(cancelled.status(), reqwest::StatusCode::NO_CONTENT);

    let repeated_cancel = delete(
        &client,
        format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}",
            server.base_url(),
            ws.slug,
            task.readable_id
        ),
    )
    .await;
    assert_eq!(repeated_cancel.status(), reqwest::StatusCode::GONE);

    let task_delete = delete(
        &client,
        format!(
            "{}/api/workspaces/{}/tasks/{}",
            server.base_url(),
            ws.slug,
            task.readable_id,
        ),
    )
    .await;
    assert_eq!(task_delete.status(), reqwest::StatusCode::CONFLICT);

    db.teardown().await;
}

#[tokio::test]
async fn task_draft_upload_conceals_missing_or_mismatched_drafts_without_residue() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "task-draft-upload-concealment").await;

    client
        .create_project(&ws.slug, project_req("task-draft-upload-concealment", "TC"))
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            "task-draft-upload-concealment",
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
                title: "Draft owner task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create draft owner task");
    let wrong_task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: column.id,
                title: "Wrong parent task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create wrong parent task");
    let draft_response = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts",
            server.base_url(),
            ws.slug,
            task.readable_id
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create task draft");
    assert_eq!(draft_response.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft_response.json().await.expect("decode task draft");
    let draft_id = draft["id"]
        .as_str()
        .expect("draft id")
        .parse::<uuid::Uuid>()
        .expect("draft UUID");

    let other_principal = add_member(
        &db,
        &server,
        ws.id,
        "task-draft-upload-concealment-other-principal",
    )
    .await;
    let (other_workspace_client, other_workspace, _) = support::login_user_with_workspace(
        &server,
        &db,
        "task-draft-upload-concealment-other-workspace",
    )
    .await;
    other_workspace_client
        .create_project(
            &other_workspace.slug,
            project_req("task-draft-upload-concealment-other-workspace", "TO"),
        )
        .await
        .expect("create other workspace project");
    let other_board = other_workspace_client
        .create_board(
            &other_workspace.slug,
            "task-draft-upload-concealment-other-workspace",
            CreateBoardRequest {
                name: "Other board".into(),
            },
        )
        .await
        .expect("create other workspace board");
    let other_column = other_workspace_client
        .create_column(
            &other_workspace.slug,
            other_board.id,
            CreateColumnRequest {
                name: "Other todo".into(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create other workspace column");
    let other_workspace_task = other_workspace_client
        .create_task(
            &other_workspace.slug,
            other_board.id,
            CreateTaskRequest {
                column_id: other_column.id,
                title: "Other workspace task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create other workspace task");

    let cases = [
        (
            &client,
            format!(
                "{}/api/workspaces/{}/tasks/{}/comment-drafts/{}/attachments",
                server.base_url(),
                ws.slug,
                task.readable_id,
                uuid::Uuid::now_v7(),
            ),
            "unknown draft",
        ),
        (
            &client,
            format!(
                "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}/attachments",
                server.base_url(),
                ws.slug,
                wrong_task.readable_id,
            ),
            "wrong task parent",
        ),
        (
            &other_workspace_client,
            format!(
                "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}/attachments",
                server.base_url(),
                other_workspace.slug,
                other_workspace_task.readable_id,
            ),
            "wrong workspace",
        ),
        (
            &other_principal,
            format!(
                "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}/attachments",
                server.base_url(),
                ws.slug,
                task.readable_id,
            ),
            "cross-principal access",
        ),
    ];

    for (request_client, url, scenario) in cases {
        let response = upload_draft(
            request_client,
            url,
            uuid::Uuid::now_v7(),
            "concealed.txt",
            "text/plain",
            scenario.as_bytes().to_vec(),
        )
        .await;

        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "{scenario} must remain concealed after parent authorization"
        );
    }

    assert_no_task_draft_upload_residue(&db, draft_id).await;

    db.teardown().await;
}

#[tokio::test]
async fn document_comment_draft_create_returns_reserved_comment_identity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-comment-draft").await;

    client
        .create_project(&ws.slug, project_req("document-comment-draft", "DD"))
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-comment-draft",
            CreateDocumentRequest {
                title: "Comment draft document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");

    let response = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/documents/{}/comment-drafts",
            server.base_url(),
            ws.slug,
            slug
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create comment draft");

    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let draft: Value = response.json().await.expect("decode comment draft");
    assert!(draft["id"].as_str().is_some());
    assert!(draft["expires_at"].as_str().is_some());

    db.teardown().await;
}

#[tokio::test]
async fn document_comment_draft_cancel_is_directly_available_and_terminal_on_repeat() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-comment-draft-cancel").await;

    client
        .create_project(&ws.slug, project_req("document-comment-draft-cancel", "DC"))
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-comment-draft-cancel",
            CreateDocumentRequest {
                title: "Comment draft cancel document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");
    let draft_response = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts",
            server.base_url(),
            ws.slug,
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create document comment draft");
    assert_eq!(draft_response.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft_response.json().await.expect("decode comment draft");
    let draft_id = draft["id"].as_str().expect("draft id");
    let cancel_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comment-drafts/{draft_id}",
        server.base_url(),
        ws.slug,
    );

    let cancelled = delete(&client, cancel_url.clone()).await;
    assert_eq!(cancelled.status(), reqwest::StatusCode::NO_CONTENT);

    let repeated_cancel = delete(&client, cancel_url).await;
    assert_eq!(repeated_cancel.status(), reqwest::StatusCode::GONE);

    db.teardown().await;
}

#[tokio::test]
async fn document_delete_conflicts_while_a_retained_comment_draft_exists() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-retained-draft-delete").await;

    client
        .create_project(
            &ws.slug,
            project_req("document-retained-draft-delete", "DR"),
        )
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-retained-draft-delete",
            CreateDocumentRequest {
                title: "Retained draft document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");
    let create_draft = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts",
            server.base_url(),
            ws.slug,
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create document draft");
    assert_eq!(create_draft.status(), reqwest::StatusCode::CREATED);

    let deleted = delete(
        &client,
        format!(
            "{}/api/workspaces/{}/documents/{slug}",
            server.base_url(),
            ws.slug,
        ),
    )
    .await;
    assert_eq!(deleted.status(), reqwest::StatusCode::CONFLICT);

    db.teardown().await;
}

#[tokio::test]
async fn task_comment_finalization_transfers_draft_attachments_and_replays() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "task-comment-finalization").await;

    client
        .create_project(&ws.slug, project_req("comment-finalization", "CF"))
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            "comment-finalization",
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
                title: "Finalization task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let link_target = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: column.id,
                title: "Finalization link target".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create link target");

    let draft = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts",
            server.base_url(),
            ws.slug,
            task.readable_id
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create comment draft");
    assert_eq!(draft.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft.json().await.expect("decode draft");
    let draft_id = draft["id"].as_str().expect("draft id");
    let draft_upload = upload_draft(
        &client,
        format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}/attachments",
            server.base_url(),
            ws.slug,
            task.readable_id
        ),
        uuid::Uuid::now_v7(),
        "final] image.png",
        "image/png",
        b"final attachment".to_vec(),
    )
    .await;
    assert_eq!(draft_upload.status(), reqwest::StatusCode::CREATED);
    let uploaded: Value = draft_upload.json().await.expect("decode upload");
    assert_eq!(
        uploaded["markdown"],
        format!(
            "![final image]({})",
            uploaded["url"].as_str().expect("uploaded URL")
        )
    );

    let comment_url = format!(
        "{}/api/workspaces/{}/tasks/{}/comments",
        server.base_url(),
        ws.slug,
        task.readable_id
    );
    let failed_finalization = client
        .http_client()
        .post(&comment_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&serde_json::json!({ "body": "   ", "draft_id": draft_id }))
        .send()
        .await
        .expect("reject invalid finalization");
    assert_eq!(
        failed_finalization.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );

    let draft_attachments = get(&client, format!("{comment_url}/{draft_id}/attachments")).await;
    assert_eq!(draft_attachments.status(), reqwest::StatusCode::OK);
    let draft_attachments: Vec<Value> = draft_attachments
        .json()
        .await
        .expect("decode attachments after failed finalization");
    assert_eq!(draft_attachments.len(), 1);
    assert_eq!(draft_attachments[0]["id"], uploaded["id"]);
    assert_eq!(draft_attachments[0]["markdown"], uploaded["markdown"]);

    let request = serde_json::json!({
        "body": format!("finalized [[{}|Task]]", link_target.id),
        "draft_id": draft_id,
    });
    let finalized = client
        .http_client()
        .post(&comment_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&request)
        .send()
        .await
        .expect("finalize comment");
    assert_eq!(finalized.status(), reqwest::StatusCode::CREATED);
    let comment: Value = finalized.json().await.expect("decode finalized comment");
    assert_eq!(comment["id"], draft_id);

    let attachment_id = uploaded["id"].as_str().expect("attachment id");
    let attachments_url = format!("{comment_url}/{draft_id}/attachments");
    let attachments = get(&client, attachments_url.clone()).await;
    let attachments_status = attachments.status();
    let attachments_body = attachments.text().await.expect("read attachments");
    assert_eq!(
        attachments_status,
        reqwest::StatusCode::OK,
        "{attachments_body}"
    );
    let attachments: Vec<Value> =
        serde_json::from_str(&attachments_body).expect("decode attachments");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["id"], attachment_id);
    assert_eq!(attachments[0]["url"], uploaded["url"]);
    assert_eq!(attachments[0]["markdown"], uploaded["markdown"]);
    assert_eq!(comment_graph_counts(&db, draft_id).await, (1, 1));

    let published_upload = upload(
        &client,
        attachments_url.clone(),
        "published.txt",
        "text/plain",
        b"published attachment".to_vec(),
    )
    .await;
    assert_eq!(published_upload.status(), reqwest::StatusCode::CREATED);

    let content = get(
        &client,
        format!("{attachments_url}/{attachment_id}/content"),
    )
    .await;
    let content_status = content.status();
    let content_headers = content.headers().clone();
    let content_bytes = content.bytes().await.expect("read attachment response");
    assert_eq!(
        content_status,
        reqwest::StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&content_bytes)
    );
    assert_eq!(content_headers["content-type"], "image/png");
    assert_eq!(content_headers["x-content-type-options"], "nosniff");
    assert_eq!(
        content_headers["content-disposition"],
        "attachment; filename=\"final] image.png\"; filename*=UTF-8''final%5D%20image.png"
    );
    assert_eq!(content_bytes.as_ref(), b"final attachment");

    let deleted = delete(&client, format!("{attachments_url}/{attachment_id}")).await;
    assert_eq!(deleted.status(), reqwest::StatusCode::NO_CONTENT);

    let deleted_download = get(
        &client,
        format!("{attachments_url}/{attachment_id}/content"),
    )
    .await;
    assert_eq!(deleted_download.status(), reqwest::StatusCode::GONE);

    let repeated_delete = delete(&client, format!("{attachments_url}/{attachment_id}")).await;
    assert_eq!(repeated_delete.status(), reqwest::StatusCode::GONE);

    let finalized_upload = upload_draft(
        &client,
        format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}/attachments",
            server.base_url(),
            ws.slug,
            task.readable_id
        ),
        uuid::Uuid::now_v7(),
        "race.txt",
        "text/plain",
        b"must not resurrect".to_vec(),
    )
    .await;
    assert_eq!(finalized_upload.status(), reqwest::StatusCode::CONFLICT);

    let finalized_cancel = delete(
        &client,
        format!(
            "{}/api/workspaces/{}/tasks/{}/comment-drafts/{draft_id}",
            server.base_url(),
            ws.slug,
            task.readable_id
        ),
    )
    .await;
    assert_eq!(finalized_cancel.status(), reqwest::StatusCode::CONFLICT);

    let replay = client
        .http_client()
        .post(&comment_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&request)
        .send()
        .await
        .expect("replay finalization");
    assert_eq!(replay.status(), reqwest::StatusCode::OK);
    assert_eq!(
        replay.json::<Value>().await.expect("decode replay")["id"],
        draft_id
    );
    assert_eq!(comment_graph_counts(&db, draft_id).await, (1, 1));

    let conflict = client
        .http_client()
        .post(comment_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&serde_json::json!({ "body": "changed", "draft_id": draft_id }))
        .send()
        .await
        .expect("conflicting finalization");
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);

    db.teardown().await;
}

#[tokio::test]
async fn document_comment_draft_upload_replays_raw_bytes_and_rejects_changed_reuse() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-comment-draft-upload").await;

    client
        .create_project(&ws.slug, project_req("document-comment-draft-upload", "DU"))
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-comment-draft-upload",
            CreateDocumentRequest {
                title: "Comment draft upload document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");
    let draft_response = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts",
            server.base_url(),
            ws.slug,
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create comment draft");
    assert_eq!(draft_response.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft_response.json().await.expect("decode comment draft");
    let draft_id = draft["id"].as_str().expect("draft id");
    let upload_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comment-drafts/{draft_id}/attachments",
        server.base_url(),
        ws.slug,
    );
    let upload_token = uuid::Uuid::now_v7();
    let payload = b"document draft bytes".to_vec();

    let unknown_draft = upload_draft_raw(
        &client,
        format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts/{}/attachments",
            server.base_url(),
            ws.slug,
            uuid::Uuid::now_v7(),
        ),
        uuid::Uuid::now_v7(),
        "unknown.txt",
        "text/plain",
        b"unknown draft".to_vec(),
    )
    .await;
    assert_eq!(unknown_draft.status(), reqwest::StatusCode::NOT_FOUND);

    let first = upload_draft_raw(
        &client,
        upload_url.clone(),
        upload_token,
        "draft.txt",
        "text/plain",
        payload.clone(),
    )
    .await;
    assert_eq!(first.status(), reqwest::StatusCode::CREATED);
    let attachment: Value = first.json().await.expect("decode attachment");
    assert_eq!(attachment["comment_id"], draft_id);
    assert_eq!(attachment["file_name"], "draft.txt");
    assert_eq!(attachment["content_type"], "text/plain");
    assert_eq!(attachment["size_bytes"], payload.len() as i64);
    let attachment_id = attachment["id"].as_str().expect("attachment id");
    assert_eq!(
        attachment["url"],
        format!(
            "/api/workspaces/{}/documents/{slug}/comments/{draft_id}/attachments/{attachment_id}",
            ws.slug,
        )
    );
    assert_eq!(
        attachment["markdown"],
        format!(
            "[draft.txt]({})",
            attachment["url"].as_str().expect("attachment url")
        )
    );

    let canonical_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comments/{draft_id}/attachments",
        server.base_url(),
        ws.slug,
    );
    let listed = get(&client, canonical_url.clone()).await;
    assert_eq!(listed.status(), reqwest::StatusCode::OK);
    let attachments: Vec<Value> = listed.json().await.expect("decode draft attachments");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["id"], attachment_id);

    let content = get(&client, format!("{canonical_url}/{attachment_id}")).await;
    assert_eq!(content.status(), reqwest::StatusCode::OK);
    assert_eq!(content.headers()["content-type"], "text/plain");
    assert_eq!(
        content
            .bytes()
            .await
            .expect("read draft attachment")
            .as_ref(),
        b"document draft bytes"
    );

    let replay = upload_draft_raw(
        &client,
        upload_url.clone(),
        upload_token,
        "draft.txt",
        "text/plain",
        payload,
    )
    .await;
    assert_eq!(replay.status(), reqwest::StatusCode::OK);
    let replayed: Value = replay.json().await.expect("decode replayed attachment");
    assert_eq!(replayed["id"], attachment["id"]);

    let conflict = upload_draft_raw(
        &client,
        upload_url.clone(),
        upload_token,
        "changed.txt",
        "text/plain",
        b"changed bytes".to_vec(),
    )
    .await;
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);

    let deleted = delete(&client, format!("{canonical_url}/{attachment_id}")).await;
    assert_eq!(deleted.status(), reqwest::StatusCode::NO_CONTENT);

    let deleted_download = get(&client, format!("{canonical_url}/{attachment_id}")).await;
    assert_eq!(deleted_download.status(), reqwest::StatusCode::GONE);

    let repeated_delete = delete(&client, format!("{canonical_url}/{attachment_id}")).await;
    assert_eq!(repeated_delete.status(), reqwest::StatusCode::GONE);

    let invalid_content_type = client
        .http_client()
        .post(upload_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-upload-token", uuid::Uuid::now_v7().to_string())
        .header("x-file-name", "invalid.txt")
        .header("content-type", "text/plain; charset=utf-8")
        .body("invalid content type")
        .send()
        .await
        .expect("send invalid content type");
    assert_eq!(
        invalid_content_type.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );

    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE comment_attachment_drafts SET state = 'cancelled', terminal_at = NOW() WHERE id = $1",
            [draft_id
                .parse::<uuid::Uuid>()
                .expect("draft UUID")
                .into()],
        ))
        .await
        .expect("mark draft cancelled");
    let terminal_draft = upload_draft_raw(
        &client,
        format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts/{draft_id}/attachments",
            server.base_url(),
            ws.slug,
        ),
        uuid::Uuid::now_v7(),
        "terminal.txt",
        "text/plain",
        b"terminal draft".to_vec(),
    )
    .await;
    assert_eq!(terminal_draft.status(), reqwest::StatusCode::GONE);

    db.teardown().await;
}

#[tokio::test]
async fn document_comment_finalization_transfers_draft_attachments_and_replays() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-comment-finalization").await;

    client
        .create_project(&ws.slug, project_req("document-finalization", "DF"))
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-finalization",
            CreateDocumentRequest {
                title: "Finalization document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");

    let draft = client
        .http_client()
        .post(format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts",
            server.base_url(),
            ws.slug,
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create comment draft");
    assert_eq!(draft.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft.json().await.expect("decode draft");
    let draft_id = draft["id"].as_str().expect("draft id");
    let upload = upload_draft_raw(
        &client,
        format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts/{draft_id}/attachments",
            server.base_url(),
            ws.slug,
        ),
        uuid::Uuid::now_v7(),
        "final] image.png",
        "image/png",
        b"final attachment".to_vec(),
    )
    .await;
    assert_eq!(upload.status(), reqwest::StatusCode::CREATED);
    let uploaded: Value = upload.json().await.expect("decode upload");
    assert_eq!(
        uploaded["markdown"],
        format!(
            "![final image]({})",
            uploaded["url"].as_str().expect("uploaded URL")
        )
    );

    let comment_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comments",
        server.base_url(),
        ws.slug,
    );
    let failed_finalization = client
        .http_client()
        .post(&comment_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&serde_json::json!({ "body": "   ", "draft_id": draft_id }))
        .send()
        .await
        .expect("reject invalid finalization");
    assert_eq!(
        failed_finalization.status(),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY
    );

    let draft_attachments = get(&client, format!("{comment_url}/{draft_id}/attachments")).await;
    assert_eq!(draft_attachments.status(), reqwest::StatusCode::OK);
    let draft_attachments: Vec<Value> = draft_attachments
        .json()
        .await
        .expect("decode attachments after failed finalization");
    assert_eq!(draft_attachments.len(), 1);
    assert_eq!(draft_attachments[0]["id"], uploaded["id"]);
    assert_eq!(draft_attachments[0]["markdown"], uploaded["markdown"]);

    let request = serde_json::json!({
        "body": format!("finalized [[{}|Document]]", document.id),
        "draft_id": draft_id,
    });
    let finalized = client
        .http_client()
        .post(&comment_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&request)
        .send()
        .await
        .expect("finalize comment");
    assert_eq!(finalized.status(), reqwest::StatusCode::CREATED);
    let comment: Value = finalized.json().await.expect("decode finalized comment");
    assert_eq!(comment["id"], draft_id);

    let attachment_id = uploaded["id"].as_str().expect("attachment id");
    let attachments_url = format!("{comment_url}/{draft_id}/attachments");
    let attachments = get(&client, attachments_url.clone()).await;
    assert_eq!(attachments.status(), reqwest::StatusCode::OK);
    let attachments: Vec<Value> = attachments.json().await.expect("decode attachments");
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["id"], attachment_id);
    assert_eq!(attachments[0]["url"], uploaded["url"]);
    assert_eq!(attachments[0]["markdown"], uploaded["markdown"]);
    assert_eq!(comment_graph_counts(&db, draft_id).await, (1, 1));

    let content = get(&client, format!("{attachments_url}/{attachment_id}")).await;
    assert_eq!(content.status(), reqwest::StatusCode::OK);
    assert_eq!(content.headers()["content-type"], "image/png");
    assert_eq!(content.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        content.headers()["content-disposition"],
        "attachment; filename=\"final] image.png\"; filename*=UTF-8''final%5D%20image.png"
    );
    assert_eq!(
        content.bytes().await.expect("read attachment").as_ref(),
        b"final attachment"
    );

    let deleted = delete(&client, format!("{attachments_url}/{attachment_id}")).await;
    assert_eq!(deleted.status(), reqwest::StatusCode::NO_CONTENT);

    let deleted_download = get(&client, format!("{attachments_url}/{attachment_id}")).await;
    assert_eq!(deleted_download.status(), reqwest::StatusCode::GONE);

    let repeated_delete = delete(&client, format!("{attachments_url}/{attachment_id}")).await;
    assert_eq!(repeated_delete.status(), reqwest::StatusCode::GONE);

    let finalized_upload = upload_draft_raw(
        &client,
        format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts/{draft_id}/attachments",
            server.base_url(),
            ws.slug,
        ),
        uuid::Uuid::now_v7(),
        "race.txt",
        "text/plain",
        b"must not resurrect".to_vec(),
    )
    .await;
    assert_eq!(finalized_upload.status(), reqwest::StatusCode::CONFLICT);

    let finalized_cancel = delete(
        &client,
        format!(
            "{}/api/workspaces/{}/documents/{slug}/comment-drafts/{draft_id}",
            server.base_url(),
            ws.slug,
        ),
    )
    .await;
    assert_eq!(finalized_cancel.status(), reqwest::StatusCode::CONFLICT);

    let replay = client
        .http_client()
        .post(&comment_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&request)
        .send()
        .await
        .expect("replay finalization");
    assert_eq!(replay.status(), reqwest::StatusCode::OK);
    assert_eq!(
        replay.json::<Value>().await.expect("decode replay")["id"],
        draft_id
    );
    assert_eq!(comment_graph_counts(&db, draft_id).await, (1, 1));

    let conflict = client
        .http_client()
        .post(comment_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&serde_json::json!({ "body": "changed", "draft_id": draft_id }))
        .send()
        .await
        .expect("conflicting finalization");
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);

    db.teardown().await;
}

#[tokio::test]
async fn document_comment_finalization_rejects_terminal_drafts() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-finalization-terminal").await;

    client
        .create_project(
            &ws.slug,
            project_req("document-finalization-terminal", "DT"),
        )
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-finalization-terminal",
            CreateDocumentRequest {
                title: "Terminal finalization document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");
    let drafts_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comment-drafts",
        server.base_url(),
        ws.slug,
    );
    let comment_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comments",
        server.base_url(),
        ws.slug,
    );

    for state in ["cancelled", "expired", "deleted_finalized"] {
        let draft = client
            .http_client()
            .post(&drafts_url)
            .bearer_auth(client.token().expect("authenticated token"))
            .header("x-create-token", uuid::Uuid::now_v7().to_string())
            .send()
            .await
            .expect("create terminal draft");
        assert_eq!(draft.status(), reqwest::StatusCode::CREATED);
        let draft: Value = draft.json().await.expect("decode terminal draft");
        let draft_id = draft["id"].as_str().expect("draft id");

        db.conn()
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE comment_attachment_drafts SET state = $1, terminal_at = NOW() WHERE id = $2",
                [state.into(), draft_id.parse::<uuid::Uuid>().expect("draft UUID").into()],
            ))
            .await
            .expect("mark terminal draft");

        let finalized = client
            .http_client()
            .post(&comment_url)
            .bearer_auth(client.token().expect("authenticated token"))
            .json(&serde_json::json!({ "body": "terminal", "draft_id": draft_id }))
            .send()
            .await
            .expect("finalize terminal draft");
        assert_eq!(
            finalized.status(),
            reqwest::StatusCode::GONE,
            "{state} drafts must retain finalization replay protection"
        );
    }

    db.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_draft_upload_losing_to_finalization_is_conflict_without_residue() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "task-draft-upload-finalize-race").await;

    client
        .create_project(
            &ws.slug,
            project_req("task-draft-upload-finalize-race", "TR"),
        )
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            "task-draft-upload-finalize-race",
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
                title: "Finalization race task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");
    let drafts_url = format!(
        "{}/api/workspaces/{}/tasks/{}/comment-drafts",
        server.base_url(),
        ws.slug,
        task.readable_id
    );
    let comment_url = format!(
        "{}/api/workspaces/{}/tasks/{}/comments",
        server.base_url(),
        ws.slug,
        task.readable_id
    );
    let draft = client
        .http_client()
        .post(&drafts_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create draft");
    assert_eq!(draft.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft.json().await.expect("decode draft");
    let draft_id = draft["id"].as_str().expect("draft id").to_owned();
    let (gate, lock_key) = install_finalization_gate(&db, &draft_id).await;
    let http = client.http_client().clone();
    let token = client.token().expect("authenticated token").to_owned();
    let finalization_request = serde_json::json!({ "body": "finalized", "draft_id": draft_id });
    let finalizer_url = comment_url.clone();
    let finalizer = tokio::spawn(async move {
        http.post(finalizer_url)
            .bearer_auth(&token)
            .json(&finalization_request)
            .send()
            .await
            .expect("send finalization")
    });

    wait_for_advisory_waiter(&db, lock_key).await;

    let http = reqwest::Client::new();
    let token = client.token().expect("authenticated token").to_owned();
    let upload_url = format!("{drafts_url}/{draft_id}/attachments");
    let upload_token = uuid::Uuid::now_v7();
    let uploader = tokio::spawn(async move {
        let boundary = "task-finalization-race-boundary";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"race.txt\"\r\nContent-Type: text/plain\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(b"must not publish");
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        http.post(upload_url)
            .bearer_auth(&token)
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header("x-upload-token", upload_token.to_string())
            .body(body)
            .send()
            .await
            .expect("send draft upload")
    });

    wait_for_draft_lock_waiter(&db).await;
    gate.commit().await.expect("release finalization gate");
    let finalized = finalizer.await.expect("finalizer joined");
    let upload = uploader.await.expect("uploader joined");

    assert_eq!(finalized.status(), reqwest::StatusCode::CREATED);
    assert_eq!(
        upload.status(),
        reqwest::StatusCode::CONFLICT,
        "an upload that resolved as a draft before finalization must be frozen at 409"
    );
    let attachments = get(&client, format!("{comment_url}/{draft_id}/attachments")).await;
    assert_eq!(attachments.status(), reqwest::StatusCode::OK);
    assert!(
        attachments
            .json::<Vec<Value>>()
            .await
            .expect("decode finalized attachments")
            .is_empty(),
        "the losing upload must not be transferred to the finalized comment"
    );
    assert_no_losing_upload_residue(&db, &draft_id).await;

    db.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn document_draft_upload_losing_to_finalization_is_conflict_without_residue() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-draft-upload-finalize-race")
            .await;

    client
        .create_project(
            &ws.slug,
            project_req("document-draft-upload-finalize-race", "DR"),
        )
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-draft-upload-finalize-race",
            CreateDocumentRequest {
                title: "Finalization race document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");
    let drafts_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comment-drafts",
        server.base_url(),
        ws.slug,
    );
    let comment_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comments",
        server.base_url(),
        ws.slug,
    );
    let draft = client
        .http_client()
        .post(&drafts_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create draft");
    assert_eq!(draft.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft.json().await.expect("decode draft");
    let draft_id = draft["id"].as_str().expect("draft id").to_owned();
    let (gate, lock_key) = install_finalization_gate(&db, &draft_id).await;
    let http = client.http_client().clone();
    let token = client.token().expect("authenticated token").to_owned();
    let finalization_request = serde_json::json!({ "body": "finalized", "draft_id": draft_id });
    let finalizer_url = comment_url.clone();
    let finalizer = tokio::spawn(async move {
        http.post(finalizer_url)
            .bearer_auth(&token)
            .json(&finalization_request)
            .send()
            .await
            .expect("send finalization")
    });

    wait_for_advisory_waiter(&db, lock_key).await;

    let http = reqwest::Client::new();
    let token = client.token().expect("authenticated token").to_owned();
    let upload_url = format!("{drafts_url}/{draft_id}/attachments");
    let upload_token = uuid::Uuid::now_v7();
    let uploader = tokio::spawn(async move {
        http.post(upload_url)
            .bearer_auth(&token)
            .header("x-upload-token", upload_token.to_string())
            .header("x-file-name", "race.txt")
            .header("content-type", "text/plain")
            .body("must not publish")
            .send()
            .await
            .expect("send draft upload")
    });

    wait_for_draft_lock_waiter(&db).await;
    gate.commit().await.expect("release finalization gate");
    let finalized = finalizer.await.expect("finalizer joined");
    let upload = uploader.await.expect("uploader joined");

    assert_eq!(finalized.status(), reqwest::StatusCode::CREATED);
    assert_eq!(
        upload.status(),
        reqwest::StatusCode::CONFLICT,
        "an upload that resolved as a draft before finalization must be frozen at 409"
    );
    let attachments = get(&client, format!("{comment_url}/{draft_id}/attachments")).await;
    assert_eq!(attachments.status(), reqwest::StatusCode::OK);
    assert!(
        attachments
            .json::<Vec<Value>>()
            .await
            .expect("decode finalized attachments")
            .is_empty(),
        "the losing upload must not be transferred to the finalized comment"
    );
    assert_no_losing_upload_residue(&db, &draft_id).await;

    db.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_canonical_draft_operations_losing_to_finalization_are_conflicts() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "task-canonical-finalization-races").await;

    client
        .create_project(
            &ws.slug,
            project_req("task-canonical-finalization-races", "TC"),
        )
        .await
        .expect("create project");
    let board = client
        .create_board(
            &ws.slug,
            "task-canonical-finalization-races",
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
                title: "Canonical finalization race task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");
    let drafts_url = format!(
        "{}/api/workspaces/{}/tasks/{}/comment-drafts",
        server.base_url(),
        ws.slug,
        task.readable_id
    );
    let comment_url = format!(
        "{}/api/workspaces/{}/tasks/{}/comments",
        server.base_url(),
        ws.slug,
        task.readable_id
    );

    for operation in ["list", "download", "delete"] {
        assert_canonical_operation_loses_to_finalization(
            &db,
            &client,
            &drafts_url,
            &comment_url,
            false,
            operation,
        )
        .await;
    }

    db.teardown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn document_canonical_draft_operations_losing_to_finalization_are_conflicts() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "document-canonical-finalization-races")
            .await;

    client
        .create_project(
            &ws.slug,
            project_req("document-canonical-finalization-races", "DC"),
        )
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            "document-canonical-finalization-races",
            CreateDocumentRequest {
                title: "Canonical finalization race document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");
    let drafts_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comment-drafts",
        server.base_url(),
        ws.slug,
    );
    let comment_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comments",
        server.base_url(),
        ws.slug,
    );

    for operation in ["list", "download", "delete"] {
        assert_canonical_operation_loses_to_finalization(
            &db,
            &client,
            &drafts_url,
            &comment_url,
            true,
            operation,
        )
        .await;
    }

    db.teardown().await;
}
