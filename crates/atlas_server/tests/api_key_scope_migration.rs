#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use sea_orm::{ConnectionTrait, FromQueryResult, Statement};
use support::{TestDb, seed_workspace};

#[derive(Debug, FromQueryResult)]
struct ScopesRow {
    scopes: Vec<String>,
}

const ALL_TWENTY: &[&str] = &[
    "tasks:read",
    "tasks:create",
    "tasks:update",
    "tasks:delete",
    "docs:read",
    "docs:create",
    "docs:update",
    "docs:delete",
    "boards:read",
    "boards:create",
    "boards:update",
    "boards:delete",
    "folders:read",
    "folders:create",
    "folders:update",
    "folders:delete",
    "projects:read",
    "projects:create",
    "projects:update",
    "projects:delete",
];

/// Reproduces a key row exactly as it existed before the `scopes` column
/// migration, then applies that migration, and asserts the row is
/// grandfathered to the full 20-entry catalog — i.e. its effective access is
/// identical to what it was pre-migration (unrestricted), not silently
/// downgraded to read-only or empty.
#[tokio::test]
async fn pre_migration_key_is_grandfathered_to_all_twenty_scopes_after_backfill() {
    // Stop one migration short of the scopes migration, so `api_keys` has no
    // `scopes` column yet — this is the exact pre-migration shape.
    let db = TestDb::create_with_migration_steps(Some(37))
        .await
        .expect("create db at pre-scopes migration state");

    let (_ws, user) = seed_workspace(&db, "pre-scopes-migration").await;

    let key_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global) \
             VALUES ($1, NULL, $2, 'pre-migration-key', 'pre-migration-hash', 'agent', now(), false)",
            [key_id.into(), user.id.0.into()],
        ))
        .await
        .expect("insert pre-migration-shaped api key row");

    db.run_remaining_migrations()
        .await
        .expect("apply scopes migration");

    let row = ScopesRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT scopes FROM api_keys WHERE id = $1",
        [key_id.into()],
    ))
    .one(db.conn())
    .await
    .expect("query scopes")
    .expect("row must exist");

    let mut got = row.scopes.clone();
    got.sort();
    let mut want: Vec<String> = ALL_TWENTY.iter().map(|s| s.to_string()).collect();
    want.sort();

    assert_eq!(
        got, want,
        "a key that existed before the scopes migration must be grandfathered to all 20 capabilities"
    );

    db.teardown().await;
}

/// A key created fresh after the migration (never touched by the back-fill)
/// gets the column's `DEFAULT '{}'` when the insert omits `scopes` entirely —
/// the fail-safe deny-all default, distinct from the grandfather back-fill.
#[tokio::test]
async fn post_migration_insert_without_scopes_defaults_to_empty() {
    let db = TestDb::create()
        .await
        .expect("create db with all migrations");
    let (_ws, user) = seed_workspace(&db, "post-scopes-migration").await;

    let key_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global) \
             VALUES ($1, NULL, $2, 'post-migration-key', 'post-migration-hash', 'agent', now(), false)",
            [key_id.into(), user.id.0.into()],
        ))
        .await
        .expect("insert row omitting scopes");

    let row = ScopesRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT scopes FROM api_keys WHERE id = $1",
        [key_id.into()],
    ))
    .one(db.conn())
    .await
    .expect("query scopes")
    .expect("row must exist");

    assert!(
        row.scopes.is_empty(),
        "the column default must fail closed (deny-all) for any insert path that forgets scopes"
    );

    db.teardown().await;
}
