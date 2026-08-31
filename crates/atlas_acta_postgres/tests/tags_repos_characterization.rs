#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization test for `PgTagRepo`'s current query shapes, ported from
//! `atlas_server::persistence::repos::tags` before that code moves into this
//! crate (S4 PR8, T8.1). Must keep passing unmodified once the move lands
//! (T8.4).
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::entities::tags::NewTag;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta::ports::tags::TagRepo;
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_acta_postgres::repos::tags::PgTagRepo;
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

// `tags.created_by_user_id` is a foreign key into `custos.users`: seed a real
// user rather than a random UUID.
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
async fn tag_repo_create_find_update_and_soft_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "tag-repo-test").await;
    let user_id = seed_user(&db, "tag-repo-owner").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));
    let repo = PgTagRepo {
        conn: db.conn().clone(),
    };

    let created = repo
        .create(
            &ctx,
            NewTag {
                name: "urgent".to_string(),
            },
        )
        .await
        .expect("tag must be created");
    assert_eq!(created.name, "urgent");
    assert!(created.color.is_none());

    let found = repo
        .find_by_name(&ctx, "urgent")
        .await
        .expect("find_by_name must not error")
        .expect("tag must exist");
    assert_eq!(found.id, created.id);

    let listed = repo.list(&ctx).await.expect("list must not error");
    assert_eq!(listed.len(), 1);

    let updated = repo
        .update(
            &ctx,
            created.id,
            Some("urgent-renamed".to_string()),
            Some("#ff0000".to_string()),
        )
        .await
        .expect("update must succeed");
    assert_eq!(updated.name, "urgent-renamed");
    assert_eq!(updated.color.as_deref(), Some("#ff0000"));

    repo.soft_delete(&ctx, created.id)
        .await
        .expect("soft_delete must succeed");
    let listed_after_delete = repo.list(&ctx).await.expect("list must not error");
    assert!(listed_after_delete.is_empty());

    db.teardown().await.expect("teardown must succeed");
}
