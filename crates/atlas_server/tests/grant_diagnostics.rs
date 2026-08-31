#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! T6.8 — `count_orphaned_grants`, the orphaned-grant doctor finding query.
//! Ships as a repo method + test only; no `doctor` surface is built in S3.

mod support;

use atlas_acta::entities::workspace_core::NewProject;
use atlas_acta::permissions::ResourceRef;
use atlas_acta::permissions::Visibility;
use atlas_acta::permissions::VisibilityRole;
use atlas_acta::permissions::resource_ref_codec;
use atlas_acta::ports::workspace_core::ProjectRepo;
use atlas_custos_postgres::repos::permissions::PgPermissionGrantRepo;
use atlas_server::authz::ResourceRole;
use atlas_server::authz::policy::NewPermissionGrant;
use atlas_server::persistence::repos::{PermissionGrantRepo, count_orphaned_grants};
use sea_orm::ConnectionTrait;

#[tokio::test]
async fn a_grant_on_a_live_project_is_not_orphaned() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "orphan-live").await;
    let ctx = support::ctx(&ws, &owner);

    let project = db
        .project_repo()
        .create(
            &ctx,
            NewProject {
                name: "Orphan diagnostics project".into(),
                slug: "orphan-live".into(),
                task_prefix: "ORL".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: atlas_custos::WorkspaceScope(ws.id.0),
            user_id: Some(owner.id),
            api_key_id: None,
            group_id: None,
            resource_ref: resource_ref_codec::to_core(&ResourceRef::Project(project.id), ws.id),
            role: ResourceRole::Viewer,
            created_by_user_id: Some(owner.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert grant");

    let orphans = count_orphaned_grants(db.conn())
        .await
        .expect("count_orphaned_grants");
    assert_eq!(orphans, 0, "a grant on a live project is not orphaned");

    db.teardown().await;
}

#[tokio::test]
async fn a_grant_naming_a_vanished_project_is_orphaned() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "orphan-vanished").await;

    let vanished_project_id = uuid::Uuid::now_v7();
    let resource_ref = format!("acta::project::{vanished_project_id}");

    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO custos.permission_grants \
             (id, workspace_id, user_id, resource_ref, role, created_at, updated_at) \
             VALUES ('{}', '{}', '{}', '{resource_ref}', 'viewer', now(), now())",
            uuid::Uuid::now_v7(),
            ws.id.0,
            owner.id.0,
        ))
        .await
        .expect("seed orphaned grant referencing a never-created project");

    let orphans = count_orphaned_grants(db.conn())
        .await
        .expect("count_orphaned_grants");
    assert_eq!(
        orphans, 1,
        "a grant naming a project that never existed (or has been hard-deleted \
         outside the hygiene path) must be counted as orphaned"
    );

    db.teardown().await;
}
