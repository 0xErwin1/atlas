#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization tests for `PgWorkspaceRepo`/`PgMembershipRepo`'s current
//! query shapes (CRUD, membership listing), ported from
//! `atlas_server::persistence::repos::identity` before that code moves into
//! this crate (S4 PR6, T6.1). These assertions must keep passing unmodified
//! once the move lands (T6.4) and again once PR11's `SET SCHEMA acta` batch
//! moves `workspaces`/`workspace_memberships` (T11.10) — this file is the
//! byte-frozen baseline both PRs re-run against.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`
//! (see `atlas_test_db`). `atlas_custos_postgres`/`atlas_custos` are
//! dev-dependencies only, used here to seed a `custos.users` row that
//! `workspace_memberships` and `list_for_api_key` require by foreign key —
//! this crate's production code never depends on either.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::identity::MemberRole;
use atlas_acta::ids::WorkspaceId;
use atlas_acta_postgres::repos::identity::{
    MembershipRepo, NewWorkspace, PgMembershipRepo, PgWorkspaceRepo, WorkspaceRepo,
};
use atlas_core::principal::UserId;
use atlas_custos::entities::identity::NewUser;
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use uuid::Uuid;

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
async fn workspace_repo_create_find_rename_and_slug_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };

    let workspace_id = WorkspaceId(Uuid::now_v7());
    let created = repo
        .create(NewWorkspace {
            id: workspace_id,
            name: "Atlas".to_string(),
            slug: "atlas".to_string(),
        })
        .await
        .expect("workspace must be created");
    assert_eq!(created.id, workspace_id);
    assert_eq!(created.name, "Atlas");
    assert_eq!(created.slug, "atlas");

    let by_id = repo
        .find_by_id(workspace_id)
        .await
        .expect("find_by_id must not error")
        .expect("workspace must exist");
    assert_eq!(by_id.slug, "atlas");

    let by_slug = repo
        .find_by_slug("atlas")
        .await
        .expect("find_by_slug must not error")
        .expect("workspace must exist by slug");
    assert_eq!(by_slug.id, workspace_id);

    let renamed = repo
        .rename(workspace_id, "Atlas Renamed".to_string())
        .await
        .expect("rename must succeed");
    assert_eq!(renamed.name, "Atlas Renamed");

    let resloged = repo
        .set_slug(workspace_id, "atlas-2".to_string())
        .await
        .expect("set_slug must succeed");
    assert_eq!(resloged.slug, "atlas-2");

    let slugs = repo.list_slugs().await.expect("list_slugs must not error");
    assert!(slugs.contains(&"atlas-2".to_string()));

    let all = repo.list_all().await.expect("list_all must not error");
    assert!(all.iter().any(|w| w.id == workspace_id));

    repo.soft_delete(workspace_id)
        .await
        .expect("soft_delete must succeed");
    assert!(
        repo.find_by_id(workspace_id)
            .await
            .expect("find_by_id must not error")
            .is_none(),
        "a soft-deleted workspace must not resolve by id"
    );

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn workspace_repo_lists_workspaces_for_a_user_and_by_api_key_grant() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };
    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };

    let user_id = seed_user(&db, "workspace-lister").await;
    let workspace_id = WorkspaceId(Uuid::now_v7());
    workspace_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: "Membership Test".to_string(),
            slug: "membership-test".to_string(),
        })
        .await
        .expect("workspace must be created");

    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));
    membership_repo
        .add(&ctx, user_id, MemberRole::Owner)
        .await
        .expect("membership must be added");

    let for_user = workspace_repo
        .list_for_user(user_id)
        .await
        .expect("list_for_user must not error");
    assert_eq!(for_user.len(), 1);
    assert_eq!(for_user[0].id, workspace_id);

    let memberships = workspace_repo
        .list_memberships_for_user(user_id)
        .await
        .expect("list_memberships_for_user must not error");
    assert_eq!(memberships.len(), 1);
    assert!(matches!(memberships[0].1, MemberRole::Owner));

    // `list_for_api_key` reads `custos.permission_grants` by raw SQL (D6):
    // with no grant row for this key, it must return no workspaces rather
    // than erroring, proving the query shape survives the move unmodified.
    let for_unrelated_key = workspace_repo
        .list_for_api_key(atlas_core::principal::ApiKeyId(Uuid::now_v7()))
        .await
        .expect("list_for_api_key must not error");
    assert!(for_unrelated_key.is_empty());

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn membership_repo_add_find_list_update_role_and_remove_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };
    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };

    let user_id = seed_user(&db, "membership-crud").await;
    let workspace_id = WorkspaceId(Uuid::now_v7());
    workspace_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: "Membership CRUD".to_string(),
            slug: "membership-crud".to_string(),
        })
        .await
        .expect("workspace must be created");

    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));

    let added = membership_repo
        .add(&ctx, user_id, MemberRole::Member)
        .await
        .expect("membership must be added");
    assert!(matches!(added.role, MemberRole::Member));

    let found = membership_repo
        .find(&ctx, user_id)
        .await
        .expect("find must not error")
        .expect("membership must exist");
    assert_eq!(found.user_id, user_id);

    let listed = membership_repo
        .list(&ctx)
        .await
        .expect("list must not error");
    assert_eq!(listed.len(), 1);

    let updated = membership_repo
        .update_role(&ctx, user_id, MemberRole::Admin)
        .await
        .expect("update_role must succeed");
    assert!(matches!(updated.role, MemberRole::Admin));

    membership_repo
        .remove(&ctx, user_id)
        .await
        .expect("remove must succeed");
    assert!(
        membership_repo
            .find(&ctx, user_id)
            .await
            .expect("find must not error")
            .is_none(),
        "membership must be gone after remove"
    );

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn membership_remove_is_blocked_by_a_retained_comment_draft() {
    let db = TestDb::create().await.expect("TestDb::create");
    let user_id = seed_user(&db, "draft-guard-user").await;

    let workspace_repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };
    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };

    let workspace_id = WorkspaceId(Uuid::now_v7());
    workspace_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: "Draft guard workspace".to_string(),
            slug: "draft-guard-ws".to_string(),
        })
        .await
        .expect("workspace must be created");
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));

    membership_repo
        .add(&ctx, user_id, MemberRole::Member)
        .await
        .expect("add membership");

    let document_id = Uuid::now_v7();
    sea_orm::ConnectionTrait::execute_unprepared(
        db.conn(),
        &format!(
            "INSERT INTO documents (id, workspace_id, title, created_by_user_id) \
             VALUES ('{document_id}', '{}', 'Draft guard doc', '{}')",
            workspace_id.0, user_id.0
        ),
    )
    .await
    .expect("seed parent document");
    sea_orm::ConnectionTrait::execute_unprepared(
        db.conn(),
        &format!(
            "INSERT INTO comment_attachment_drafts \
             (id, workspace_id, document_id, created_by_user_id, create_token, \
              create_digest, state, expires_at, created_at, updated_at) \
             VALUES ('{}', '{}', '{document_id}', '{}', 'draft-guard-token', \
                     decode(repeat('00', 32), 'hex'), 'active', \
                     now() + interval '1 hour', now(), now())",
            Uuid::now_v7(),
            workspace_id.0,
            user_id.0
        ),
    )
    .await
    .expect("seed retained draft");

    let blocked = membership_repo.remove(&ctx, user_id).await;
    assert!(
        matches!(
            blocked,
            Err(atlas_core::error::DomainError::CommentDraftConflict { .. })
        ),
        "the relocated guard must still block removal while a draft is retained"
    );

    sea_orm::ConnectionTrait::execute_unprepared(
        db.conn(),
        &format!(
            "DELETE FROM comment_attachment_drafts WHERE workspace_id = '{}'",
            workspace_id.0
        ),
    )
    .await
    .expect("clear draft");

    membership_repo
        .remove(&ctx, user_id)
        .await
        .expect("remove must succeed once no draft is retained");
}
