#![allow(clippy::expect_used, clippy::unwrap_used)]

mod support;

use atlas_domain::{
    entities::{
        documents::{NewAttachment, NewDocument},
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
use atlas_server::{
    persistence::repos::{PgAttachmentRepo, PgSecurityAuditRepo},
    services::DocumentService,
};
use serde_json::json;

#[tokio::test]
async fn delete_audit_idempotency_records_safe_lifecycle_events() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "lifecycle-audit").await;
    let ctx = support::ctx(&workspace, &user);

    let project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Lifecycle project".into(),
                slug: "lifecycle-audit-project".into(),
                task_prefix: "LAUD".into(),
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
                name: "Lifecycle folder".into(),
            },
        )
        .await
        .expect("create folder");
    let document = db
        .doc_repo()
        .create(
            &ctx,
            NewDocument {
                title: "Lifecycle document".into(),
                slug: Some("lifecycle-audit-document".into()),
                content: "private document content".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");
    let attachments = PgAttachmentRepo {
        conn: db.conn().clone(),
    };
    let attachment = attachments
        .record(
            &ctx,
            NewAttachment {
                document_id: Some(document.id),
                task_id: None,
                comment_id: None,
                file_name: "private.pdf".into(),
                content_type: "application/pdf".into(),
                size_bytes: 17,
                sha256: "a".repeat(64),
            },
        )
        .await
        .expect("create attachment");

    attachments
        .soft_delete(&ctx, attachment.id)
        .await
        .expect("delete attachment");
    DocumentService::new(db.conn().clone(), 25)
        .soft_delete(&ctx, document.id)
        .await
        .expect("delete document");
    db.folder_repo()
        .soft_delete(&ctx, folder.id)
        .await
        .expect("delete folder");
    db.project_repo()
        .soft_delete(&ctx, project.id)
        .await
        .expect("delete project");

    let audit = PgSecurityAuditRepo::new(db.conn().clone());
    let rows = audit
        .list_for_workspace(workspace.id, &AuditFilters::default(), None, 100)
        .await
        .expect("list audit rows");
    let deleted: Vec<_> = rows
        .iter()
        .filter(|row| row.action == "resource.deleted")
        .collect();

    assert_eq!(
        deleted.len(),
        4,
        "one lifecycle row per completed tombstone"
    );

    for (kind, target_id) in [
        ("attachment", attachment.id.0),
        ("document", document.id.0),
        ("folder", folder.id.0),
        ("project", project.id.0),
    ] {
        let row = deleted
            .iter()
            .find(|row| row.target_type == kind && row.target_id == Some(target_id))
            .expect("lifecycle row for deleted resource");

        assert_eq!(row.workspace_id, Some(workspace.id));
        assert_eq!(row.actor, ctx.actor);
        assert_eq!(row.metadata, json!({ "kind": kind, "outcome": "deleted" }));
    }

    assert!(attachments.soft_delete(&ctx, attachment.id).await.is_err());
    assert!(
        DocumentService::new(db.conn().clone(), 25)
            .soft_delete(&ctx, document.id)
            .await
            .is_err()
    );
    assert!(db.folder_repo().soft_delete(&ctx, folder.id).await.is_err());
    assert!(
        db.project_repo()
            .soft_delete(&ctx, project.id)
            .await
            .is_err()
    );

    let retried_rows = audit
        .list_for_workspace(workspace.id, &AuditFilters::default(), None, 100)
        .await
        .expect("list audit rows after retries");
    assert_eq!(
        retried_rows
            .iter()
            .filter(|row| row.action == "resource.deleted")
            .count(),
        4,
        "already-deleted retries must not append lifecycle rows"
    );

    db.teardown().await;
}
