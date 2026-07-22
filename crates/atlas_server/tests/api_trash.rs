#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

mod support;

use atlas_domain::{
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        comments::CommentOwner,
        documents::{NewAttachment, NewDocument},
        identity::MemberRole,
        security_audit::AuditFilters,
        workspace_core::{NewFolder, NewProject},
    },
    permissions::{Visibility, VisibilityRole},
    ports::{
        boards_tasks::{BoardRepo, TaskRepo},
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
    assert!(ROUTE_REGISTRY.iter().any(|entry| {
        entry.method == "POST" && entry.openapi_path == Some("/api/admin/trash/purge")
    }));
    assert!(ROUTE_REGISTRY.iter().any(|entry| {
        entry.method == "GET"
            && entry.openapi_path == Some("/api/admin/trash/purges/{operation_id}")
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

async fn purge(
    client: &atlas_client::AtlasClient,
    server: &support::TestServer,
    kind: &str,
    target_id: uuid::Uuid,
    confirm: bool,
) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{}/api/admin/trash/purge", server.base_url()))
        .bearer_auth(client.token().expect("authenticated client token"))
        .json(&json!({
            "kind": kind,
            "target_id": target_id,
            "confirm": confirm,
        }))
        .send()
        .await
        .expect("purge request")
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

async fn document_is_deleted(db: &support::TestDb, document_id: uuid::Uuid) -> bool {
    db.conn()
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at IS NOT NULL AS deleted FROM documents WHERE id = $1",
            [document_id.into()],
        ))
        .await
        .expect("query document")
        .expect("document row")
        .try_get("", "deleted")
        .expect("decode deleted flag")
}

async fn count_purge_side_effects(db: &support::TestDb, target_id: uuid::Uuid) -> (i64, i64) {
    let row = db
        .conn()
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT \
                (SELECT count(*)::bigint FROM purge_operations WHERE target_id = $1) AS operations, \
                (SELECT count(*)::bigint FROM security_audit_log WHERE target_id = $1 AND action = 'resource.purge_committed') AS audits",
            [target_id.into()],
        ))
        .await
        .expect("count purge side effects")
        .expect("purge side effects row");

    (
        row.try_get("", "operations").expect("operation count"),
        row.try_get("", "audits").expect("audit count"),
    )
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
async fn restore_reports_project_and_folder_identity_conflicts_with_actionable_problems() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace, owner) = support::seed_workspace(&db, "trash-project-folder-conflicts").await;
    let ctx = support::ctx(&workspace, &owner);

    let deleted_project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Deleted project".into(),
                slug: "trash-project-identity".into(),
                task_prefix: "TPI".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create deleted project");
    db.project_repo()
        .soft_delete(&ctx, deleted_project.id)
        .await
        .expect("delete project");
    db.project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Live project".into(),
                slug: "trash-project-identity".into(),
                task_prefix: "TPI".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("reuse deleted project identity");

    let project_response = restore(&admin, &server, "project", deleted_project.id.0).await;
    assert_eq!(project_response.status(), reqwest::StatusCode::CONFLICT);
    let project_problem: Value = project_response
        .json()
        .await
        .expect("decode project problem");
    assert_eq!(
        project_problem["detail"],
        "restore is blocked because a live project has the same identity"
    );
    assert_eq!(
        project_problem["hint"],
        "Resolve the live conflicting identity before restoring this item."
    );

    let deleted_folder = db
        .folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "Trash folder identity".into(),
            },
        )
        .await
        .expect("create deleted folder");
    db.folder_repo()
        .soft_delete(&ctx, deleted_folder.id)
        .await
        .expect("delete folder");
    db.folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "Trash folder identity".into(),
            },
        )
        .await
        .expect("reuse deleted folder identity");

    let folder_response = restore(&admin, &server, "folder", deleted_folder.id.0).await;
    assert_eq!(folder_response.status(), reqwest::StatusCode::CONFLICT);
    let folder_problem: Value = folder_response.json().await.expect("decode folder problem");
    assert_eq!(
        folder_problem["detail"],
        "restore is blocked because a live folder has the same identity"
    );
    assert_eq!(
        folder_problem["hint"],
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

    let project = db
        .project_repo()
        .create(
            &ctx_a,
            NewProject {
                name: "Mixed page project".into(),
                slug: "trash-mixed-page-project".into(),
                task_prefix: "TMP".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create mixed page project");
    let folder = db
        .folder_repo()
        .create(
            &ctx_a,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "Mixed page folder".into(),
            },
        )
        .await
        .expect("create mixed page folder");
    let comment_parent = db
        .doc_repo()
        .create(
            &ctx_a,
            NewDocument {
                title: "Mixed page comment parent".into(),
                slug: Some("trash-mixed-page-comment-parent".into()),
                content: "body".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create mixed page comment parent");
    let comment = CommentService::new(db.conn().clone())
        .create(
            &ctx_a,
            CommentOwner::Document(comment_parent.id),
            "mixed page comment".into(),
        )
        .await
        .expect("create mixed page comment");
    let attachments = PgAttachmentRepo {
        conn: db.conn().clone(),
    };
    let attachment = attachments
        .record(
            &ctx_a,
            NewAttachment {
                document_id: Some(comment_parent.id),
                task_id: None,
                comment_id: None,
                file_name: "mixed-page.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "e".repeat(64),
            },
        )
        .await
        .expect("create mixed page attachment");
    db.project_repo()
        .soft_delete(&ctx_a, project.id)
        .await
        .expect("delete mixed page project");
    db.folder_repo()
        .soft_delete(&ctx_a, folder.id)
        .await
        .expect("delete mixed page folder");
    CommentService::new(db.conn().clone())
        .remove(
            &ctx_a,
            CommentOwner::Document(comment_parent.id),
            comment.id,
            false,
        )
        .await
        .expect("delete mixed page comment");
    attachments
        .soft_delete(&ctx_a, attachment.id)
        .await
        .expect("delete mixed page attachment");
    for table in ["projects", "folders", "comments", "attachments"] {
        db.conn()
            .execute_unprepared(&format!(
                "UPDATE {table} SET deleted_at = '2026-07-21T00:00:00Z' WHERE deleted_at IS NOT NULL"
            ))
            .await
            .expect("tie mixed deletion timestamps");
    }

    let mut seen = std::collections::BTreeSet::new();
    let mut cursor = None;
    loop {
        let query = cursor
            .as_deref()
            .map(|value| format!("?limit=1&cursor={value}"))
            .unwrap_or_else(|| "?limit=1".into());
        let page = list_trash(&admin, &server, &query).await;
        for item in page["items"].as_array().expect("page items") {
            assert!(
                seen.insert(item["target_id"].as_str().expect("target id").to_string()),
                "a cursor page must not repeat an item"
            );
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
            .chain([
                project.id.0.to_string(),
                folder.id.0.to_string(),
                comment.id.0.to_string(),
                attachment.id.0.to_string(),
            ])
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

    let ctx = support::ctx(&workspace, &owner);
    let create_deleted_document = |title: &str, slug: &str| NewDocument {
        title: title.into(),
        slug: Some(slug.into()),
        content: "body".into(),
        folder_id: None,
        project_id: None,
        frontmatter: None,
    };
    let root_document = db
        .doc_repo()
        .create(
            &ctx,
            create_deleted_document("Root restore", "trash-root-restore"),
        )
        .await
        .expect("create root restore document");
    let system_admin_document = db
        .doc_repo()
        .create(
            &ctx,
            create_deleted_document("System restore", "trash-system-restore"),
        )
        .await
        .expect("create system restore document");
    let denied_document = db
        .doc_repo()
        .create(
            &ctx,
            create_deleted_document("Denied restore", "trash-denied-restore"),
        )
        .await
        .expect("create denied restore document");
    let system_purge_document = db
        .doc_repo()
        .create(
            &ctx,
            create_deleted_document("System purge", "trash-system-purge"),
        )
        .await
        .expect("create system purge document");
    let member_purge_document = db
        .doc_repo()
        .create(
            &ctx,
            create_deleted_document("Member purge", "trash-member-purge"),
        )
        .await
        .expect("create member purge document");
    let api_key_purge_document = db
        .doc_repo()
        .create(
            &ctx,
            create_deleted_document("API key purge", "trash-api-key-purge"),
        )
        .await
        .expect("create API key purge document");
    let root_document_id = root_document.id.0;
    let system_admin_document_id = system_admin_document.id.0;
    let denied_document_id = denied_document.id.0;
    let system_purge_document_id = system_purge_document.id.0;
    let member_purge_document_id = member_purge_document.id.0;
    let api_key_purge_document_id = api_key_purge_document.id.0;
    let document_service = DocumentService::new(db.conn().clone(), 25);
    for document in [
        root_document,
        system_admin_document,
        denied_document,
        system_purge_document,
        member_purge_document,
        api_key_purge_document,
    ] {
        document_service
            .soft_delete(&ctx, document.id)
            .await
            .expect("delete restore authorization document");
    }

    for client in [&root, &system_admin] {
        assert_eq!(
            trash_response(client, &server, reqwest::Method::GET, "/api/admin/trash")
                .await
                .status(),
            reqwest::StatusCode::OK
        );
    }
    assert_eq!(
        restore(&root, &server, "document", root_document_id)
            .await
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    assert_eq!(
        restore(&system_admin, &server, "document", system_admin_document_id,)
            .await
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );
    for client in [&workspace_admin, &member, &key_client] {
        assert_eq!(
            restore(client, &server, "document", denied_document_id)
                .await
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        purge(
            &workspace_admin,
            &server,
            "document",
            denied_document_id,
            true,
        )
        .await
        .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    assert_eq!(
        purge(
            &system_admin,
            &server,
            "document",
            system_purge_document_id,
            true,
        )
        .await
        .status(),
        reqwest::StatusCode::ACCEPTED
    );
    assert_eq!(
        purge(&member, &server, "document", member_purge_document_id, true)
            .await
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    assert!(document_is_deleted(&db, member_purge_document_id).await);
    assert_eq!(
        count_purge_side_effects(&db, member_purge_document_id).await,
        (0, 0)
    );
    assert_eq!(
        purge(
            &key_client,
            &server,
            "document",
            api_key_purge_document_id,
            true
        )
        .await
        .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    assert!(document_is_deleted(&db, api_key_purge_document_id).await);
    assert_eq!(
        count_purge_side_effects(&db, api_key_purge_document_id).await,
        (0, 0)
    );
    assert_eq!(
        restore(&root, &server, "document", denied_document_id)
            .await
            .status(),
        reqwest::StatusCode::NO_CONTENT
    );

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

#[tokio::test]
async fn confirmed_purge_removes_the_five_kinds_and_reuses_its_pending_operation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace, owner) = support::seed_workspace(&db, "trash-purge-five-kinds").await;
    let ctx = support::ctx(&workspace, &owner);
    let attachments = PgAttachmentRepo {
        conn: db.conn().clone(),
    };

    let project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Purge project".into(),
                slug: "trash-purge-project".into(),
                task_prefix: "TPG".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");
    let project_folder = db
        .folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: Some(project.id),
                parent_folder_id: None,
                name: "project child".into(),
            },
        )
        .await
        .expect("create project folder");
    let project_document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "project child".into(),
                slug: Some("trash-purge-project-child".into()),
                content: "body".into(),
                folder_id: Some(project_folder.id),
                project_id: Some(project.id),
                frontmatter: None,
            },
        )
        .await
        .expect("create project document");
    let folder = db
        .folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "Purge folder".into(),
            },
        )
        .await
        .expect("create folder");
    let folder_document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "folder child".into(),
                slug: Some("trash-purge-folder-child".into()),
                content: "body".into(),
                folder_id: Some(folder.id),
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create folder document");
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Purge document".into(),
                slug: Some("trash-purge-document".into()),
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
                title: "Purge comment parent".into(),
                slug: Some("trash-purge-comment-parent".into()),
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
            "Purge comment".into(),
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
                file_name: "purge.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 1,
                sha256: "f".repeat(64),
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
    let document_service = DocumentService::new(db.conn().clone(), 25);
    document_service
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

    let rejected = purge(&admin, &server, "document", document.id.0, false).await;
    assert_eq!(rejected.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(attachment_is_deleted(&db, attachment.id.0).await);
    assert!(document_is_deleted(&db, document.id.0).await);
    assert_eq!(count_purge_side_effects(&db, document.id.0).await, (0, 0));

    for (kind, target_id) in [
        ("project", project.id.0),
        ("folder", folder.id.0),
        ("document", document.id.0),
        ("comment", comment.id.0),
        ("attachment", attachment.id.0),
    ] {
        let response = purge(&admin, &server, kind, target_id, true).await;
        assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED, "{kind}");
        let body: Value = response.json().await.expect("decode purge status");
        assert_eq!(body["kind"], kind);
        assert_eq!(body["target_id"], target_id.to_string());
        assert_eq!(body["status"], "cleanup_pending");
        assert!(body["operation_id"].is_string());

        let status = trash_response(
            &admin,
            &server,
            reqwest::Method::GET,
            &format!(
                "/api/admin/trash/purges/{}",
                body["operation_id"].as_str().expect("operation id")
            ),
        )
        .await;
        assert_eq!(status.status(), reqwest::StatusCode::OK, "{kind} status");
        assert_eq!(
            status.json::<Value>().await.expect("decode purge status")["status"],
            "cleanup_pending"
        );

        let retry = purge(&admin, &server, kind, target_id, true).await;
        assert_eq!(
            retry.status(),
            reqwest::StatusCode::ACCEPTED,
            "{kind} retry"
        );
        assert_eq!(
            retry.json::<Value>().await.expect("decode retry")["operation_id"],
            body["operation_id"]
        );
    }

    for (table, id) in [
        ("projects", project.id.0),
        ("folders", folder.id.0),
        ("documents", document.id.0),
        ("comments", comment.id.0),
        ("attachments", attachment.id.0),
        ("folders", project_folder.id.0),
        ("documents", project_document.id.0),
        ("documents", folder_document.id.0),
    ] {
        let row = db
            .conn()
            .query_one_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                format!("SELECT id FROM {table} WHERE id = $1"),
                [id.into()],
            ))
            .await
            .expect("query purged row");
        assert!(row.is_none(), "{table} row must be purged");
    }

    let audit_count = PgSecurityAuditRepo::new(db.conn().clone())
        .list_for_workspace(workspace.id, &AuditFilters::default(), None, 100)
        .await
        .expect("list audit")
        .into_iter()
        .filter(|event| event.action == "resource.purge_committed")
        .count();
    assert_eq!(audit_count, 5);

    db.teardown().await;
}

#[tokio::test]
async fn purge_removes_active_draft_dependencies_and_retains_all_closure_digests_after_restart() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (owner_client, workspace, owner) =
        support::login_user_with_workspace(&server, &db, "trash-purge-active-draft").await;
    let ctx = support::ctx(&workspace, &owner);
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Draft purge document".into(),
                slug: Some("trash-purge-active-draft".into()),
                content: "body".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("document slug");
    let draft_url = format!(
        "{}/api/workspaces/{}/documents/{slug}/comment-drafts",
        server.base_url(),
        workspace.slug
    );
    let draft_response = owner_client
        .http_client()
        .post(&draft_url)
        .bearer_auth(owner_client.token().expect("owner token"))
        .header("x-create-token", uuid::Uuid::now_v7().to_string())
        .send()
        .await
        .expect("create draft");
    assert_eq!(draft_response.status(), reqwest::StatusCode::CREATED);
    let draft: Value = draft_response.json().await.expect("decode draft");
    let draft_id = draft["id"].as_str().expect("draft id");
    let draft_attachment = owner_client
        .http_client()
        .post(format!("{draft_url}/{draft_id}/attachments"))
        .bearer_auth(owner_client.token().expect("owner token"))
        .header("x-upload-token", uuid::Uuid::now_v7().to_string())
        .header("x-file-name", "draft.txt")
        .header("content-type", "text/plain")
        .body("draft attachment")
        .send()
        .await
        .expect("upload draft attachment");
    assert_eq!(draft_attachment.status(), reqwest::StatusCode::CREATED);
    let draft_attachment: Value = draft_attachment.json().await.expect("decode attachment");
    let draft_attachment_id = draft_attachment["id"]
        .as_str()
        .expect("attachment id")
        .parse::<uuid::Uuid>()
        .expect("attachment UUID");
    let digest = db
        .conn()
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT sha256 FROM attachments WHERE id = $1",
            [draft_attachment_id.into()],
        ))
        .await
        .expect("query draft attachment digest")
        .expect("draft attachment row")
        .try_get::<String>("", "sha256")
        .expect("decode draft attachment digest");

    db.conn()
        .execute_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE documents SET deleted_at = now() WHERE id = $1",
            [document.id.0.into()],
        ))
        .await
        .expect("tombstone document for purge");

    let purge_response = purge(&admin, &server, "document", document.id.0, true).await;
    assert_eq!(purge_response.status(), reqwest::StatusCode::ACCEPTED);
    let purge_status: Value = purge_response.json().await.expect("decode purge status");
    let operation_id = purge_status["operation_id"]
        .as_str()
        .expect("operation id")
        .parse::<uuid::Uuid>()
        .expect("operation UUID");

    drop(server);
    let restarted_server = support::TestServer::spawn(&db).await;
    let status = trash_response(
        &admin,
        &restarted_server,
        reqwest::Method::GET,
        &format!("/api/admin/trash/purges/{operation_id}"),
    )
    .await;
    assert_eq!(status.status(), reqwest::StatusCode::OK);
    assert_eq!(
        status
            .json::<Value>()
            .await
            .expect("decode restarted status")["status"],
        "cleanup_pending"
    );

    let row = db
        .conn()
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT \
                (SELECT count(*)::bigint FROM comment_attachment_drafts WHERE id = $1) AS drafts, \
                (SELECT count(*)::bigint FROM attachments WHERE id = $2) AS attachments, \
                (SELECT count(*)::bigint FROM comment_attachment_draft_uploads WHERE draft_id = $1) AS uploads, \
                (SELECT count(*)::bigint FROM purge_operation_digests WHERE operation_id = $3 AND digest = $4) AS digests",
            [
                draft_id.parse::<uuid::Uuid>().expect("draft UUID").into(),
                draft_attachment_id.into(),
                operation_id.into(),
                digest.into(),
            ],
        ))
        .await
        .expect("query purged draft closure")
        .expect("purged draft closure row");
    for column in ["drafts", "attachments", "uploads"] {
        assert_eq!(
            row.try_get::<i64>("", column).expect("closure count"),
            0,
            "{column} must be removed by the purge closure"
        );
    }
    assert_eq!(
        row.try_get::<i64>("", "digests").expect("digest count"),
        1,
        "the operation must retain the draft attachment digest"
    );

    db.teardown().await;
}

