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
};
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, UserRepo};

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
