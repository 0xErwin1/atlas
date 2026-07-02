#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateProjectRequest,
    boards_tasks::{CreateCommentRequest, UpdateCommentRequest},
    documents::CreateDocumentRequest,
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
) -> atlas_client::AtlasClient {
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

    client
}

/// Creates a project and a document in it; returns the document's `(slug, id)`.
async fn seed_document(
    client: &atlas_client::AtlasClient,
    ws_slug: &str,
    proj_slug: &str,
    prefix: &str,
) -> (String, uuid::Uuid) {
    client
        .create_project(ws_slug, project_req(proj_slug, prefix))
        .await
        .expect("create project");

    let doc = client
        .create_document(
            ws_slug,
            proj_slug,
            CreateDocumentRequest {
                title: "Doc with comments".to_string(),
                folder_id: None,
                content: Some("# Doc\n\nBody".to_string()),
            },
        )
        .await
        .expect("create document");

    (doc.slug.expect("document must have a slug"), doc.id)
}

// ---------------------------------------------------------------------------
// Create + list + delete roundtrip, with polymorphic-DTO assertions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_list_delete_document_comment_roundtrip() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "doc-comment-roundtrip").await;

    let (slug, doc_id) = seed_document(&client, &ws.slug, "doc-comment-proj", "DC").await;

    let created = client
        .add_document_comment(
            &ws.slug,
            &slug,
            CreateCommentRequest {
                body: "First document comment".to_string(),
            },
        )
        .await
        .expect("create document comment");

    assert_eq!(created.body, "First document comment");
    assert_eq!(created.author.r#type, "user");
    assert_eq!(created.document_id, Some(doc_id), "DTO must carry document_id");
    assert!(
        created.task_id.is_none(),
        "a document comment must not carry a task_id"
    );

    let page = client
        .list_document_comments(&ws.slug, &slug, None, None)
        .await
        .expect("list document comments");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, created.id);
    assert_eq!(page.items[0].document_id, Some(doc_id));
    assert!(!page.has_more);

    client
        .delete_document_comment(&ws.slug, &slug, created.id)
        .await
        .expect("delete document comment");

    let after = client
        .list_document_comments(&ws.slug, &slug, None, None)
        .await
        .expect("list after delete");
    assert!(after.items.is_empty());

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Empty state + oldest-first cursor pagination
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_document_comments_empty_state() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "doc-comment-empty").await;

    let (slug, _) = seed_document(&client, &ws.slug, "doc-comment-empty-proj", "DE").await;

    let page = client
        .list_document_comments(&ws.slug, &slug, None, None)
        .await
        .expect("list on document with no comments");

    assert!(page.items.is_empty());
    assert!(!page.has_more);
    assert!(page.next_cursor.is_none());

    db.teardown().await;
}

