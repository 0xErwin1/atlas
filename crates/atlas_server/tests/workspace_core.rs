#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_api::dtos::{CreateProjectRequest, folders::CreateFolderRequest};
use atlas_client::ClientError;
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::identity::{ApiKeyType, NewApiKey},
    entities::workspace_core::{
        AppliesTo, NewFolder, NewProject, NewPropertyDefinition, PropertyKind,
    },
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    ApiKeyRepo, FolderRepo, ProjectRepo, PropertyDefinitionRepo,
};

// ---- Characterization: duplicate-name unique-violation maps to 409 --------------
//
// Pins the current behavior of `workspace_core.rs`'s `db_err` before the S2b-2
// consolidation touches it: a `folders_name_uq` unique-constraint violation
// (same workspace/project/parent/name) must surface as HTTP 409 with a fixed
// `AlreadyExists` message, not the generic `Internal.message` derived from
// `e.to_string()`. This test must stay green across the consolidation.
#[tokio::test]
async fn duplicate_folder_name_returns_409_with_fixed_message() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "dup-folder-name-409").await;

    let project = client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "DupNameProj".into(),
                slug: "dup-name-proj".into(),
                task_prefix: "DNP".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Docs".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create first folder");

    let result = client
        .create_folder(
            &ws.slug,
            &project.slug,
            CreateFolderRequest {
                name: "Docs".to_string(),
                parent_folder_id: None,
            },
        )
        .await;

    match result {
        Err(ClientError::Api(ref p)) => {
            assert_eq!(p.status, 409, "duplicate folder name must return 409");
            assert_eq!(
                p.detail.as_deref(),
                Some("an item with the same name already exists in this location"),
                "duplicate folder name must keep the fixed AlreadyExists message"
            );
        }
        other => panic!("duplicate folder name must return 409 AlreadyExists, got {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn project_slug_unique_per_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "slug-test-user").await;
    let ctx = support::ctx(&ws, &user);
    let repo = db.project_repo();

    repo.create(
        &ctx,
        NewProject {
            name: "Alpha".into(),
            slug: "alpha".into(),
            task_prefix: "ALPHA".into(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("first project");

    let result = repo
        .create(
            &ctx,
            NewProject {
                name: "Alpha Duplicate".into(),
                slug: "alpha".into(),
                task_prefix: "ALPHA2".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await;

    assert!(result.is_err(), "duplicate slug must be rejected");

    db.teardown().await;
}

#[tokio::test]
async fn project_task_prefix_unique_per_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "prefix-test-user").await;
    let ctx = support::ctx(&ws, &user);
    let repo = db.project_repo();

    repo.create(
        &ctx,
        NewProject {
            name: "Beta".into(),
            slug: "beta".into(),
            task_prefix: "BT".into(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("first project");

    let result = repo
        .create(
            &ctx,
            NewProject {
                name: "Beta2".into(),
                slug: "beta2".into(),
                task_prefix: "BT".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await;

    assert!(result.is_err(), "duplicate task_prefix must be rejected");

    db.teardown().await;
}

#[tokio::test]
async fn project_soft_delete_frees_slug_for_reuse() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "soft-delete-user").await;
    let ctx = support::ctx(&ws, &user);
    let repo = db.project_repo();

    let p = repo
        .create(
            &ctx,
            NewProject {
                name: "Gamma".into(),
                slug: "gamma".into(),
                task_prefix: "GM".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project");

    repo.soft_delete(&ctx, p.id).await.expect("soft delete");

    repo.create(
        &ctx,
        NewProject {
            name: "Gamma Reborn".into(),
            slug: "gamma".into(),
            task_prefix: "GMR".into(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("slug reuse after soft delete must succeed");

    db.teardown().await;
}

#[tokio::test]
async fn folder_name_unique_per_parent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "folder-user").await;
    let ctx = support::ctx(&ws, &user);
    let folder_repo = db.folder_repo();

    folder_repo
        .create(
            &ctx,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "Docs".into(),
            },
        )
        .await
        .expect("first folder");

    let result = folder_repo
        .create(
            &ctx,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "Docs".into(),
            },
        )
        .await;

    assert!(
        result.is_err(),
        "duplicate folder name in same parent must be rejected"
    );

    db.teardown().await;
}

#[tokio::test]
async fn property_definition_key_unique_per_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "prop-user").await;
    let ctx = support::ctx(&ws, &user);
    let repo = db.property_definition_repo();

    repo.create(
        &ctx,
        NewPropertyDefinition {
            key: "priority".into(),
            name: "Priority".into(),
            kind: PropertyKind::Select,
            options: None,
            applies_to: AppliesTo::Task,
        },
    )
    .await
    .expect("first property");

    let result = repo
        .create(
            &ctx,
            NewPropertyDefinition {
                key: "priority".into(),
                name: "Priority2".into(),
                kind: PropertyKind::Text,
                options: None,
                applies_to: AppliesTo::Document,
            },
        )
        .await;

    assert!(
        result.is_err(),
        "duplicate property key in same workspace must be rejected"
    );

    db.teardown().await;
}

// ---- Regression: api_key actor can create a project (num_actors_check) ----------

#[tokio::test]
async fn project_created_by_api_key_succeeds() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "api-key-project-actor").await;

    let user_ctx = support::ctx(&ws, &user);
    let key = db
        .api_key_repo()
        .create(
            &user_ctx,
            NewApiKey {
                name: "regression-key-project".to_string(),
                token_hash: atlas_server::auth::tokens::hash_token(
                    "atlas_regression_project_secret",
                ),
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key");

    let api_key_ctx = WorkspaceCtx::new(
        ws.id,
        Actor::ApiKey(atlas_domain::ApiKeyAttributionId(key.id.0)),
    );
    let result = db
        .project_repo()
        .create(
            &api_key_ctx,
            NewProject {
                name: "ApiKeyProject".into(),
                slug: "api-key-project".into(),
                task_prefix: "AKP".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "api_key actor must be able to create a project: {result:?}"
    );

    db.teardown().await;
}

// ---- Regression: api_key actor can create a folder (num_actors_check) -----------

#[tokio::test]
async fn folder_created_by_api_key_succeeds() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "api-key-folder-actor").await;

    let user_ctx = support::ctx(&ws, &user);
    let key = db
        .api_key_repo()
        .create(
            &user_ctx,
            NewApiKey {
                name: "regression-key-folder".to_string(),
                token_hash: atlas_server::auth::tokens::hash_token(
                    "atlas_regression_folder_secret",
                ),
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key");

    let api_key_ctx = WorkspaceCtx::new(
        ws.id,
        Actor::ApiKey(atlas_domain::ApiKeyAttributionId(key.id.0)),
    );
    let result = db
        .folder_repo()
        .create(
            &api_key_ctx,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "ApiKeyFolder".into(),
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "api_key actor must be able to create a folder: {result:?}"
    );

    db.teardown().await;
}
