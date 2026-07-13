#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    ApiKeyScope, CreateProjectRequest, CreateUserApiKeyRequest, InitialGrantRequest,
    UpdateProjectRequest,
    boards_tasks::{
        CreateBoardRequest, CreateColumnRequest, CreateCommentRequest, CreateTaskRequest,
        UpdateCommentRequest,
    },
    documents::CreateDocumentRequest,
};
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, UserRepo};
use sea_orm::ConnectionTrait;
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

/// Creates and logs in a second user with the given membership role in `ws`.
async fn add_member(
    db: &support::TestDb,
    server: &support::TestServer,
    ws_id: atlas_domain::ids::WorkspaceId,
    username: &str,
    role: MemberRole,
) -> (
    atlas_client::AtlasClient,
    atlas_domain::entities::identity::User,
) {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    support::activate_user_in_db(db, user.id.0).await;

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(user.id));
    db.membership_repo()
        .add(&ctx, user.id, role)
        .await
        .expect("add membership");

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    (client, user)
}

/// Creates a project, board, column, and a single task; returns the task's readable id.
async fn seed_task(
    client: &atlas_client::AtlasClient,
    ws_slug: &str,
    slug: &str,
    prefix: &str,
) -> String {
    client
        .create_project(ws_slug, project_req(slug, prefix))
        .await
        .expect("create project");

    let board = client
        .create_board(
            ws_slug,
            slug,
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
                color: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            ws_slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task with comments".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    task.readable_id
}

// ---------------------------------------------------------------------------
// Create + list + delete roundtrip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_list_delete_comment_roundtrip() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-roundtrip").await;

    let readable_id = seed_task(&client, &ws.slug, "comment-proj", "CM").await;

    let created = client
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "First comment".to_string(),
            },
        )
        .await
        .expect("create comment");

    assert_eq!(created.body, "First comment");
    assert_eq!(created.author.r#type, "user");
    assert_eq!(
        created.author.display_name.as_deref(),
        Some("comment-roundtrip")
    );

    let page = client
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments");

    assert_eq!(page.items.len(), 1, "task must have exactly one comment");
    assert_eq!(page.items[0].id, created.id);
    assert!(!page.has_more);

    client
        .delete_comment(&ws.slug, &readable_id, created.id)
        .await
        .expect("delete comment");

    let after_delete = client
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments after delete");

    assert!(
        after_delete.items.is_empty(),
        "soft-deleted comment must not appear in the list"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_backlinks_expose_only_authorized_comment_parent_navigation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-backlink").await;
    let source_readable_id = seed_task(&client, &ws.slug, "comment-backlink-source", "CBS").await;
    let target_readable_id = seed_task(&client, &ws.slug, "comment-backlink-target", "CBT").await;
    let source_task = client
        .get_task(&ws.slug, &source_readable_id)
        .await
        .expect("get source task");
    let target_task = client
        .get_task(&ws.slug, &target_readable_id)
        .await
        .expect("get target task");
    let comment = client
        .add_comment(
            &ws.slug,
            &source_readable_id,
            CreateCommentRequest {
                body: "linked comment".into(),
            },
        )
        .await
        .expect("create source comment");

    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_links (id, workspace_id, comment_id, target_task_id, created_at) VALUES ('{}', '{}', '{}', '{}', now())",
            uuid::Uuid::now_v7(), ws.id.0, comment.id, target_task.id,
        ))
        .await
        .expect("insert derived comment link");

    let backlinks = client
        .list_task_backlinks(&ws.slug, &target_readable_id)
        .await
        .expect("list target backlinks");
    let backlink = backlinks
        .items
        .iter()
        .find(|backlink| backlink.comment_source.is_some())
        .expect("comment backlink");
    let source = backlink.comment_source.as_ref().expect("comment source");
    assert_eq!(source.kind, "comment");
    assert_eq!(source.comment_id, comment.id);
    assert!(matches!(
        &source.parent,
        atlas_api::dtos::documents::CommentBacklinkParentDto::Task { id, readable_id, title }
            if *id == source_task.id && readable_id == &source_readable_id && title == "Task with comments"
    ));

    db.teardown().await;
}

#[tokio::test]
async fn document_backlinks_expose_comment_source_without_attachment_links() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-document-backlink").await;
    let source_readable_id = seed_task(&client, &ws.slug, "comment-document-source", "CDS").await;
    let project = client
        .create_project(&ws.slug, project_req("comment-document-target", "CDT"))
        .await
        .expect("create document project");
    let target = client
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Comment target".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create target document");
    let comment = client
        .add_comment(
            &ws.slug,
            &source_readable_id,
            CreateCommentRequest {
                body: "linked comment".into(),
            },
        )
        .await
        .expect("create source comment");

    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_links (id, workspace_id, comment_id, target_document_id, created_at) VALUES ('{}', '{}', '{}', '{}', now())",
            uuid::Uuid::now_v7(), ws.id.0, comment.id, target.id,
        ))
        .await
        .expect("insert derived comment link");

    let backlinks = client
        .list_backlinks(
            &ws.slug,
            target.slug.as_deref().expect("target slug"),
            None,
            None,
        )
        .await
        .expect("list document backlinks");
    let source = backlinks
        .items
        .iter()
        .find_map(|backlink| backlink.comment_source.as_ref())
        .expect("comment backlink source");
    assert_eq!(source.kind, "comment");
    assert_eq!(source.comment_id, comment.id);
    assert!(matches!(
        &source.parent,
        atlas_api::dtos::documents::CommentBacklinkParentDto::Task { readable_id, .. }
            if readable_id == &source_readable_id
    ));

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Author attribution: api-key actor
// ---------------------------------------------------------------------------