#[tokio::test]
async fn project_purge_correlates_each_descendant_digest_once_and_removes_every_dependency() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let admin = support::login_root_user(&server, &db).await;
    let (workspace, owner) = support::seed_workspace(&db, "trash-purge-digest-matrix").await;
    let ctx = support::ctx(&workspace, &owner);
    let attachments = PgAttachmentRepo {
        conn: db.conn().clone(),
    };

    let project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Digest matrix project".into(),
                slug: "trash-purge-digest-matrix".into(),
                task_prefix: "TDM".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");
    let parent_folder = db
        .folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: Some(project.id),
                parent_folder_id: None,
                name: "Parent folder".into(),
            },
        )
        .await
        .expect("create parent folder");
    let nested_folder = db
        .folder_repo()
        .create(
            &ctx,
            NewFolder {
                project_id: Some(project.id),
                parent_folder_id: Some(parent_folder.id),
                name: "Nested folder".into(),
            },
        )
        .await
        .expect("create nested folder");
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Digest matrix document".into(),
                slug: Some("trash-purge-digest-matrix-document".into()),
                content: "body".into(),
                folder_id: Some(nested_folder.id),
                project_id: Some(project.id),
                frontmatter: None,
            },
        )
        .await
        .expect("create document");
    let board = db
        .board_repo()
        .create_board(
            &ctx,
            NewBoard {
                project_id: project.id,
                folder_id: Some(nested_folder.id),
                name: "Digest matrix board".into(),
            },
        )
        .await
        .expect("create board");
    let column = db
        .board_repo()
        .add_column(
            &ctx,
            board.id,
            "Todo".into(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");
    let task = db
        .task_repo()
        .create(
            &ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: column.id,
                title: "Digest matrix task".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: Vec::new(),
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("create task");
    let document_comment = CommentService::new(db.conn().clone())
        .create(
            &ctx,
            CommentOwner::Document(document.id),
            "document comment".into(),
        )
        .await
        .expect("create document comment");
    let task_comment = CommentService::new(db.conn().clone())
        .create(&ctx, CommentOwner::Task(task.id), "task comment".into())
        .await
        .expect("create task comment");

    let direct_document_digest = "1".repeat(64);
    let direct_task_digest = "2".repeat(64);
    let document_comment_digest = "3".repeat(64);
    let task_comment_digest = "4".repeat(64);
    let active_draft_digest = "5".repeat(64);
    let finalized_draft_digest = "6".repeat(64);
    let shared_digest = "7".repeat(64);
    let expected_digests = std::collections::BTreeSet::from([
        direct_document_digest.clone(),
        direct_task_digest.clone(),
        document_comment_digest.clone(),
        task_comment_digest.clone(),
        active_draft_digest.clone(),
        finalized_draft_digest.clone(),
        shared_digest.clone(),
    ]);

    for (document_id, task_id, comment_id, file_name, sha256) in [
        (
            Some(document.id),
            None,
            None,
            "direct-document.txt",
            direct_document_digest,
        ),
        (
            None,
            Some(task.id),
            None,
            "direct-task.txt",
            direct_task_digest,
        ),
        (
            None,
            None,
            Some(document_comment.id),
            "document-comment.txt",
            document_comment_digest,
        ),
        (
            None,
            None,
            Some(task_comment.id),
            "task-comment.txt",
            task_comment_digest,
        ),
        (
            Some(document.id),
            None,
            None,
            "shared-document.txt",
            shared_digest.clone(),
        ),
        (
            None,
            None,
            Some(task_comment.id),
            "shared-comment.txt",
            shared_digest,
        ),
    ] {
        attachments
            .record(
                &ctx,
                NewAttachment {
                    document_id,
                    task_id,
                    comment_id,
                    file_name: file_name.into(),
                    content_type: "text/plain".into(),
                    size_bytes: 1,
                    sha256,
                },
            )
            .await
            .expect("create descendant attachment");
    }

    let active_draft_id = uuid::Uuid::now_v7();
    let active_attachment_id = uuid::Uuid::now_v7();
    let finalized_draft_id = uuid::Uuid::now_v7();
    let finalized_attachment_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_drafts \
             (id, workspace_id, document_id, created_by_user_id, create_token, create_digest, state, expires_at) \
             VALUES ('{active_draft_id}', '{}', '{}', '{}', '{active_draft_id}', '\\x{}', 'active', '2999-01-01T00:00:00Z'); \
             INSERT INTO attachments (id, workspace_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
             VALUES ('{active_attachment_id}', '{}', '{active_draft_id}', 'active-draft.txt', 'text/plain', 1, '{active_draft_digest}', '{}'); \
             INSERT INTO comment_attachment_draft_uploads \
             (draft_id, upload_token, original_attachment_id, attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes) \
             VALUES ('{active_draft_id}', 'active-upload', '{active_attachment_id}', '{active_attachment_id}', '\\x{}', '\\x{}', 'active-draft.txt', 'text/plain', 1); \
             INSERT INTO comment_attachment_drafts \
             (id, workspace_id, task_id, created_by_user_id, create_token, create_digest, state, expires_at, finalized_comment_id, final_body_digest, final_request_digest) \
             VALUES ('{finalized_draft_id}', '{}', '{}', '{}', '{finalized_draft_id}', '\\x{}', 'finalized', '2999-01-01T00:00:00Z', '{}', '\\x{}', '\\x{}'); \
             INSERT INTO attachments (id, workspace_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
             VALUES ('{finalized_attachment_id}', '{}', '{finalized_draft_id}', 'finalized-draft.txt', 'text/plain', 1, '{finalized_draft_digest}', '{}'); \
             INSERT INTO comment_attachment_draft_uploads \
             (draft_id, upload_token, original_attachment_id, attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes) \
             VALUES ('{finalized_draft_id}', 'finalized-upload', '{finalized_attachment_id}', '{finalized_attachment_id}', '\\x{}', '\\x{}', 'finalized-draft.txt', 'text/plain', 1)",
            workspace.id.0,
            document.id.0,
            owner.id.0,
            "01".repeat(32),
            workspace.id.0,
            owner.id.0,
            "02".repeat(32),
            "03".repeat(32),
            workspace.id.0,
            task.id.0,
            owner.id.0,
            "04".repeat(32),
            task_comment.id.0,
            "05".repeat(32),
            "06".repeat(32),
            workspace.id.0,
            owner.id.0,
            "07".repeat(32),
            "08".repeat(32),
        ))
        .await
        .expect("seed active and finalized draft dependencies");

    db.project_repo()
        .soft_delete(&ctx, project.id)
        .await
        .expect("delete project");
    let response = purge(&admin, &server, "project", project.id.0, true).await;
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let operation: Value = response.json().await.expect("decode purge operation");
    let operation_id = operation["operation_id"]
        .as_str()
        .expect("operation id")
        .parse::<uuid::Uuid>()
        .expect("operation UUID");

    let digest_rows = db
        .conn()
        .query_all_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT digest, count(*)::bigint AS occurrences FROM purge_operation_digests WHERE operation_id = $1 GROUP BY digest",
            [operation_id.into()],
        ))
        .await
        .expect("load correlated purge digests");
    let actual_digests = digest_rows
        .iter()
        .map(|row| {
            let digest = row.try_get::<String>("", "digest").expect("digest");
            let occurrences = row
                .try_get::<i64>("", "occurrences")
                .expect("digest occurrences");
            assert_eq!(occurrences, 1, "{digest} must be correlated exactly once");
            digest
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(actual_digests, expected_digests);

    let remaining = db
        .conn()
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT \
                (SELECT count(*)::bigint FROM projects WHERE workspace_id = $1) AS projects, \
                (SELECT count(*)::bigint FROM folders WHERE workspace_id = $1) AS folders, \
                (SELECT count(*)::bigint FROM documents WHERE workspace_id = $1) AS documents, \
                (SELECT count(*)::bigint FROM boards WHERE workspace_id = $1) AS boards, \
                (SELECT count(*)::bigint FROM tasks WHERE workspace_id = $1) AS tasks, \
                (SELECT count(*)::bigint FROM comments WHERE workspace_id = $1) AS comments, \
                (SELECT count(*)::bigint FROM attachments WHERE workspace_id = $1) AS attachments, \
                (SELECT count(*)::bigint FROM comment_attachment_drafts WHERE workspace_id = $1) AS drafts, \
                (SELECT count(*)::bigint FROM comment_attachment_draft_uploads WHERE draft_id IN ($2, $3)) AS uploads",
            [workspace.id.0.into(), active_draft_id.into(), finalized_draft_id.into()],
        ))
        .await
        .expect("load remaining closure rows")
        .expect("closure count row");
    for column in [
        "projects",
        "folders",
        "documents",
        "boards",
        "tasks",
        "comments",
        "attachments",
        "drafts",
        "uploads",
    ] {
        assert_eq!(
            remaining.try_get::<i64>("", column).expect("closure count"),
            0,
            "{column} must be removed by the project purge closure"
        );
    }

    db.teardown().await;
}
