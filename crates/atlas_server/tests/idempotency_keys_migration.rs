#![allow(clippy::expect_used, clippy::unwrap_used)]

//! S3 PR3 (T3.1, T3.4–T3.8): `platform.idempotency_keys` migration
//! round-trip (design §D9).

mod support;

use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::MigratorTrait;
use support::TestDb;

/// Number of composed migration steps that land before the idempotency-keys
/// migration — it is appended last inside `acta_new()` (T3.1/T3.2), so the
/// step count right before it is the full composed count minus one.
fn steps_before_idempotency_keys_migration() -> u32 {
    let historical = migration::Migrator::migrations().len();
    let custos = atlas_custos_postgres::migrations::custos_new().len();
    let acta = atlas_acta_postgres::migrations::acta_new().len();

    (historical + custos + acta - 1) as u32
}

async fn relation_exists(conn: &sea_orm::DatabaseConnection, relation: &str) -> bool {
    conn.query_one_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT to_regclass($1) IS NOT NULL AS exists",
        [relation.into()],
    ))
    .await
    .expect("query relation")
    .expect("relation existence row")
    .try_get::<bool>("", "exists")
    .expect("relation existence")
}

async fn index_exists(conn: &sea_orm::DatabaseConnection, index: &str) -> bool {
    relation_exists(conn, index).await
}

/// T3.4/T3.5: `up` creates the table and both indexes; `down` drops exactly
/// the table, leaving the `platform` schema and its sibling `ui_state` table
/// alone.
#[tokio::test]
async fn migration_up_then_down_round_trips_the_table_and_indexes() {
    let db = TestDb::create_with_migration_steps(Some(steps_before_idempotency_keys_migration()))
        .await
        .expect("TestDb paused before the idempotency-keys migration");

    assert!(
        !relation_exists(db.conn(), "platform.idempotency_keys").await,
        "table must not exist before this migration runs"
    );

    ComposedMigrator::up(db.conn(), Some(1))
        .await
        .expect("apply the idempotency-keys migration");

    assert!(
        relation_exists(db.conn(), "platform.idempotency_keys").await,
        "table must exist after up"
    );
    assert!(
        index_exists(db.conn(), "platform.idempotency_keys_scope_key_idx").await,
        "unique scope index must exist after up"
    );
    assert!(
        index_exists(
            db.conn(),
            "platform.idempotency_keys_principal_expires_at_idx"
        )
        .await,
        "principal_id/expires_at composite index must exist after up"
    );
    assert!(
        relation_exists(db.conn(), "platform.ui_state").await,
        "sibling platform.ui_state table must be unaffected by this migration"
    );

    ComposedMigrator::down(db.conn(), Some(1))
        .await
        .expect("revert the idempotency-keys migration");

    assert!(
        !relation_exists(db.conn(), "platform.idempotency_keys").await,
        "table must not exist after down"
    );
    assert!(
        relation_exists(db.conn(), "platform.ui_state").await,
        "down must drop exactly platform.idempotency_keys, leaving platform.ui_state alone"
    );

    db.teardown().await;
}

/// T3.6/T3.7: re-applying the migration against an already-migrated database
/// is a no-op, not an error (`CREATE SCHEMA IF NOT EXISTS`/`CREATE TABLE IF
/// NOT EXISTS`/`CREATE INDEX IF NOT EXISTS` throughout).
#[tokio::test]
async fn migration_is_idempotent_when_reapplied() {
    let db = TestDb::create().await.expect("fully migrated TestDb");

    assert!(
        relation_exists(db.conn(), "platform.idempotency_keys").await,
        "table must exist from the normal migration run"
    );

    let migration = atlas_acta_postgres::migrations::acta_new()
        .into_iter()
        .find(|m| m.name() == "m20260906_000058_acta_platform_idempotency_keys")
        .expect("idempotency-keys migration present in acta_new()");

    let manager = sea_orm_migration::SchemaManager::new(db.conn());
    migration
        .up(&manager)
        .await
        .expect("re-running up against an already-migrated database must be a no-op");

    assert!(
        relation_exists(db.conn(), "platform.idempotency_keys").await,
        "table must still exist after a repeated up"
    );

    db.teardown().await;
}
