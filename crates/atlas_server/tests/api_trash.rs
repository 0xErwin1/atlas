#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::unwrap_used)]

mod support;

use atlas_domain::{
    entities::{
        comments::CommentOwner,
        documents::{NewAttachment, NewDocument},
        workspace_core::{NewFolder, NewProject},
    },
    permissions::{Visibility, VisibilityRole},
    ports::{
        documents::{AttachmentRepo, DocumentRepo},
        workspace_core::{FolderRepo, ProjectRepo},
    },
};
use atlas_server::routes::registry::ROUTE_REGISTRY;
use atlas_server::{
    persistence::repos::PgAttachmentRepo,
    services::{CommentService, DocumentService},
};
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
