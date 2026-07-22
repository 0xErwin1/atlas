#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

mod support;

use atlas_domain::{
    entities::{
        comments::CommentOwner,
        documents::{NewAttachment, NewDocument},
        identity::MemberRole,
        security_audit::AuditFilters,
        workspace_core::{NewFolder, NewProject},
    },
    permissions::{Visibility, VisibilityRole},
    ports::{
        documents::{AttachmentRepo, DocumentRepo},
        security_audit::SecurityAuditRepo,
        workspace_core::{FolderRepo, ProjectRepo},
    },
};
use atlas_server::routes::registry::ROUTE_REGISTRY;
use atlas_server::{
    persistence::repos::{
        MembershipRepo, NewUser, PgAttachmentRepo, PgSecurityAuditRepo, UserRepo,
    },
    services::{CommentService, DocumentService},
};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

#[test]
fn trash_routes_are_registered_for_openapi_and_protection_sweeps() {
    assert!(
        ROUTE_REGISTRY.iter().any(|entry| {
            entry.method == "GET" && entry.openapi_path == Some("/api/admin/trash")
        })
    );
    assert!(ROUTE_REGISTRY.iter().any(|entry| {
        entry.method == "POST" && entry.openapi_path == Some("/api/admin/trash/restore")
    }));
}

async fn restore(
    client: &atlas_client::AtlasClient,
    server: &support::TestServer,
    kind: &str,
    target_id: uuid::Uuid,
) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}/api/admin/trash/restore", server.base_url()))
        .bearer_auth(client.token().expect("authenticated client token"))
        .json(&json!({ "kind": kind, "target_id": target_id }))
        .send()
        .await
        .expect("restore request")
}

async fn list_trash(
    client: &atlas_client::AtlasClient,
    server: &support::TestServer,
    query: &str,
) -> Value {
    let response = reqwest::Client::new()
        .get(format!("{}/api/admin/trash{query}", server.base_url()))
        .bearer_auth(client.token().expect("authenticated client token"))
        .send()
        .await
        .expect("trash list request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json().await.expect("decode trash page")
}

async fn trash_response(
    client: &atlas_client::AtlasClient,
    server: &support::TestServer,
    method: reqwest::Method,
    path: &str,
) -> reqwest::Response {
    reqwest::Client::new()
        .request(method, format!("{}{}", server.base_url(), path))
        .bearer_auth(client.token().expect("authenticated client token"))
        .send()
        .await
        .expect("trash request")
}

async fn login_system_admin(
    server: &support::TestServer,
    db: &support::TestDb,
    username: &str,
) -> atlas_client::AtlasClient {
    let password = "TestPassword1!";
    let password_hash = atlas_server::auth::password::hash(password.to_string())
        .await
        .expect("hash password");
    let user = db
        .user_repo()
        .create(NewUser {
            username: username.into(),
            display_name: username.into(),
            email: None,
            password_hash: Some(password_hash),
            is_root: false,
            is_system_admin: true,
        })
        .await
        .expect("create system admin");
    support::activate_user_in_db(db, user.id.0).await;

    let mut client = server.client();
    client
        .login(atlas_api::dtos::LoginRequest {
            username: username.into(),
            password: password.into(),
        })
        .await
        .expect("system admin login");
    client
}

async fn attachment_is_deleted(db: &support::TestDb, attachment_id: uuid::Uuid) -> bool {
    db.conn()
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at IS NOT NULL AS deleted FROM attachments WHERE id = $1",
            [attachment_id.into()],
        ))
        .await
        .expect("query attachment")
        .expect("attachment row")
        .try_get("", "deleted")
        .expect("decode deleted flag")
}

