#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization test for `PgCommentRepo`'s current query shapes, ported
//! from `atlas_server::persistence::repos::comments` before that code moves
//! into this crate (S4 PR7, T7.1). Must keep passing unmodified once the
//! move lands (T7.4).
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::comments::{CommentOwner, NewComment};
use atlas_acta::entities::documents::NewDocument;
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::ports::comments::CommentRepo;
use atlas_acta::ports::documents::DocumentRepo;
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta_postgres::repos::comments::PgCommentRepo;
use atlas_acta_postgres::repos::documents::PgDocumentRepo;
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_custos::entities::identity::NewUser;
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use uuid::Uuid;

#[tokio::test]
async fn comment_repo_create_list_and_soft_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");

    let workspace_repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };
    let document_repo = PgDocumentRepo {
        conn: db.conn().clone(),
        anchor_interval: 20,
    };
    let comment_repo = PgCommentRepo::new(db.conn().clone());

    let workspace_id = WorkspaceId(Uuid::now_v7());
    workspace_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: "Comment repo workspace".to_string(),
            slug: "comment-repo-workspace".to_string(),
        })
        .await
        .expect("workspace must be created");

    let user_repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let user = user_repo
        .create(NewUser {
            username: "comment-repo-author".to_string(),
            display_name: "comment-repo-author".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed user must be created");

    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user.id.0)));

    let document = document_repo
        .create(
            &ctx,
            NewDocument {
                title: "Commented doc".to_string(),
                slug: Some("commented-doc".to_string()),
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("document must be created");

    let owner = CommentOwner::Document(document.id);

    let created = comment_repo
        .create(
            &ctx,
            NewComment {
                owner,
                body: "First comment".to_string(),
            },
        )
        .await
        .expect("comment must be created");
    assert_eq!(created.body, "First comment");

    let fetched = comment_repo
        .get_for_owner(&ctx, owner, created.id)
        .await
        .expect("get_for_owner must succeed");
    assert_eq!(fetched.id, created.id);

    let listed = comment_repo
        .list_for_owner(&ctx, owner, None, 10)
        .await
        .expect("list_for_owner must not error");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);

    comment_repo
        .soft_delete(&ctx, owner, created.id)
        .await
        .expect("soft_delete must succeed");

    let after_delete = comment_repo.get_for_owner(&ctx, owner, created.id).await;
    assert!(
        matches!(
            after_delete,
            Err(atlas_core::error::DomainError::NotFound { .. })
        ),
        "a soft-deleted comment must resolve as not-found for its owner"
    );

    db.teardown().await.expect("teardown must succeed");
}