/// Automations (E11) post comments as an api-key actor, not a user — this is the
/// feature's primary use case, so the returned author must report the api-key's
/// type and display name, not fall back to a user attribution.
#[tokio::test]
async fn create_comment_as_api_key_reports_api_key_author() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-api-key-actor").await;

    let readable_id = seed_task(&client, &ws.slug, "comment-api-key-proj", "CK").await;

    let api_key = client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "comment-bot".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![ApiKeyScope::TasksRead, ApiKeyScope::TasksUpdate]),
        })
        .await
        .expect("create api key");

    let mut api_key_client = atlas_client::AtlasClient::new(server.base_url().to_string());
    api_key_client.set_token(api_key.secret.clone());

    let created = api_key_client
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "Automated comment".to_string(),
            },
        )
        .await
        .expect("create comment as api key");

    assert_eq!(created.body, "Automated comment");
    assert_eq!(created.author.r#type, "api_key");
    assert_eq!(created.author.id, api_key.id);
    assert_eq!(created.author.display_name.as_deref(), Some("comment-bot"));

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// List: oldest-first ordering, empty state, cursor pagination
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_comments_empty_state_returns_no_error() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-empty").await;

    let readable_id = seed_task(&client, &ws.slug, "comment-empty-proj", "CE").await;

    let page = client
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments on task with no comments");

    assert!(page.items.is_empty());
    assert!(!page.has_more);
    assert!(page.next_cursor.is_none());

    db.teardown().await;
}

#[tokio::test]
async fn list_comments_oldest_first_with_cursor_walk_has_no_gaps_or_duplicates() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-cursor").await;

    let readable_id = seed_task(&client, &ws.slug, "comment-cursor-proj", "CC").await;

    let mut created_ids = Vec::new();
    for i in 0..5 {
        let comment = client
            .add_comment(
                &ws.slug,
                &readable_id,
                CreateCommentRequest {
                    body: format!("Comment {i}"),
                },
            )
            .await
            .expect("create comment");
        created_ids.push(comment.id);
    }

    let mut collected = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = client
            .list_comments(&ws.slug, &readable_id, cursor.as_deref(), Some(2))
            .await
            .expect("list comments page");

        collected.extend(page.items.iter().map(|c| c.id));

        if !page.has_more {
            assert!(page.next_cursor.is_none());
            break;
        }

        cursor = page.next_cursor;
        assert!(cursor.is_some(), "has_more page must carry a next_cursor");
    }

    assert_eq!(
        collected, created_ids,
        "cursor walk must return every comment exactly once, oldest-first, no gaps or duplicates"
    );

    db.teardown().await;
}

#[tokio::test]
async fn full_feed_is_opt_in_and_default_comment_page_is_unchanged() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-full-feed").await;
    let readable_id = seed_task(&client, &ws.slug, "comment-full-feed-proj", "CF").await;

    client
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "Comment with no links".to_string(),
            },
        )
        .await
        .expect("create comment");

    let default_page = client
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("default list comments");
    assert_eq!(default_page.items.len(), 1);
    assert_eq!(default_page.items[0].body, "Comment with no links");

    let response = reqwest::Client::new()
        .get(format!(
            "{}/api/workspaces/{}/tasks/{}/comments?feed=full",
            server.base_url(),
            ws.slug,
            readable_id
        ))
        .bearer_auth(client.token().expect("session token"))
        .send()
        .await
        .expect("full feed response");

    assert!(response.status().is_success());
    let page: Value = response.json().await.expect("full feed JSON");
    assert_eq!(page["items"][0]["type"], "comment");
    assert_eq!(page["items"][0]["comment"]["body"], "Comment with no links");
    assert_eq!(page["items"][0]["links"], serde_json::json!([]));

    db.teardown().await;
}