#[tokio::test]
async fn root_lists_and_restores_each_first_class_trash_kind() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace, owner) = support::seed_workspace(&db, "trash-five-kinds").await;
    let ctx = support::ctx(&workspace, &owner);
    let attachments = PgAttachmentRepo {
        conn: db.conn().clone(),
    };

    let project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Trash project".into(),
                slug: "trash-five-project".into(),
                task_prefix: "TFK".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");
    let folder = db
        .folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "Trash folder".into(),
            },
        )
        .await
        .expect("create folder");
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Trash document".into(),
                slug: Some("trash-five-document".into()),
                content: "body".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");
    let comment_parent = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Comment parent".into(),
                slug: Some("trash-five-comment-parent".into()),
                content: "body".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create comment parent");
    let comment = CommentService::new(db.conn().clone())
        .create(
            &ctx,
            CommentOwner::Document(comment_parent.id),
            "Trash comment".into(),
        )
        .await
        .expect("create comment");
    let attachment = attachments
        .record(
            &ctx,
            NewAttachment {
                document_id: Some(comment_parent.id),
                task_id: None,
                comment_id: None,
                file_name: "trash.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "a".repeat(64),
            },
        )
        .await
        .expect("create attachment");

    db.project_repo()
        .soft_delete(&ctx, project.id)
        .await
        .expect("delete project");
    db.folder_repo()
        .soft_delete(&ctx, folder.id)
        .await
        .expect("delete folder");
    DocumentService::new(db.conn().clone(), 25)
        .soft_delete(&ctx, document.id)
        .await
        .expect("delete document");
    CommentService::new(db.conn().clone())
        .remove(
            &ctx,
            CommentOwner::Document(comment_parent.id),
            comment.id,
            false,
        )
        .await
        .expect("delete comment");
    attachments
        .soft_delete(&ctx, attachment.id)
        .await
        .expect("delete attachment");

    let page = list_trash(&admin, &server, "?limit=20").await;
    let kinds = page["items"]
        .as_array()
        .expect("trash items")
        .iter()
        .filter(|item| item["workspace_id"] == workspace.id.0.to_string())
        .map(|item| item["kind"].as_str().expect("kind"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        kinds,
        std::collections::BTreeSet::from([
            "project",
            "folder",
            "document",
            "comment",
            "attachment"
        ])
    );

    for (kind, id) in [
        ("project", project.id.0),
        ("folder", folder.id.0),
        ("document", document.id.0),
        ("comment", comment.id.0),
        ("attachment", attachment.id.0),
    ] {
        let first = restore(&admin, &server, kind, id).await;
        let first_status = first.status();
        let first_body = first.text().await.expect("restore response body");
        assert_eq!(
            first_status,
            reqwest::StatusCode::NO_CONTENT,
            "{kind}: {first_body}"
        );
        assert_eq!(
            restore(&admin, &server, kind, id).await.status(),
            reqwest::StatusCode::NO_CONTENT
        );
    }

    db.teardown().await;
}

#[tokio::test]
async fn restore_reports_deleted_parent_with_a_parent_specific_problem() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace, owner) = support::seed_workspace(&db, "trash-parent-conflict").await;
    let ctx = support::ctx(&workspace, &owner);

    let project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Trash parent conflict".into(),
                slug: "trash-parent-conflict".into(),
                task_prefix: "TPC".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");
    let folder = db
        .folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: Some(project.id),
                parent_folder_id: None,
                name: "Deleted parent".into(),
            },
        )
        .await
        .expect("create folder");
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Blocked document".into(),
                slug: Some("trash-parent-conflict-document".into()),
                content: "body".into(),
                folder_id: Some(folder.id),
                project_id: Some(project.id),
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    DocumentService::new(db.conn().clone(), 25)
        .soft_delete(&ctx, document.id)
        .await
        .expect("delete document");
    db.folder_repo()
        .soft_delete(&ctx, folder.id)
        .await
        .expect("delete folder");

    let response = restore(&admin, &server, "document", document.id.0).await;
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let problem: Value = response.json().await.expect("decode problem");
    assert_eq!(
        problem["detail"],
        "restore is blocked because the document's parent is deleted"
    );
    assert_eq!(
        problem["hint"],
        "Restore the deleted parent before restoring this item."
    );

    db.teardown().await;
}

