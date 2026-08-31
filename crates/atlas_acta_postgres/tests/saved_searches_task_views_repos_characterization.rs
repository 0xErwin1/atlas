#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization test for `PgSavedSearchRepo`/`PgTaskViewRepo`'s current
//! query shapes, ported from `atlas_server::persistence::repos::{saved_searches,
//! task_views}` before that code moves into this crate (S4 PR8, T8.1). Must
//! keep passing unmodified once the move lands (T8.4).
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::entities::saved_searches::NewSavedSearch;
use atlas_acta::entities::task_views::{NewTaskView, TaskViewFilters};
use atlas_acta::ids::WorkspaceId;
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta::ports::saved_searches::SavedSearchRepo;
use atlas_acta::ports::task_views::TaskViewRepo;
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_acta_postgres::repos::saved_searches::PgSavedSearchRepo;
use atlas_acta_postgres::repos::task_views::PgTaskViewRepo;
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

// `saved_searches.owner_user_id`/`task_views.owner_user_id` are foreign keys
// into `custos.users`: seed a real user rather than a random UUID.
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
async fn saved_search_repo_create_find_rename_and_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "saved-search-test").await;
    let user_id = seed_user(&db, "saved-search-owner").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));
    let repo = PgSavedSearchRepo {
        conn: db.conn().clone(),
    };

    let created = repo
        .create(
            &ctx,
            NewSavedSearch {
                name: "My search".to_string(),
                query: "status:open".to_string(),
            },
        )
        .await
        .expect("saved search must be created");
    assert_eq!(created.name, "My search");

    let found = repo
        .find(&ctx, created.id)
        .await
        .expect("find must not error")
        .expect("saved search must exist");
    assert_eq!(found.query, "status:open");

    let listed = repo
        .list_for_owner(&ctx)
        .await
        .expect("list_for_owner must not error");
    assert_eq!(listed.len(), 1);

    let renamed = repo
        .rename(&ctx, created.id, "Renamed search".to_string())
        .await
        .expect("rename must succeed");
    assert_eq!(renamed.name, "Renamed search");

    repo.delete(&ctx, created.id)
        .await
        .expect("delete must succeed");
    let listed_after_delete = repo
        .list_for_owner(&ctx)
        .await
        .expect("list_for_owner must not error");
    assert!(listed_after_delete.is_empty());

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn task_view_repo_create_find_update_and_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "task-view-test").await;
    let user_id = seed_user(&db, "task-view-owner").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));
    let repo = PgTaskViewRepo {
        conn: db.conn().clone(),
    };

    let created = repo
        .create(
            &ctx,
            NewTaskView {
                name: "My view".to_string(),
                filters: TaskViewFilters::default(),
            },
        )
        .await
        .expect("task view must be created");
    assert_eq!(created.name, "My view");

    let found = repo
        .find(&ctx, created.id)
        .await
        .expect("find must not error")
        .expect("task view must exist");
    assert_eq!(found.id, created.id);

    let listed = repo
        .list_for_owner(&ctx)
        .await
        .expect("list_for_owner must not error");
    assert_eq!(listed.len(), 1);

    let updated = repo
        .update(
            &ctx,
            created.id,
            "Renamed view".to_string(),
            TaskViewFilters::default(),
        )
        .await
        .expect("update must succeed");
    assert_eq!(updated.name, "Renamed view");

    repo.delete(&ctx, created.id)
        .await
        .expect("delete must succeed");
    let listed_after_delete = repo
        .list_for_owner(&ctx)
        .await
        .expect("list_for_owner must not error");
    assert!(listed_after_delete.is_empty());

    db.teardown().await.expect("teardown must succeed");
}