#[tokio::test]
async fn full_feeds_redact_deleted_targets_for_human_and_api_key_viewers() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, user) =
        support::login_user_with_workspace(&server, &db, "comment-feed-target-matrix").await;
    let task_id = seed_task(&owner, &ws.slug, "comment-feed-target-proj", "CFM").await;

    let document = owner
        .create_document(
            &ws.slug,
            "comment-feed-target-proj",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Document feed parent".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document feed parent");
    let document_slug = document.slug.clone().expect("document slug");

    let source_task_comment = owner
        .add_comment(
            &ws.slug,
            &task_id,
            CreateCommentRequest {
                body: "task source".into(),
            },
        )
        .await
        .expect("create task source comment");
    let source_document_comment = owner
        .add_document_comment(
            &ws.slug,
            &document_slug,
            CreateCommentRequest {
                body: "document source".into(),
            },
        )
        .await
        .expect("create document source comment");

    let target_document = owner
        .create_document(
            &ws.slug,
            "comment-feed-target-proj",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Linked document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create linked document");
    let target_task = owner
        .create_task(
            &ws.slug,
            owner
                .list_boards(&ws.slug, "comment-feed-target-proj", None, None)
                .await
                .expect("list boards")
                .items[0]
                .id,
            CreateTaskRequest {
                column_id: owner
                    .list_columns(
                        &ws.slug,
                        owner
                            .list_boards(&ws.slug, "comment-feed-target-proj", None, None)
                            .await
                            .expect("list boards")
                            .items[0]
                            .id,
                    )
                    .await
                    .expect("list columns")[0]
                    .id,
                title: "Linked task".into(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create linked task");
    let attachment_comment = owner
        .add_comment(
            &ws.slug,
            &target_task.readable_id,
            CreateCommentRequest {
                body: "attachment owner".into(),
            },
        )
        .await
        .expect("create attachment owner comment");
    let direct_attachment = uuid::Uuid::now_v7();
    let comment_attachment = uuid::Uuid::now_v7();

    for (attachment_id, task_owner, comment_owner, digest) in [
        (
            direct_attachment,
            Some(target_task.id),
            None,
            "a".repeat(64),
        ),
        (
            comment_attachment,
            None,
            Some(attachment_comment.id),
            "b".repeat(64),
        ),
    ] {
        let task_owner = task_owner
            .map(|id| format!("'{id}'"))
            .unwrap_or_else(|| "NULL".into());
        let comment_owner = comment_owner
            .map(|id| format!("'{id}'"))
            .unwrap_or_else(|| "NULL".into());
        db.conn()
            .execute_unprepared(&format!(
                "INSERT INTO attachments (id, workspace_id, task_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) VALUES ('{attachment_id}', '{}', {task_owner}, {comment_owner}, 'target.txt', 'text/plain', 1, '{digest}', '{}', now(), now())",
                ws.id.0, user.id.0,
            ))
            .await
            .expect("insert target attachment");
    }

    for source_comment in [source_task_comment.id, source_document_comment.id] {
        for (column, target) in [
            ("target_document_id", target_document.id),
            ("target_task_id", target_task.id),
            ("target_attachment_id", direct_attachment),
            ("target_attachment_id", comment_attachment),
        ] {
            db.conn()
                .execute_unprepared(&format!(
                    "INSERT INTO comment_links (id, workspace_id, comment_id, {column}, created_at) VALUES ('{}', '{}', '{}', '{target}', now())",
                    uuid::Uuid::now_v7(), ws.id.0, source_comment,
                ))
                .await
                .expect("insert derived target link");
        }
    }

    let api_key = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "feed-reader".into(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".into(),
            }),
            scopes: Some(vec![ApiKeyScope::DocsRead, ApiKeyScope::TasksRead]),
        })
        .await
        .expect("create feed reader key");

    for token in [owner.token().expect("owner token"), api_key.secret.as_str()] {
        for url in [
            format!(
                "{}/api/workspaces/{}/tasks/{task_id}/comments?feed=full",
                server.base_url(),
                ws.slug
            ),
            format!(
                "{}/api/workspaces/{}/documents/{document_slug}/comments?feed=full",
                server.base_url(),
                ws.slug
            ),
        ] {
            let page: Value = reqwest::Client::new()
                .get(url)
                .bearer_auth(token)
                .send()
                .await
                .expect("get full feed")
                .json()
                .await
                .expect("decode full feed");
            let links = page["items"][0]["links"].as_array().expect("feed links");
            assert_eq!(links.len(), 4);
            for link in links {
                assert_eq!(link["target"]["status"], "available");
                assert!(link["target"].get("type").is_some());
                assert!(link["target"].get("id").is_some());
                assert!(link["target"].get("title").is_none());
            }
        }
    }

    db.conn()
        .execute_unprepared(&format!(
            "UPDATE documents SET deleted_at = now() WHERE id = '{}'; UPDATE tasks SET deleted_at = now() WHERE id = '{}'; UPDATE attachments SET deleted_at = now() WHERE id IN ('{direct_attachment}', '{comment_attachment}')",
            target_document.id, target_task.id,
        ))
        .await
        .expect("delete linked targets");

    for url in [
        format!(
            "{}/api/workspaces/{}/tasks/{task_id}/comments?feed=full",
            server.base_url(),
            ws.slug
        ),
        format!(
            "{}/api/workspaces/{}/documents/{document_slug}/comments?feed=full",
            server.base_url(),
            ws.slug
        ),
    ] {
        let page: Value = reqwest::Client::new()
            .get(url)
            .bearer_auth(owner.token().expect("owner token"))
            .send()
            .await
            .expect("get deleted-target feed")
            .json()
            .await
            .expect("decode deleted-target feed");
        for link in page["items"][0]["links"].as_array().expect("feed links") {
            assert_eq!(
                link["target"],
                serde_json::json!({"status":"unavailable","label":"Recurso no disponible"})
            );
        }
    }

    db.teardown().await;
}