#[tokio::test]
async fn restore_reports_live_identity_with_an_identity_specific_problem() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace, owner) = support::seed_workspace(&db, "trash-identity-conflict").await;
    let ctx = support::ctx(&workspace, &owner);
    let repo = db.doc_repo();

    let deleted = repo
        .create(
            &ctx,
            NewDocument {
                title: "Deleted identity".into(),
                slug: Some("trash-identity-conflict-document".into()),
                content: "body".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create deleted document");
    DocumentService::new(db.conn().clone(), 25)
        .soft_delete(&ctx, deleted.id)
        .await
        .expect("delete document");
    repo.create(
        &ctx,
        NewDocument {
            title: "Live identity".into(),
            slug: Some("trash-identity-conflict-document".into()),
            content: "body".into(),
            folder_id: None,
            project_id: None,
            frontmatter: None,
        },
    )
    .await
    .expect("reuse deleted slug");

    let response = restore(&admin, &server, "document", deleted.id.0).await;
    assert_eq!(response.status(), reqwest::StatusCode::CONFLICT);
    let problem: Value = response.json().await.expect("decode problem");
    assert_eq!(
        problem["detail"],
        "restore is blocked because a live document has the same identity"
    );
    assert_eq!(
        problem["hint"],
        "Resolve the live conflicting identity before restoring this item."
    );

    db.teardown().await;
}

#[tokio::test]
async fn trash_filters_and_tied_timestamp_pages_are_complete_without_duplicates() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace_a, owner_a) = support::seed_workspace(&db, "trash-page-a").await;
    let (workspace_b, owner_b) = support::seed_workspace(&db, "trash-page-b").await;
    let ctx_a = support::ctx(&workspace_a, &owner_a);
    let ctx_b = support::ctx(&workspace_b, &owner_b);

    let mut document_ids = Vec::new();
    for (index, ctx) in [&ctx_a, &ctx_a, &ctx_a, &ctx_b].into_iter().enumerate() {
        let document = db
            .doc_repo()
            .create(
                ctx,
                NewDocument {
                    title: format!("Trash page {index}"),
                    slug: Some(format!("trash-page-{index}")),
                    content: "body".into(),
                    folder_id: None,
                    project_id: None,
                    frontmatter: None,
                },
            )
            .await
            .expect("create document");
        DocumentService::new(db.conn().clone(), 25)
            .soft_delete(ctx, document.id)
            .await
            .expect("delete document");
        document_ids.push(document.id.0);
    }
    db.conn()
        .execute_unprepared(
            "UPDATE documents SET deleted_at = '2026-07-21T00:00:00Z' WHERE slug LIKE 'trash-page-%'",
        )
        .await
        .expect("tie deletion timestamps");

    let filtered = list_trash(
        &admin,
        &server,
        &format!("?workspace_id={}&kind=document", workspace_a.id.0),
    )
    .await;
    let filtered_ids = filtered["items"]
        .as_array()
        .expect("filtered items")
        .iter()
        .map(|item| item["target_id"].as_str().expect("target id").to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        filtered_ids,
        document_ids[..3]
            .iter()
            .map(uuid::Uuid::to_string)
            .collect::<std::collections::BTreeSet<_>>()
    );

    let mut seen = std::collections::BTreeSet::new();
    let mut cursor = None;
    loop {
        let query = cursor
            .as_deref()
            .map(|value| format!("?kind=document&limit=1&cursor={value}"))
            .unwrap_or_else(|| "?kind=document&limit=1".into());
        let page = list_trash(&admin, &server, &query).await;
        for item in page["items"].as_array().expect("page items") {
            seen.insert(item["target_id"].as_str().expect("target id").to_string());
        }
        cursor = page["next_cursor"].as_str().map(str::to_string);
        if !page["has_more"].as_bool().expect("has more") {
            break;
        }
        assert!(cursor.is_some(), "a non-final page must provide a cursor");
    }
    assert_eq!(
        seen,
        document_ids
            .iter()
            .map(uuid::Uuid::to_string)
            .collect::<std::collections::BTreeSet<_>>()
    );

    db.teardown().await;
}

