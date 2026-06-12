#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::ports::permission_grant_repo::ResolutionQuery;
use atlas_domain::{
    entities::permissions::NewPermissionGrant,
    entities::workspace_core::NewProject,
    permissions::{ResourceRef, ResourceRole},
};
use atlas_server::persistence::repos::{PermissionGrantRepo, PgPermissionGrantRepo, ProjectRepo};

#[tokio::test]
async fn upsert_creates_and_updates_on_conflict() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "alice").await;
    let ctx = support::ctx(&ws, &user);

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    let project_repo = db.project_repo();

    let project = project_repo
        .create(
            &ctx,
            NewProject {
                name: "Alpha".into(),
                slug: "alpha".into(),
                task_prefix: "ALP".into(),
            },
        )
        .await
        .expect("create project");

    let grant = NewPermissionGrant {
        workspace_id: ws.id,
        user_id: Some(user.id),
        api_key_id: None,
        project_id: Some(project.id),
        folder_id: None,
        document_id: None,
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: Some(user.id),
        created_by_api_key_id: None,
    };

    let created = grant_repo
        .upsert(grant.clone())
        .await
        .expect("upsert create");
    assert_eq!(created.role, ResourceRole::Viewer);

    let updated = grant_repo
        .upsert(NewPermissionGrant {
            role: ResourceRole::Editor,
            ..grant
        })
        .await
        .expect("upsert update");

    assert_eq!(updated.role, ResourceRole::Editor);
    assert_eq!(created.id.0, updated.id.0, "same row updated");

    db.teardown().await;
}

#[tokio::test]
async fn load_grants_for_resolution_returns_matching_grants() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "bob").await;
    let ctx = support::ctx(&ws, &user);

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    let project_repo = db.project_repo();

    let project = project_repo
        .create(
            &ctx,
            NewProject {
                name: "Beta".into(),
                slug: "beta".into(),
                task_prefix: "BET".into(),
            },
        )
        .await
        .expect("create project");

    // Workspace-scope grant (all targets null).
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(user.id),
            api_key_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Viewer,
            created_by_user_id: Some(user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("ws-scope upsert");

    // Project-scope grant.
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(user.id),
            api_key_id: None,
            project_id: Some(project.id),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Editor,
            created_by_user_id: Some(user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("project-scope upsert");

    let grants = grant_repo
        .load_grants_for_resolution(ResolutionQuery {
            workspace_id: ws.id,
            user_id: Some(user.id.0),
            api_key_id: None,
            chain_projects: vec![project.id.0],
            chain_folders: vec![],
            doc_id: None,
            board_id: None,
        })
        .await
        .expect("load_grants_for_resolution");

    assert_eq!(grants.len(), 2, "should return ws-scope and project-scope");
    assert!(
        grants
            .iter()
            .any(|(r, role)| r == &ResourceRef::Workspace && *role == ResourceRole::Viewer)
    );
    assert!(
        grants.iter().any(
            |(r, role)| r == &ResourceRef::Project(project.id) && *role == ResourceRole::Editor
        )
    );

    db.teardown().await;
}

#[tokio::test]
async fn load_grants_cross_tenant_returns_empty() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let (ws_a, user_a) = support::seed_workspace(&db, "crossa").await;
    let (ws_b, _user_b) = support::seed_workspace(&db, "crossb").await;
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };

    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws_a.id,
            user_id: Some(user_a.id),
            api_key_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Editor,
            created_by_user_id: Some(user_a.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert ws_a grant");

    let grants = grant_repo
        .load_grants_for_resolution(ResolutionQuery {
            workspace_id: ws_b.id,
            user_id: Some(user_a.id.0),
            api_key_id: None,
            chain_projects: vec![],
            chain_folders: vec![],
            doc_id: None,
            board_id: None,
        })
        .await
        .expect("cross-tenant query");

    assert!(
        grants.is_empty(),
        "cross-tenant query must return no grants"
    );

    db.teardown().await;
}