#[tokio::test]
async fn full_feeds_retain_deleted_comment_events_without_deleted_comment_data_or_actor_identity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "comment-feed-retained-events").await;
    let task_id = seed_task(&owner, &ws.slug, "comment-feed-retained-proj", "CFR").await;
    let document = owner
        .create_document(
            &ws.slug,
            "comment-feed-retained-proj",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Document feed parent".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document parent");
    let document_slug = document.slug.expect("document slug");

    let task_comment = owner
        .add_comment(
            &ws.slug,
            &task_id,
            CreateCommentRequest {
                body: "task comment that must disappear".into(),
            },
        )
        .await
        .expect("create task comment");
    let document_comment = owner
        .add_document_comment(
            &ws.slug,
            &document_slug,
            CreateCommentRequest {
                body: "document comment that must disappear".into(),
            },
        )
        .await
        .expect("create document comment");

    let target_document = owner
        .create_document(
            &ws.slug,
            "comment-feed-retained-proj",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Retained event target".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create retained event target");

    for comment_id in [task_comment.id, document_comment.id] {
        db.conn()
            .execute_unprepared(&format!(
                "INSERT INTO comment_links (id, workspace_id, comment_id, target_document_id, created_at) VALUES ('{}', '{}', '{comment_id}', '{}', now())",
                uuid::Uuid::now_v7(), ws.id.0, target_document.id,
            ))
            .await
            .expect("insert comment link");
    }

    let (viewer, _) = add_member(
        &db,
        &server,
        ws.id,
        "comment-feed-retained-viewer",
        MemberRole::Member,
    )
    .await;

    owner
        .delete_comment(&ws.slug, &task_id, task_comment.id)
        .await
        .expect("delete task comment");
    owner
        .delete_document_comment(&ws.slug, &document_slug, document_comment.id)
        .await
        .expect("delete document comment");

    db.conn()
        .execute_unprepared(&format!(
            "UPDATE users SET disabled_at = now() WHERE id = '{}'",
            owner_user.id.0
        ))
        .await
        .expect("disable deleted-comment actor");

    for url in [
        format!(
            "{}/api/workspaces/{}/tasks/{task_id}/comments?feed=full",
            server.base_url(),
            ws.slug
        ),
        format!(
            "{}/api/workspaces/{}/documents/{document_slug}/comments?feed=full",
            server.base_url(),
            ws.slug
        ),
    ] {
        let page: Value = reqwest::Client::new()
            .get(url)
            .bearer_auth(viewer.token().expect("viewer token"))
            .send()
            .await
            .expect("get retained event feed")
            .json()
            .await
            .expect("decode retained event feed");
        let items = page["items"].as_array().expect("feed items");

        assert_eq!(items.len(), 2, "only retained deletion events remain");
        assert!(items.iter().all(|entry| entry["type"] == "event"));
        assert!(items.iter().any(|entry| entry["kind"] == "link_removed"));
        assert!(items.iter().any(|entry| entry["kind"] == "comment_deleted"));
        for entry in items {
            assert_eq!(entry["actor"], Value::Null);
            assert!(entry.get("body").is_none());
            assert!(entry.get("links").is_none());
            assert!(entry.get("attachments").is_none());
        }
    }

    db.teardown().await;
}

#[tokio::test]
async fn full_feeds_fail_closed_with_the_internal_problem_for_invalid_cursors() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-feed-invalid-cursor").await;
    let task_id = seed_task(&owner, &ws.slug, "comment-feed-invalid-cursor-proj", "CFC").await;
    let document = owner
        .create_document(
            &ws.slug,
            "comment-feed-invalid-cursor-proj",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Document feed parent".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document parent");
    let document_slug = document.slug.expect("document slug");

    for url in [
        format!(
            "{}/api/workspaces/{}/tasks/{task_id}/comments?feed=full&cursor=not-a-feed-cursor",
            server.base_url(),
            ws.slug
        ),
        format!(
            "{}/api/workspaces/{}/documents/{document_slug}/comments?feed=full&cursor=not-a-feed-cursor",
            server.base_url(),
            ws.slug
        ),
    ] {
        let response = reqwest::Client::new()
            .get(url)
            .bearer_auth(owner.token().expect("owner token"))
            .send()
            .await
            .expect("invalid cursor response");
        let status = response.status();
        let problem: Value = response.json().await.expect("internal problem JSON");

        assert_eq!(status, reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(problem["type"], "urn:atlas:error:internal");
    }

    db.teardown().await;
}

#[tokio::test]
async fn full_feeds_recheck_target_access_after_the_link_is_written() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-feed-revoked-target").await;
    let task_id = seed_task(&owner, &ws.slug, "comment-feed-source", "CFS").await;
    owner
        .create_project(&ws.slug, project_req("comment-feed-target", "CFT"))
        .await
        .expect("create target project");
    let source_document = owner
        .create_document(
            &ws.slug,
            "comment-feed-source",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Source document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create source document");
    let source_slug = source_document.slug.expect("source slug");
    let target_document = owner
        .create_document(
            &ws.slug,
            "comment-feed-target",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Target document".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create target document");
    let task_comment = owner
        .add_comment(
            &ws.slug,
            &task_id,
            CreateCommentRequest {
                body: "task".into(),
            },
        )
        .await
        .expect("create task comment");
    let document_comment = owner
        .add_document_comment(
            &ws.slug,
            &source_slug,
            CreateCommentRequest {
                body: "document".into(),
            },
        )
        .await
        .expect("create document comment");
    for comment_id in [task_comment.id, document_comment.id] {
        db.conn()
            .execute_unprepared(&format!(
                "INSERT INTO comment_links (id, workspace_id, comment_id, target_document_id, created_at) VALUES ('{}', '{}', '{comment_id}', '{}', now())",
                uuid::Uuid::now_v7(), ws.id.0, target_document.id,
            ))
            .await
            .expect("insert target link");
    }
    let (viewer, _) = add_member(
        &db,
        &server,
        ws.id,
        "comment-feed-revoked-viewer",
        MemberRole::Member,
    )
    .await;
    db.conn()
        .execute_unprepared("UPDATE projects SET visibility = 'workspace', visibility_role = 'viewer' WHERE slug IN ('comment-feed-source', 'comment-feed-target')")
        .await
        .expect("make projects visible");

    let urls = [
        format!(
            "{}/api/workspaces/{}/tasks/{task_id}/comments?feed=full",
            server.base_url(),
            ws.slug
        ),
        format!(
            "{}/api/workspaces/{}/documents/{source_slug}/comments?feed=full",
            server.base_url(),
            ws.slug
        ),
    ];
    for url in &urls {
        let page: Value = reqwest::Client::new()
            .get(url)
            .bearer_auth(viewer.token().expect("viewer token"))
            .send()
            .await
            .expect("initial feed")
            .json()
            .await
            .expect("initial JSON");
        assert_eq!(
            page["items"][0]["links"][0]["target"]["status"],
            "available"
        );
    }
    db.conn()
        .execute_unprepared("UPDATE projects SET visibility = 'private', visibility_role = NULL WHERE slug = 'comment-feed-target'")
        .await
        .expect("revoke target visibility");
    for url in &urls {
        let page: Value = reqwest::Client::new()
            .get(url)
            .bearer_auth(viewer.token().expect("viewer token"))
            .send()
            .await
            .expect("revoked feed")
            .json()
            .await
            .expect("revoked JSON");
        assert_eq!(
            page["items"][0]["links"][0]["target"],
            serde_json::json!({"status":"unavailable","label":"Recurso no disponible"})
        );
    }
    db.teardown().await;
}