#[tokio::test]
async fn trash_allows_only_root_and_system_admin_humans() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let root = support::login_root_user(&server, &db).await;
    let system_admin = login_system_admin(&server, &db, "trash-system-admin").await;
    let (_workspace_owner, workspace, owner) =
        support::login_user_with_workspace(&server, &db, "trash-workspace-owner").await;
    let (workspace_admin, admin_user) =
        support::login_user(&server, &db, "trash-workspace-admin").await;
    let (member, _) = support::login_user(&server, &db, "trash-member").await;
    db.membership_repo()
        .add(
            &support::ctx(&workspace, &owner),
            admin_user.id,
            MemberRole::Admin,
        )
        .await
        .expect("add workspace admin");
    let api_key = root
        .create_user_api_key(atlas_api::dtos::CreateUserApiKeyRequest {
            name: "trash-agent".into(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
            scopes: None,
        })
        .await
        .expect("create api key");
    let mut key_client = server.client();
    key_client.set_token(api_key.secret);

    for client in [&root, &system_admin] {
        assert_eq!(
            trash_response(client, &server, reqwest::Method::GET, "/api/admin/trash")
                .await
                .status(),
            reqwest::StatusCode::OK
        );
    }
    for client in [&workspace_admin, &member, &key_client] {
        assert_eq!(
            trash_response(client, &server, reqwest::Method::GET, "/api/admin/trash")
                .await
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
    }

    db.teardown().await;
}

#[tokio::test]
async fn restore_is_idempotent_and_preserves_independently_deleted_children() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace, owner) = support::seed_workspace(&db, "trash-child-preservation").await;
    let ctx = support::ctx(&workspace, &owner);
    let attachments = PgAttachmentRepo {
        conn: db.conn().clone(),
    };
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Restore parent".into(),
                slug: Some("trash-child-preservation".into()),
                content: "body".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");
    let attachment = attachments
        .record(
            &ctx,
            NewAttachment {
                document_id: Some(document.id),
                task_id: None,
                comment_id: None,
                file_name: "independent.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "b".repeat(64),
            },
        )
        .await
        .expect("create attachment");
    attachments
        .soft_delete(&ctx, attachment.id)
        .await
        .expect("delete attachment independently");
    DocumentService::new(db.conn().clone(), 25)
        .soft_delete(&ctx, document.id)
        .await
        .expect("delete document");

    assert_eq!(
        restore(&admin, &server, "document", document.id.0)
            .await
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert!(attachment_is_deleted(&db, attachment.id.0).await);
    assert_eq!(
        restore(&admin, &server, "document", document.id.0)
            .await
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(
        restore(&admin, &server, "attachment", uuid::Uuid::now_v7())
            .await
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    assert_eq!(
        restore(&admin, &server, "attachment", document.id.0)
            .await
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );

    db.teardown().await;
}

#[tokio::test]
async fn comment_restore_uses_its_tombstone_timestamp_and_writes_one_audit_event() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace, owner) = support::seed_workspace(&db, "trash-comment-restore").await;
    let ctx = support::ctx(&workspace, &owner);
    let attachments = PgAttachmentRepo {
        conn: db.conn().clone(),
    };
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Comment owner".into(),
                slug: Some("trash-comment-restore".into()),
                content: "body".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");
    let comment = CommentService::new(db.conn().clone())
        .create(&ctx, CommentOwner::Document(document.id), "comment".into())
        .await
        .expect("create comment");
    let live_attachment = attachments
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: None,
                comment_id: Some(comment.id),
                file_name: "restored.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "c".repeat(64),
            },
        )
        .await
        .expect("create live attachment");
    let independently_deleted = attachments
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: None,
                comment_id: Some(comment.id),
                file_name: "retained.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "d".repeat(64),
            },
        )
        .await
        .expect("create independently deleted attachment");
    attachments
        .soft_delete(&ctx, independently_deleted.id)
        .await
        .expect("delete attachment independently");
    CommentService::new(db.conn().clone())
        .remove(&ctx, CommentOwner::Document(document.id), comment.id, false)
        .await
        .expect("delete comment");

    assert_eq!(
        restore(&admin, &server, "comment", comment.id.0)
            .await
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert!(!attachment_is_deleted(&db, live_attachment.id.0).await);
    assert!(attachment_is_deleted(&db, independently_deleted.id.0).await);
    assert_eq!(
        restore(&admin, &server, "comment", comment.id.0)
            .await
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    let restored = PgSecurityAuditRepo::new(db.conn().clone())
        .list_for_workspace(workspace.id, &AuditFilters::default(), None, 100)
        .await
        .expect("list audit rows")
        .into_iter()
        .filter(|row| row.action == "resource.restored" && row.target_id == Some(comment.id.0))
        .count();
    assert_eq!(
        restored, 1,
        "retry must not append a second restore audit event"
    );

    db.teardown().await;
}

#[tokio::test]
async fn restoring_one_document_restores_its_normal_route_without_reviving_another() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (client, workspace, owner) =
        support::login_user_with_workspace(&server, &db, "trash-normal-routes").await;
    let ctx = support::ctx(&workspace, &owner);
    let create_document = |title: &str, slug: &str| NewDocument {
        title: title.into(),
        slug: Some(slug.into()),
        content: "body".into(),
        folder_id: None,
        project_id: None,
        frontmatter: None,
    };
    let restored = db
        .doc_repo()
        .create(&ctx, create_document("Restored", "trash-normal-restored"))
        .await
        .expect("create restored document");
    let still_deleted = db
        .doc_repo()
        .create(&ctx, create_document("Deleted", "trash-normal-deleted"))
        .await
        .expect("create deleted document");
    let document_service = DocumentService::new(db.conn().clone(), 25);
    document_service
        .soft_delete(&ctx, restored.id)
        .await
        .expect("delete restored document");
    document_service
        .soft_delete(&ctx, still_deleted.id)
        .await
        .expect("delete still-deleted document");

    assert_eq!(
        restore(&admin, &server, "document", restored.id.0)
            .await
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    for (slug, expected) in [
        ("trash-normal-restored", reqwest::StatusCode::OK),
        ("trash-normal-deleted", reqwest::StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            client
                .http_client()
                .get(format!(
                    "{}/api/workspaces/{}/documents/{slug}",
                    server.base_url(),
                    workspace.slug
                ))
                .bearer_auth(client.token().expect("client token"))
                .send()
                .await
                .expect("get normal document route")
                .status(),
            expected
        );
    }

    db.teardown().await;
}
