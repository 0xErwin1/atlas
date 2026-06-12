#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use sea_orm::ConnectionTrait;

#[tokio::test]
async fn test_db_creates_and_migrates_and_drops() {
    let db = support::TestDb::create()
        .await
        .expect("TestDb::create must succeed");

    let result = db
        .conn()
        .execute_unprepared("SELECT 1 FROM seaql_migrations LIMIT 1")
        .await;

    assert!(
        result.is_ok(),
        "seaql_migrations must exist after TestDb::create"
    );

    db.teardown().await;
}
