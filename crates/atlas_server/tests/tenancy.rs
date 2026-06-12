#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_server::persistence::repos::ApiKeyRepo;

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