#[tokio::test]
async fn full_feeds_paginate_merged_comment_and_event_boundaries_without_gaps_or_duplicates() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, user) =
        support::login_user_with_workspace(&server, &db, "comment-feed-merged-pages").await;
    let task_id = seed_task(&owner, &ws.slug, "comment-feed-merged-pages", "CFP").await;
    let document = owner
        .create_document(
            &ws.slug,
            "comment-feed-merged-pages",
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Document parent".into(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let document_slug = document.slug.expect("document slug");
    let task_comments = [
        owner
            .add_comment(
                &ws.slug,
                &task_id,
                CreateCommentRequest {
                    body: "task one".into(),
                },
            )
            .await
            .expect("task comment one"),
        owner
            .add_comment(
                &ws.slug,
                &task_id,
                CreateCommentRequest {
                    body: "task two".into(),
                },
            )
            .await
            .expect("task comment two"),
    ];
    let document_comments = [
        owner
            .add_document_comment(
                &ws.slug,
                &document_slug,
                CreateCommentRequest {
                    body: "document one".into(),
                },
            )
            .await
            .expect("document comment one"),
        owner
            .add_document_comment(
                &ws.slug,
                &document_slug,
                CreateCommentRequest {
                    body: "document two".into(),
                },
            )
            .await
            .expect("document comment two"),
    ];
    for (task_parent, document_parent, comment_id) in [
        (task_comments[0].task_id, None, task_comments[1].id),
        (None, Some(document.id), document_comments[1].id),
    ] {
        let task_parent = task_parent
            .map(|id| format!("'{id}'"))
            .unwrap_or_else(|| "NULL".into());
        let document_parent = document_parent
            .map(|id| format!("'{id}'"))
            .unwrap_or_else(|| "NULL".into());
        db.conn().execute_unprepared(&format!("INSERT INTO comment_link_events (id, workspace_id, parent_task_id, parent_document_id, comment_id, event_kind, actor_type, actor_id, created_at) VALUES ('{}', '{}', {task_parent}, {document_parent}, '{comment_id}', 'comment_deleted', 'user', '{}', now())", uuid::Uuid::now_v7(), ws.id.0, user.id.0)).await.expect("insert retained event");
    }
    for (url, first, second) in [
        (
            format!(
                "{}/api/workspaces/{}/tasks/{task_id}/comments?feed=full&limit=2",
                server.base_url(),
                ws.slug
            ),
            task_comments[0].id,
            task_comments[1].id,
        ),
        (
            format!(
                "{}/api/workspaces/{}/documents/{document_slug}/comments?feed=full&limit=2",
                server.base_url(),
                ws.slug
            ),
            document_comments[0].id,
            document_comments[1].id,
        ),
    ] {
        let first_page: Value = reqwest::Client::new()
            .get(&url)
            .bearer_auth(owner.token().expect("owner token"))
            .send()
            .await
            .expect("first page")
            .json()
            .await
            .expect("first JSON");
        assert_eq!(
            first_page["items"].as_array().expect("first items").len(),
            2
        );
        assert_eq!(first_page["items"][0]["comment"]["id"], first.to_string());
        assert_eq!(first_page["items"][1]["comment"]["id"], second.to_string());
        let cursor = first_page["next_cursor"].as_str().expect("next cursor");
        let second_page: Value = reqwest::Client::new()
            .get(format!("{url}&cursor={cursor}"))
            .bearer_auth(owner.token().expect("owner token"))
            .send()
            .await
            .expect("second page")
            .json()
            .await
            .expect("second JSON");
        assert_eq!(
            second_page["items"].as_array().expect("second items").len(),
            1
        );
        assert_eq!(second_page["items"][0]["type"], "event");
        assert!(!second_page["has_more"].as_bool().expect("has more"));
    }
    db.teardown().await;
}

