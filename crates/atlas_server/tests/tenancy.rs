#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::entities::workspace_core::NewProject;
use atlas_server::persistence::repos::{ApiKeyRepo, ProjectRepo};

#[tokio::test]
async fn api_key_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice").await;
    let (_ws_b, _user_b) = support::seed_workspace(&db, "bob").await;

    let ctx_a = support::ctx(&ws_a, &user_a);

    let repo = db.api_key_repo();

    let keys_a = repo.list(&ctx_a).await.expect("list keys for workspace A");
    assert!(
        keys_a.is_empty(),
        "workspace A must not see workspace B's api keys"
    );

    db.teardown().await;
}

#[tokio::test]
async fn project_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-proj").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-proj").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);
    let repo = db.project_repo();

    repo.create(
        &ctx_b,
        NewProject {
            name: "Bob's Project".into(),
            slug: "bobs-project".into(),
            task_prefix: "BP".into(),
        },
    )
    .await
    .expect("create project in ws_b");

    let projects_a = repo.list(&ctx_a).await.expect("list for ws_a");
    assert!(
        projects_a.is_empty(),
        "workspace A must not see workspace B's projects"
    );

    db.teardown().await;
}
