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
};
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, UserRepo};
use sea_orm::ConnectionTrait;

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
