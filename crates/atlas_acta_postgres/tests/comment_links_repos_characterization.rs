#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization test for `PgCommentLinkRepo`'s current query shapes,
//! ported from `atlas_server::persistence::repos::comment_links` before that
//! code moves into this crate (S4 PR8, T8.1). Must keep passing unmodified
//! once the move lands (T8.4).
//!
//! `CommentMutationFault` also relocates here from
//! `atlas_server::services::comment_service` in this PR (it is threaded into
//! `replace_for_comment_with_fault_in` as a parameter, so it must live in the
//! same crate as that method) — this test exercises the fault-injection path
//! directly to prove the relocated type still round-trips through the same
//! equality checks.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::comments::{CommentOwner, NewComment};
use atlas_acta::entities::documents::NewDocument;
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::ports::comments::{CommentLinkRepo, CommentRepo};
use atlas_acta::ports::documents::DocumentRepo;
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta_postgres::repos::comment_links::{CommentMutationFault, PgCommentLinkRepo};
use atlas_acta_postgres::repos::comments::PgCommentRepo;
use atlas_acta_postgres::repos::documents::PgDocumentRepo;
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_core::principal::UserId;
use atlas_custos::entities::identity::NewUser;
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use uuid::Uuid;

async fn seed_workspace(db: &TestDb, slug: &str) -> WorkspaceId {
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
    workspace_id
}

// `documents.created_by_user_id` is a foreign key into `custos.users`: seed a
// real user rather than a random UUID.
async fn seed_user(db: &TestDb, username: &str) -> UserId {
    let repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let user = repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed user must be created");
    user.id
}

#[tokio::test]
async fn comment_link_repo_replace_and_report_backlinks_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "comment-link-test").await;
    let user_id = seed_user(&db, "comment-link-doc-owner").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));

    let document_repo = PgDocumentRepo {
        conn: db.conn().clone(),
        anchor_interval: 20,
    };
    let source_doc = document_repo
        .create(
            &ctx,
            NewDocument {
                title: "Source doc".to_string(),
                slug: None,
                content: "content".to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("source document must be created");
    let target_doc = document_repo
        .create(
            &ctx,
            NewDocument {
                title: "Target doc".to_string(),
                slug: None,
                content: "content".to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("target document must be created");

    let comment_repo = PgCommentRepo {
        conn: db.conn().clone(),
    };
    let comment = comment_repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Document(source_doc.id),
                body: "See [[Target doc]]".to_string(),
            },
        )
        .await
        .expect("comment must be created");

    let link_repo = PgCommentLinkRepo::new(db.conn().clone());
    link_repo
        .replace_for_comment(
            &ctx,
            comment.id,
            vec![atlas_acta::entities::comments::CommentLinkTarget::Document(
                target_doc.id,
            )],
        )
        .await
        .expect("replace_for_comment must succeed");

    let backlinks = link_repo
        .backlinks_for_target(
            &ctx,
            atlas_acta::entities::comments::CommentLinkTarget::Document(target_doc.id),
        )
        .await
        .expect("backlinks_for_target must not error");
    assert_eq!(backlinks.len(), 1);
    assert_eq!(backlinks[0].comment_id, comment.id);

    link_repo
        .remove_for_comment(&ctx, comment.id)
        .await
        .expect("remove_for_comment must succeed");
    let backlinks_after_remove = link_repo
        .backlinks_for_target(
            &ctx,
            atlas_acta::entities::comments::CommentLinkTarget::Document(target_doc.id),
        )
        .await
        .expect("backlinks_for_target must not error");
    assert!(backlinks_after_remove.is_empty());

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn comment_link_repo_fault_injection_seam_still_type_checks_after_relocation() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "comment-link-fault-test").await;
    let user_id = seed_user(&db, "comment-link-doc-owner").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));

    let document_repo = PgDocumentRepo {
        conn: db.conn().clone(),
        anchor_interval: 20,
    };
    let doc = document_repo
        .create(
            &ctx,
            NewDocument {
                title: "Fault doc".to_string(),
                slug: None,
                content: "content".to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("document must be created");

    let comment_repo = PgCommentRepo {
        conn: db.conn().clone(),
    };
    let comment = comment_repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Document(doc.id),
                body: "no links".to_string(),
            },
        )
        .await
        .expect("comment must be created");

    // Proves `CommentMutationFault` still round-trips through the equality
    // check inside `replace_for_comment_with_fault_in` after relocating the
    // enum from `atlas_server::services::comment_service` into this crate.
    let result = PgCommentLinkRepo::replace_for_comment_with_fault_in(
        db.conn(),
        &ctx,
        comment.id,
        Vec::new(),
        Some(CommentMutationFault::AfterGraphReplace),
    )
    .await;
    assert!(
        result.is_err(),
        "the injected fault must surface as an error"
    );

    db.teardown().await.expect("teardown must succeed");
}
