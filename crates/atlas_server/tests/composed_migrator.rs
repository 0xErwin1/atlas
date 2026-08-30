#![allow(clippy::expect_used)]

mod support;

use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm_migration::prelude::MigratorTrait;
use support::TestDb;

/// D5: the composed migrator (`historical() ++ custos_new()`) must report
/// zero pending against a database freshly migrated from empty, proving the
/// composition applies cleanly with no gaps against `custos_new()`'s (empty)
/// contribution.
#[tokio::test]
async fn from_empty_database_composed_migrator_reports_zero_pending_after_up() {
    let db = TestDb::create_with_migration_steps(Some(0))
        .await
        .expect("empty TestDb");

    ComposedMigrator::up(db.conn(), None)
        .await
        .expect("composed migrator applies cleanly from empty");

    let pending = ComposedMigrator::get_pending_migrations(db.conn())
        .await
        .expect("get_pending_migrations");

    assert!(
        pending.is_empty(),
        "expected zero pending migrations after a full composed up from empty, got: {:?}",
        pending.iter().map(|m| m.name()).collect::<Vec<_>>()
    );

    db.teardown().await;
}

/// D5: a database migrated by the historical `migration::Migrator` alone (a
/// "V1-migrated" database) must also report zero pending against the composed
/// migrator, over the same `seaql_migrations` table — this is what proves the
/// composition is additive, not a competing migration history.
#[tokio::test]
async fn v1_migrated_database_composed_migrator_reports_zero_pending() {
    let db = TestDb::create().await.expect("V1-migrated TestDb");

    let pending = ComposedMigrator::get_pending_migrations(db.conn())
        .await
        .expect("get_pending_migrations");

    assert!(
        pending.is_empty(),
        "expected zero pending migrations against a V1-migrated database, got: {:?}",
        pending.iter().map(|m| m.name()).collect::<Vec<_>>()
    );

    db.teardown().await;
}
