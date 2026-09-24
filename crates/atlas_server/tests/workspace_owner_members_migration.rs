#![allow(clippy::expect_used, clippy::unwrap_used)]
//! `v2-e7-s4`: `m20260924_000059_acta_workspace_owner_members` round-trip.
//! `up` adds `acta.workspaces.owner_principal_id` and creates
//! `acta.workspace_members` with its principal index; `down` removes
//! exactly those, leaving `acta.workspaces` and `acta.workspace_memberships`
//! in place.

mod support;

use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::MigratorTrait;
use support::TestDb;

const MIGRATION: &str = "m20260924_000059_acta_workspace_owner_members";

/// Number of composed migration steps that land before this migration,
/// counted up to its name so appending later migrations cannot retarget it.
fn steps_before_migration() -> u32 {
    let historical = migration::Migrator::migrations().len();
    let custos = atlas_custos_postgres::migrations::custos_new().len();
    let acta = atlas_acta_postgres::migrations::acta_new();
    let offset = acta
        .iter()
        .position(|migration| migration.name() == MIGRATION)
        .expect("the workspace owner/members migration is present in acta_new()");
    (historical + custos + offset) as u32
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

async fn column_exists(conn: &sea_orm::DatabaseConnection, table: &str, column: &str) -> bool {
    conn.query_one_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT EXISTS ( \
             SELECT 1 FROM information_schema.columns \
             WHERE table_schema = 'acta' AND table_name = $1 AND column_name = $2 \
         ) AS exists",
        [table.into(), column.into()],
    ))
    .await
    .expect("query column")
    .expect("column existence row")
    .try_get::<bool>("", "exists")
    .expect("column existence")
}

#[tokio::test]
async fn migration_up_then_down_round_trips_the_column_table_and_index() {
    let db = TestDb::create_with_migration_steps(Some(steps_before_migration()))
        .await
        .expect("TestDb paused before the migration");

    assert!(!column_exists(db.conn(), "workspaces", "owner_principal_id").await);
    assert!(!relation_exists(db.conn(), "acta.workspace_members").await);

    ComposedMigrator::up(db.conn(), Some(1))
        .await
        .expect("apply the migration");

    assert!(column_exists(db.conn(), "workspaces", "owner_principal_id").await);
    assert!(relation_exists(db.conn(), "acta.workspace_members").await);
    assert!(relation_exists(db.conn(), "acta.workspace_members_principal_id_idx").await);

    ComposedMigrator::down(db.conn(), Some(1))
        .await
        .expect("revert the migration");

    assert!(!column_exists(db.conn(), "workspaces", "owner_principal_id").await);
    assert!(!relation_exists(db.conn(), "acta.workspace_members").await);
    assert!(
        relation_exists(db.conn(), "acta.workspaces").await
            && relation_exists(db.conn(), "acta.workspace_memberships").await,
        "down leaves the V1 identity tables alone"
    );

    db.teardown().await;
}

#[tokio::test]
async fn migration_is_idempotent_when_reapplied() {
    let db = TestDb::create().await.expect("fully migrated TestDb");

    let migration = atlas_acta_postgres::migrations::acta_new()
        .into_iter()
        .find(|m| m.name() == MIGRATION)
        .expect("migration present in acta_new()");

    let manager = sea_orm_migration::SchemaManager::new(db.conn());
    migration
        .up(&manager)
        .await
        .expect("re-running up against an already-migrated database must be a no-op");

    assert!(column_exists(db.conn(), "workspaces", "owner_principal_id").await);
    assert!(relation_exists(db.conn(), "acta.workspace_members").await);

    db.teardown().await;
}
