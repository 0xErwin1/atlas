#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_server::state::AppState;

#[tokio::test]
async fn anchor_interval_field_exists_and_is_u32() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("for_test");
    let _interval: u32 = state.anchor_interval;
    db.teardown().await;
}

#[tokio::test]
async fn attachments_field_is_arc_attachment_store() {
    let db = support::TestDb::create().await.expect("TestDb");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("for_test");
    let result = state.attachments.exists("nonexistent-digest").await;
    assert!(result.is_ok(), "exists() on empty store must not error");
    assert!(
        !result.expect("exists"),
        "nonexistent digest must not exist"
    );
    db.teardown().await;
}