#[tokio::test]
async fn disabled_or_revoked_principals_are_rejected_before_full_feed_projection() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, user) =
        support::login_user_with_workspace(&server, &db, "comment-feed-rejected-principals").await;
    let task_id = seed_task(&owner, &ws.slug, "comment-feed-rejected-proj", "CFR").await;
    let url = format!(
        "{}/api/workspaces/{}/tasks/{task_id}/comments?feed=full",
        server.base_url(),
        ws.slug
    );

    db.conn()
        .execute_unprepared(&format!(
            "UPDATE users SET disabled_at = now() WHERE id = '{}'",
            user.id.0
        ))
        .await
        .expect("disable owner");
    let disabled = reqwest::Client::new()
        .get(&url)
        .bearer_auth(owner.token().expect("owner token"))
        .send()
        .await
        .expect("disabled owner response");
    assert_eq!(disabled.status(), reqwest::StatusCode::UNAUTHORIZED);

    db.conn()
        .execute_unprepared(&format!(
            "UPDATE users SET disabled_at = NULL WHERE id = '{}'",
            user.id.0
        ))
        .await
        .expect("reenable owner");
    let api_key = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "revoked-feed-reader".into(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".into(),
            }),
            scopes: Some(vec![ApiKeyScope::TasksRead]),
        })
        .await
        .expect("create api key");
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE api_keys SET revoked_at = now() WHERE id = '{}'",
            api_key.id
        ))
        .await
        .expect("revoke api key");
    let revoked = reqwest::Client::new()
        .get(&url)
        .bearer_auth(api_key.secret)
        .send()
        .await
        .expect("revoked key response");
    assert_eq!(revoked.status(), reqwest::StatusCode::UNAUTHORIZED);

    let creator_disabled_key = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "disabled-creator-feed-reader".into(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".into(),
            }),
            scopes: Some(vec![ApiKeyScope::TasksRead]),
        })
        .await
        .expect("create creator-disabled key");
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE users SET disabled_at = now() WHERE id = '{}'",
            user.id.0
        ))
        .await
        .expect("disable key creator");
    let creator_disabled = reqwest::Client::new()
        .get(url)
        .bearer_auth(creator_disabled_key.secret)
        .send()
        .await
        .expect("disabled creator response");
    assert_eq!(creator_disabled.status(), reqwest::StatusCode::UNAUTHORIZED);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Body validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_comment_rejects_blank_body() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-blank").await;

    let readable_id = seed_task(&client, &ws.slug, "comment-blank-proj", "CB").await;

    for blank in ["", "   ", "\n\t "] {
        let result = client
            .add_comment(
                &ws.slug,
                &readable_id,
                CreateCommentRequest {
                    body: blank.to_string(),
                },
            )
            .await;

        assert!(
            matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
            "blank body {blank:?} must be rejected with 422, got: {result:?}"
        );
    }

    let page = client
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments");
    assert!(
        page.items.is_empty(),
        "no comment must be persisted after validation failures"
    );

    db.teardown().await;
}

#[tokio::test]
async fn create_comment_rejects_body_over_max_length() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-oversize").await;

    let readable_id = seed_task(&client, &ws.slug, "comment-oversize-proj", "CO").await;

    let oversize_body = "a".repeat(10_001);

    let result = client
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: oversize_body,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "body over 10 000 characters must be rejected with 422, got: {result:?}"
    );

    let at_max_body = "a".repeat(10_000);
    client
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest { body: at_max_body },
        )
        .await
        .expect("body at exactly the max length must be accepted");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Not found
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_comment_task_not_found_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-task-404").await;

    let result = client
        .add_comment(
            &ws.slug,
            "NOPE-1",
            CreateCommentRequest {
                body: "orphan comment".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "posting to an unreachable task must return 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_comments_task_not_found_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-list-404").await;

    let result = client.list_comments(&ws.slug, "NOPE-1", None, None).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "listing comments on an unreachable task must return 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn delete_comment_missing_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-delete-404").await;

    let readable_id = seed_task(&client, &ws.slug, "comment-delete-404-proj", "CD").await;

    let result = client
        .delete_comment(&ws.slug, &readable_id, uuid::Uuid::new_v4())
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "deleting a missing comment must return 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn cross_workspace_comment_access_is_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws_a, _) = support::login_user_with_workspace(&server, &db, "comment-xws-a").await;
    let (other, _ws_b, _) = support::login_user_with_workspace(&server, &db, "comment-xws-b").await;

    let readable_id = seed_task(&owner, &ws_a.slug, "comment-xws-proj", "CX").await;
    let comment = owner
        .add_comment(
            &ws_a.slug,
            &readable_id,
            CreateCommentRequest {
                body: "workspace A comment".to_string(),
            },
        )
        .await
        .expect("create comment in workspace A");

    let list_result = other
        .list_comments(&ws_a.slug, &readable_id, None, None)
        .await;
    assert!(
        matches!(list_result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-member listing another workspace's task comments must get 404, got: {list_result:?}"
    );

    let create_result = other
        .add_comment(
            &ws_a.slug,
            &readable_id,
            CreateCommentRequest {
                body: "trespassing".to_string(),
            },
        )
        .await;
    assert!(
        matches!(create_result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-member posting to another workspace's task must get 404, got: {create_result:?}"
    );

    let delete_result = other
        .delete_comment(&ws_a.slug, &readable_id, comment.id)
        .await;
    assert!(
        matches!(delete_result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-member deleting another workspace's comment must get 404, got: {delete_result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Authorization
// ---------------------------------------------------------------------------

#[tokio::test]
async fn viewer_cannot_create_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-authz-owner").await;

    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Comment Authz Project".to_string(),
                slug: "comment-authz-proj".to_string(),
                task_prefix: "CAZ".to_string(),
                visibility: Some("workspace".to_string()),
                visibility_role: Some("viewer".to_string()),
            },
        )
        .await
        .expect("create project");

    let board = owner
        .create_board(
            &ws.slug,
            "comment-authz-proj",
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = owner
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

    let task = owner
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Viewer target".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    let (viewer, _) = add_member(
        &db,
        &server,
        ws.id,
        "comment-authz-viewer",
        MemberRole::Member,
    )
    .await;

    let result = viewer
        .add_comment(
            &ws.slug,
            &task.readable_id,
            CreateCommentRequest {
                body: "not allowed".to_string(),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "viewer must get 403 on comment create, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn author_deletes_own_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-self-delete").await;

    let readable_id = seed_task(&owner, &ws.slug, "comment-self-delete-proj", "SD").await;

    let comment = owner
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "mine".to_string(),
            },
        )
        .await
        .expect("create comment");

    owner
        .delete_comment(&ws.slug, &readable_id, comment.id)
        .await
        .expect("author must be able to delete their own comment");

    db.teardown().await;
}

