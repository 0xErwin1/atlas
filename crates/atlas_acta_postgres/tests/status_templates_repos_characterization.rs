#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization tests for `PgStatusTemplateRepo`/`PgPlatformStatusTemplateRepo`'s
//! current query shapes, ported from `atlas_server::persistence::repos::{status_templates,
//! platform_status_templates}` before that code moves into this crate
//! (orchestrator-mandated addition to S4 PR9, per the PR8 apply-progress open gap).
//! Must keep passing unmodified once the move lands.
//!
//! Both repos have no Custos dependency: `workspace_status_templates`/
//! `platform_status_templates` carry no `custos.*` foreign key, and neither
//! repo composes `append_resource_deleted_in`/`PgOutboxRepo`. `remap_anchors`
//! (a private helper `PgPlatformStatusTemplateRepo` reuses from
//! `PgStatusTemplateRepo`'s module) stays `pub(crate)` since both repos land
//! in the same crate.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::entities::status_templates::{NewStatusTemplate, StatusTemplatePatch};
use atlas_acta::ids::WorkspaceId;
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta::ports::status_templates::{PlatformStatusTemplateRepo, StatusTemplateRepo};
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_acta_postgres::repos::platform_status_templates::PgPlatformStatusTemplateRepo;
use atlas_acta_postgres::repos::status_templates::{
    PgStatusTemplateRepo, list_templates_for_workspace,
};
use atlas_core::principal::UserId;
use atlas_custos::entities::identity::NewUser;
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use uuid::Uuid;

async fn seed_workspace(db: &TestDb, slug: &str) -> (WorkspaceId, UserId) {
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

    (workspace_id, user.id)
}

#[tokio::test]
async fn status_template_repo_create_list_patch_move_and_soft_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let (workspace_id, user_id) = seed_workspace(&db, "status-template-repo").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));
    let repo = PgStatusTemplateRepo::new(db.conn().clone());

    let first = repo
        .create(
            &ctx,
            NewStatusTemplate {
                name: "Todo".to_string(),
                color: Some("#fff".to_string()),
                position_key: "a0".to_string(),
            },
        )
        .await
        .expect("create must succeed");
    let second = repo
        .create(
            &ctx,
            NewStatusTemplate {
                name: "Done".to_string(),
                color: None,
                position_key: "a1".to_string(),
            },
        )
        .await
        .expect("create must succeed");

    let listed = repo.list(&ctx).await.expect("list must succeed");
    assert_eq!(listed.len(), 2);

    let patched = repo
        .patch(
            &ctx,
            first.id,
            StatusTemplatePatch {
                name: Some("In Progress".to_string()),
                color: None,
            },
        )
        .await
        .expect("patch must succeed");
    assert_eq!(patched.name, "In Progress");

    repo.move_template(
        &ctx,
        second.id,
        atlas_acta::entities::boards_tasks::PositionBetween {
            before: None,
            after: Some(patched.position_key.clone()),
        },
    )
    .await
    .expect("move_template must succeed");

    repo.soft_delete(&ctx, second.id)
        .await
        .expect("soft_delete must succeed");

    let after_delete = repo.list(&ctx).await.expect("list must succeed");
    assert_eq!(after_delete.len(), 1);
    assert_eq!(after_delete[0].id, patched.id);

    let via_free_fn = list_templates_for_workspace(db.conn(), workspace_id.0)
        .await
        .expect("list_templates_for_workspace must succeed");
    assert_eq!(via_free_fn.len(), 1);
}

#[tokio::test]
async fn platform_status_template_repo_create_list_patch_and_soft_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let repo = PgPlatformStatusTemplateRepo::new(db.conn().clone());

    let created = repo
        .create(NewStatusTemplate {
            name: "Backlog".to_string(),
            color: Some("#123".to_string()),
            position_key: "a0".to_string(),
        })
        .await
        .expect("create must succeed");

    let listed = repo.list().await.expect("list must succeed");
    assert!(listed.iter().any(|t| t.id == created.id));

    let patched = repo
        .patch(
            created.id,
            StatusTemplatePatch {
                name: Some("Icebox".to_string()),
                color: None,
            },
        )
        .await
        .expect("patch must succeed");
    assert_eq!(patched.name, "Icebox");

    repo.soft_delete(patched.id)
        .await
        .expect("soft_delete must succeed");

    let after_delete = repo.list().await.expect("list must succeed");
    assert!(!after_delete.iter().any(|t| t.id == patched.id));
}
