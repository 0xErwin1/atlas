#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization tests for `PgDocumentRepo`/`PgDocumentLinkRepo`'s current
//! query shapes, ported from `atlas_server::persistence::repos::documents`
//! before that code moves into this crate (S4 PR7, T7.1). These assertions
//! must keep passing unmodified once the move lands (T7.4) and again once
//! PR12's `SET SCHEMA acta` batch moves the documents-group tables (T12.8).
//!
//! `PgAttachmentRepo`/`PgAttachmentWriteIntentRepo`/`PgAttachmentLifecycle`
//! are NOT covered here: they stay in `atlas_server` because their methods
//! compose a Custos security-audit append with Acta context (see the module
//! doc comment on `atlas_acta_postgres::repos::documents`).
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::documents::{ExtractedLink, NewDocument};
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::ports::documents::{DocumentLinkRepo, DocumentRepo};
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta_postgres::repos::documents::{PgDocumentLinkRepo, PgDocumentRepo};
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_core::principal::UserId;
use atlas_custos::entities::identity::NewUser;
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use uuid::Uuid;

async fn seed_workspace_and_user(db: &TestDb, slug: &str) -> (WorkspaceId, UserId) {
    let workspace_repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };
    let workspace_id = WorkspaceId(Uuid::now_v7());
    workspace_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: slug.to_string(),
            slug: slug.to_string(),
        })
        .await
        .expect("workspace must be created");

    let user_repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let user = user_repo
        .create(NewUser {
            username: slug.to_string(),
            display_name: slug.to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed user must be created");

    (workspace_id, user.id)
}

#[tokio::test]
async fn document_repo_create_get_rename_update_content_and_soft_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let (workspace_id, user_id) = seed_workspace_and_user(&db, "document-repo-crud").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));
    let repo = PgDocumentRepo {
        conn: db.conn().clone(),
        anchor_interval: 20,
    };

    let created = repo
        .create(
            &ctx,
            NewDocument {
                title: "Design Doc".to_string(),
                slug: Some("design-doc".to_string()),
                content: "# Hello".to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("document must be created");
    assert_eq!(created.title, "Design Doc");

    let fetched = repo
        .get(&ctx, created.id)
        .await
        .expect("get must not error")
        .expect("document must exist");
    assert_eq!(fetched.content, "# Hello");

    let by_slug = repo
        .find_by_slug(&ctx, "design-doc")
        .await
        .expect("find_by_slug must not error")
        .expect("document must resolve by slug");
    assert_eq!(by_slug.id, created.id);

    let renamed = repo
        .rename(&ctx, created.id, "Design Doc v2".to_string())
        .await
        .expect("rename must succeed");
    assert_eq!(renamed.title, "Design Doc v2");

    let updated = repo
        .update_content(&ctx, created.id, renamed.current_revision_id, "# Updated")
        .await
        .expect("update_content must succeed");
    assert_eq!(updated.content, "# Updated");
    assert_eq!(updated.current_revision_seq, 2);

    let history = repo
        .history(&ctx, created.id)
        .await
        .expect("history must not error");
    assert_eq!(history.len(), 2);

    let content_at_first = repo
        .content_at(&ctx, created.id, 1)
        .await
        .expect("content_at must reconstruct the first revision");
    assert_eq!(content_at_first, "# Hello");

    repo.soft_delete(&ctx, created.id)
        .await
        .expect("soft_delete must succeed");
    assert!(
        repo.get(&ctx, created.id)
            .await
            .expect("get must not error")
            .is_none(),
        "a soft-deleted document must not resolve by id"
    );

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn document_link_repo_replaces_and_reports_backlinks() {
    let db = TestDb::create().await.expect("test db must be created");
    let (workspace_id, user_id) = seed_workspace_and_user(&db, "document-link-repo").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));
    let document_repo = PgDocumentRepo {
        conn: db.conn().clone(),
        anchor_interval: 20,
    };
    let link_repo = PgDocumentLinkRepo {
        conn: db.conn().clone(),
    };

    let source = document_repo
        .create(
            &ctx,
            NewDocument {
                title: "Source".to_string(),
                slug: Some("link-source".to_string()),
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("source document must be created");
    let target = document_repo
        .create(
            &ctx,
            NewDocument {
                title: "Target".to_string(),
                slug: Some("link-target".to_string()),
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("target document must be created");

    link_repo
        .replace_for_source(
            &ctx,
            source.id,
            vec![ExtractedLink {
                target_document_id: Some(target.id),
                target_task_id: None,
                target_attachment_id: None,
                target_title: "Target".to_string(),
            }],
        )
        .await
        .expect("replace_for_source must succeed");

    let backlinks = link_repo
        .backlinks(&ctx, target.id)
        .await
        .expect("backlinks must not error");
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].source_document_id, Some(source.id));

    // Replacing again with an empty set must clear the previous link set
    // rather than accumulate rows (the `delete_many` + reinsert shape).
    link_repo
        .replace_for_source(&ctx, source.id, Vec::new())
        .await
        .expect("replace_for_source with no links must succeed");
    assert!(
        link_repo
            .backlinks(&ctx, target.id)
            .await
            .expect("backlinks must not error")
            .is_empty()
    );

    let titles = PgDocumentLinkRepo::list_titles_for_task_source_in(
        db.conn(),
        &ctx,
        atlas_acta::ids::TaskId(Uuid::now_v7()),
    )
    .await
    .expect("list_titles_for_task_source_in must not error for an unrelated task id");
    assert!(titles.is_empty());

    db.teardown().await.expect("teardown must succeed");
}