#[tokio::test]
async fn non_author_non_admin_delete_is_forbidden() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-forbid-owner").await;

    let readable_id = seed_task(&owner, &ws.slug, "comment-forbid-proj", "CF").await;

    let comment = owner
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "owner's comment".to_string(),
            },
        )
        .await
        .expect("create comment");

    let (member, _) = add_member(
        &db,
        &server,
        ws.id,
        "comment-forbid-member",
        MemberRole::Member,
    )
    .await;

    let result = member
        .delete_comment(&ws.slug, &readable_id, comment.id)
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "non-author non-admin member must get 403 deleting another's comment, got: {result:?}"
    );

    let page = owner
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments");
    assert_eq!(
        page.items.len(),
        1,
        "comment must remain after a forbidden delete attempt"
    );

    db.teardown().await;
}

#[tokio::test]
async fn admin_can_delete_another_members_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-admin-owner").await;

    let readable_id = seed_task(&owner, &ws.slug, "comment-admin-proj", "CA").await;

    let (member, _) = add_member(
        &db,
        &server,
        ws.id,
        "comment-admin-author",
        MemberRole::Member,
    )
    .await;

    let comment = member
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "member's comment".to_string(),
            },
        )
        .await
        .expect("create comment");

    let (admin, _) = add_member(&db, &server, ws.id, "comment-admin-mod", MemberRole::Admin).await;

    admin
        .delete_comment(&ws.slug, &readable_id, comment.id)
        .await
        .expect("workspace admin must be able to delete another member's comment");

    let page = owner
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments");
    assert!(
        page.items.is_empty(),
        "admin-deleted comment must not appear in the list"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Author attribution on list: global / ungranted api-key author
// ---------------------------------------------------------------------------

/// A global api key (admitted via `is_global`, holding no workspace grant) authors
/// a comment. The batch list path must resolve its name by id — unscoped, the same
/// way the create path does — so its attribution is not lost just because it has no
/// active grant row in the workspace.
#[tokio::test]
async fn list_comments_preserves_global_api_key_author_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "comment-global-key").await;

    let readable_id = seed_task(&client, &ws.slug, "comment-global-key-proj", "GK").await;

    let api_key = client
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "global-bot".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![ApiKeyScope::TasksRead, ApiKeyScope::TasksUpdate]),
        })
        .await
        .expect("create api key");

    let mut api_key_client = atlas_client::AtlasClient::new(server.base_url().to_string());
    api_key_client.set_token(api_key.secret.clone());

    api_key_client
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "Posted while granted".to_string(),
            },
        )
        .await
        .expect("create comment as api key");

    // Promote the key to global and strip its workspace grant, so the old
    // grant-scoped author loader (`list_granted_in_workspace`) would no longer
    // return it, while the create path (`get_by_id`) still would.
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE api_keys SET is_global = true WHERE id = '{}'",
            api_key.id
        ))
        .await
        .expect("promote api key to global");
    db.conn()
        .execute_unprepared(&format!(
            "DELETE FROM permission_grants WHERE api_key_id = '{}'",
            api_key.id
        ))
        .await
        .expect("strip api key grant");

    let page = client
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments");

    assert_eq!(page.items.len(), 1);
    let author = &page.items[0].author;
    assert_eq!(author.r#type, "api_key");
    assert_eq!(author.id, api_key.id);
    assert_eq!(
        author.display_name.as_deref(),
        Some("global-bot"),
        "a global/ungranted api-key author's name must be preserved on list"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Edit: PATCH .../comments/{comment_id}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn author_edits_own_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-edit-owner").await;

    let readable_id = seed_task(&owner, &ws.slug, "comment-edit-proj", "CE1").await;

    let created = owner
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "original".to_string(),
            },
        )
        .await
        .expect("create comment");

    let updated = owner
        .update_comment(
            &ws.slug,
            &readable_id,
            created.id,
            UpdateCommentRequest {
                body: "edited body".to_string(),
            },
        )
        .await
        .expect("author must be able to edit their own comment");

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.body, "edited body");
    assert_eq!(updated.created_at, created.created_at);
    assert!(
        updated.updated_at >= created.updated_at,
        "updated_at must advance on edit"
    );

    let page = owner
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].body, "edited body");

    db.teardown().await;
}