#[tokio::test]
async fn list_document_comments_oldest_first_cursor_walk() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "doc-comment-cursor").await;

    let (slug, _) = seed_document(&client, &ws.slug, "doc-comment-cursor-proj", "DK").await;

    let mut created_ids = Vec::new();
    for i in 0..5 {
        let c = client
            .add_document_comment(
                &ws.slug,
                &slug,
                CreateCommentRequest {
                    body: format!("Comment {i}"),
                },
            )
            .await
            .expect("create comment");
        created_ids.push(c.id);
    }

    let mut collected = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = client
            .list_document_comments(&ws.slug, &slug, cursor.as_deref(), Some(2))
            .await
            .expect("list page");
        collected.extend(page.items.iter().map(|c| c.id));
        if !page.has_more {
            assert!(page.next_cursor.is_none());
            break;
        }
        cursor = page.next_cursor;
        assert!(cursor.is_some());
    }

    assert_eq!(
        collected, created_ids,
        "cursor walk must return every comment exactly once, oldest-first"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Validation + not found
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_document_comment_rejects_invalid_body() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "doc-comment-invalid").await;

    let (slug, _) = seed_document(&client, &ws.slug, "doc-comment-invalid-proj", "DI").await;

    for blank in ["", "   ", "\n\t "] {
        let result = client
            .add_document_comment(
                &ws.slug,
                &slug,
                CreateCommentRequest {
                    body: blank.to_string(),
                },
            )
            .await;
        assert!(
            matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
            "blank body {blank:?} must be 422, got: {result:?}"
        );
    }

    let oversize = client
        .add_document_comment(
            &ws.slug,
            &slug,
            CreateCommentRequest {
                body: "a".repeat(10_001),
            },
        )
        .await;
    assert!(
        matches!(oversize, Err(ClientError::Api(ref p)) if p.status == 422),
        "oversize body must be 422, got: {oversize:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn document_comment_document_not_found_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "doc-comment-404").await;

    let post = client
        .add_document_comment(
            &ws.slug,
            "nonexistent-doc",
            CreateCommentRequest {
                body: "orphan".to_string(),
            },
        )
        .await;
    assert!(
        matches!(post, Err(ClientError::Api(ref p)) if p.status == 404),
        "posting to an unknown document must be 404, got: {post:?}"
    );

    let list = client
        .list_document_comments(&ws.slug, "nonexistent-doc", None, None)
        .await;
    assert!(
        matches!(list, Err(ClientError::Api(ref p)) if p.status == 404),
        "listing an unknown document's comments must be 404, got: {list:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Authorization + author-only edit + moderation delete + cross-workspace
// ---------------------------------------------------------------------------

#[tokio::test]
async fn viewer_cannot_create_document_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-comment-authz-owner").await;

    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Doc Authz".to_string(),
                slug: "doc-comment-authz-proj".to_string(),
                task_prefix: "DAZ".to_string(),
                visibility: Some("workspace".to_string()),
                visibility_role: Some("viewer".to_string()),
            },
        )
        .await
        .expect("create project");

    let doc = owner
        .create_document(
            &ws.slug,
            "doc-comment-authz-proj",
            CreateDocumentRequest {
                title: "Viewer target doc".to_string(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = doc.slug.expect("slug");

    let viewer = add_member(&db, &server, ws.id, "doc-comment-viewer", MemberRole::Member).await;

    let result = viewer
        .add_document_comment(
            &ws.slug,
            &slug,
            CreateCommentRequest {
                body: "not allowed".to_string(),
            },
        )
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "viewer must get 403 creating a document comment, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn author_edits_own_document_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-comment-edit").await;

    let (slug, _) = seed_document(&owner, &ws.slug, "doc-comment-edit-proj", "DED").await;

    let created = owner
        .add_document_comment(
            &ws.slug,
            &slug,
            CreateCommentRequest {
                body: "original".to_string(),
            },
        )
        .await
        .expect("create comment");

    let updated = owner
        .update_document_comment(
            &ws.slug,
            &slug,
            created.id,
            UpdateCommentRequest {
                body: "edited body".to_string(),
            },
        )
        .await
        .expect("author must be able to edit their own comment");

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.body, "edited body");
    assert!(updated.updated_at >= created.updated_at);

    db.teardown().await;
}

#[tokio::test]
async fn admin_deletes_but_cannot_edit_another_members_document_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-comment-mod-owner").await;

    let (slug, _) = seed_document(&owner, &ws.slug, "doc-comment-mod-proj", "DMD").await;

    let member = add_member(&db, &server, ws.id, "doc-comment-mod-author", MemberRole::Member).await;
    let comment = member
        .add_document_comment(
            &ws.slug,
            &slug,
            CreateCommentRequest {
                body: "member words".to_string(),
            },
        )
        .await
        .expect("create comment");

    let admin = add_member(&db, &server, ws.id, "doc-comment-mod-admin", MemberRole::Admin).await;

    let edit = admin
        .update_document_comment(
            &ws.slug,
            &slug,
            comment.id,
            UpdateCommentRequest {
                body: "rewritten by admin".to_string(),
            },
        )
        .await;
    assert!(
        matches!(edit, Err(ClientError::Api(ref p)) if p.status == 403),
        "admin must not edit another member's comment, got: {edit:?}"
    );

    admin
        .delete_document_comment(&ws.slug, &slug, comment.id)
        .await
        .expect("admin must be able to delete another member's comment");

    let page = owner
        .list_document_comments(&ws.slug, &slug, None, None)
        .await
        .expect("list");
    assert!(page.items.is_empty(), "admin-deleted comment must be gone");

    db.teardown().await;
}

#[tokio::test]
async fn cross_workspace_document_comment_is_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws_a, _) =
        support::login_user_with_workspace(&server, &db, "doc-comment-xws-a").await;
    let (other, _ws_b, _) =
        support::login_user_with_workspace(&server, &db, "doc-comment-xws-b").await;

    let (slug, _) = seed_document(&owner, &ws_a.slug, "doc-comment-xws-proj", "DX").await;
    let comment = owner
        .add_document_comment(
            &ws_a.slug,
            &slug,
            CreateCommentRequest {
                body: "workspace A".to_string(),
            },
        )
        .await
        .expect("create comment");

    let list = other
        .list_document_comments(&ws_a.slug, &slug, None, None)
        .await;
    assert!(
        matches!(list, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-member listing another workspace's document comments must be 404, got: {list:?}"
    );

    let del = other
        .delete_document_comment(&ws_a.slug, &slug, comment.id)
        .await;
    assert!(
        matches!(del, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-member deleting another workspace's document comment must be 404, got: {del:?}"
    );

    db.teardown().await;
}