#[tokio::test]
async fn admin_cannot_edit_another_members_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-edit-authz-owner").await;

    let readable_id = seed_task(&owner, &ws.slug, "comment-edit-authz-proj", "CE2").await;

    let (member, _) = add_member(
        &db,
        &server,
        ws.id,
        "comment-edit-author",
        MemberRole::Member,
    )
    .await;

    let comment = member
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "member's words".to_string(),
            },
        )
        .await
        .expect("create comment");

    let (admin, _) = add_member(&db, &server, ws.id, "comment-edit-admin", MemberRole::Admin).await;

    let admin_result = admin
        .update_comment(
            &ws.slug,
            &readable_id,
            comment.id,
            UpdateCommentRequest {
                body: "rewritten by admin".to_string(),
            },
        )
        .await;
    assert!(
        matches!(admin_result, Err(ClientError::Api(ref p)) if p.status == 403),
        "an admin/owner must not be able to edit another member's comment, got: {admin_result:?}"
    );

    let owner_result = owner
        .update_comment(
            &ws.slug,
            &readable_id,
            comment.id,
            UpdateCommentRequest {
                body: "rewritten by owner".to_string(),
            },
        )
        .await;
    assert!(
        matches!(owner_result, Err(ClientError::Api(ref p)) if p.status == 403),
        "the workspace owner must not be able to edit another member's comment, got: {owner_result:?}"
    );

    let page = owner
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments");
    assert_eq!(
        page.items[0].body, "member's words",
        "body must be untouched"
    );

    db.teardown().await;
}

#[tokio::test]
async fn non_member_cannot_edit_comment_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "comment-edit-xws-a").await;
    let (other, _ws_b, _) =
        support::login_user_with_workspace(&server, &db, "comment-edit-xws-b").await;

    let readable_id = seed_task(&owner, &ws_a.slug, "comment-edit-xws-proj", "CE3").await;
    let comment = owner
        .add_comment(
            &ws_a.slug,
            &readable_id,
            CreateCommentRequest {
                body: "workspace A comment".to_string(),
            },
        )
        .await
        .expect("create comment");

    let result = other
        .update_comment(
            &ws_a.slug,
            &readable_id,
            comment.id,
            UpdateCommentRequest {
                body: "trespassing edit".to_string(),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "a non-member editing another workspace's comment must get 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn edit_comment_rejects_invalid_body() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-edit-invalid").await;

    let readable_id = seed_task(&owner, &ws.slug, "comment-edit-invalid-proj", "CE4").await;

    let comment = owner
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "original".to_string(),
            },
        )
        .await
        .expect("create comment");

    for blank in ["", "   ", "\n\t "] {
        let result = owner
            .update_comment(
                &ws.slug,
                &readable_id,
                comment.id,
                UpdateCommentRequest {
                    body: blank.to_string(),
                },
            )
            .await;
        assert!(
            matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
            "blank edit body {blank:?} must be rejected with 422, got: {result:?}"
        );
    }

    let oversize = owner
        .update_comment(
            &ws.slug,
            &readable_id,
            comment.id,
            UpdateCommentRequest {
                body: "a".repeat(10_001),
            },
        )
        .await;
    assert!(
        matches!(oversize, Err(ClientError::Api(ref p)) if p.status == 422),
        "edit body over 10 000 characters must be rejected with 422, got: {oversize:?}"
    );

    let page = owner
        .list_comments(&ws.slug, &readable_id, None, None)
        .await
        .expect("list comments");
    assert_eq!(
        page.items[0].body, "original",
        "body must be unchanged after failed edits"
    );

    db.teardown().await;
}

#[tokio::test]
async fn edit_missing_comment_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-edit-missing").await;

    let readable_id = seed_task(&owner, &ws.slug, "comment-edit-missing-proj", "CE5").await;

    let result = owner
        .update_comment(
            &ws.slug,
            &readable_id,
            uuid::Uuid::new_v4(),
            UpdateCommentRequest {
                body: "edit into the void".to_string(),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "editing a missing comment must return 404, got: {result:?}"
    );

    db.teardown().await;
}

/// A member posts a comment while the project grants Editor by default, then the
/// project is downgraded to Viewer visibility. The edit floor is author-only
/// (`ViewerMin`, mirroring delete), so the now-Viewer author can still edit their
/// own comment, while a different Viewer member editing that comment still gets
/// 403 from the service's strict author-only check.
#[tokio::test]
async fn viewer_author_can_edit_own_comment_but_not_a_different_viewer() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "comment-viewer-edit-owner").await;

    let project_slug = "comment-viewer-edit-proj";
    let readable_id = seed_task(&owner, &ws.slug, project_slug, "CVE").await;

    let (author, _) = add_member(
        &db,
        &server,
        ws.id,
        "comment-viewer-edit-author",
        MemberRole::Member,
    )
    .await;

    let comment = author
        .add_comment(
            &ws.slug,
            &readable_id,
            CreateCommentRequest {
                body: "posted while editor".to_string(),
            },
        )
        .await
        .expect("member must be able to comment under the default editor visibility");

    owner
        .update_project(
            &ws.slug,
            project_slug,
            UpdateProjectRequest {
                visibility_role: Some("viewer".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("owner must be able to downgrade the project to viewer visibility");

    let updated = author
        .update_comment(
            &ws.slug,
            &readable_id,
            comment.id,
            UpdateCommentRequest {
                body: "edited after demotion to viewer".to_string(),
            },
        )
        .await
        .expect("a viewer who authored the comment must still be able to edit it");
    assert_eq!(updated.body, "edited after demotion to viewer");

    let (other_viewer, _) = add_member(
        &db,
        &server,
        ws.id,
        "comment-viewer-edit-other",
        MemberRole::Member,
    )
    .await;

    let result = other_viewer
        .update_comment(
            &ws.slug,
            &readable_id,
            comment.id,
            UpdateCommentRequest {
                body: "trespassing edit".to_string(),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "a viewer who is not the comment's author must get 403, got: {result:?}"
    );

    db.teardown().await;
}
