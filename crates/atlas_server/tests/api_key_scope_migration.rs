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
/// migration, then applies the remaining migrations, and asserts the row is
/// grandfathered to the full 20-entry catalog — i.e. its effective access is
/// identical to what it was pre-migration (unrestricted), not silently
/// downgraded to read-only or empty.
///
/// The frozen `m20260705_000038_apikey_scopes` back-fill writes the raw legacy
/// `<family>:<action>` bytes in `ALL_TWENTY` below; E4-S2b's
/// `m20260918_000055_custos_scope_wire_form` then rewrites every stored entry
/// to the canonical `<product>::<kind>::<action>` form, so the final stored
/// bytes this test observes are the canonical spellings. The spelling
/// changes; the effective capability set (and therefore access) does not.
/// Inverts the canonical spelling the scope wire-form migration stores: the
/// 20 legacy catalog strings above map 1:1 onto `acta::<family>::<action>`.
fn legacy_to_canonical(legacy: &str) -> String {
    let (family, action) = legacy
        .split_once(':')
        .expect("catalog legacy strings are well-formed <family>:<action>");
    format!("acta::{family}::{action}")
}

#[tokio::test]
async fn pre_migration_key_is_grandfathered_to_all_twenty_scopes_after_backfill() {
    // Stop one migration short of the scopes migration, so `api_keys` has no
    // `scopes` column yet — this is the exact pre-migration shape. At this step
    // `custos_new()` (including the S3d SET SCHEMA migration) has not run yet,
    // so `users`/`api_keys` still physically live in `public`. Seeding via raw
    // SQL rather than `support::seed_workspace` is deliberate: the repo layer
    // goes through sea-orm entities that now hardcode `schema_name = "custos"`,
    // which would look for a table that does not exist yet at this migration step.
    let db = TestDb::create_with_migration_steps(Some(37))
        .await
        .expect("create db at pre-scopes migration state");

    // schema-gate:off — pre-schema-move fixture, see the doc comment above.
    let user_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO users (id, username, display_name, is_root, is_system_admin, created_at, updated_at) \
             VALUES ('{user_id}', 'pre-scopes-migration', 'Pre Scopes Migration', false, false, now(), now())"
        ))
        .await
        .expect("seed pre-migration-shaped user row");

    let key_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global) \
             VALUES ($1, NULL, $2, 'pre-migration-key', 'pre-migration-hash', 'agent', now(), false)",
            [key_id.into(), user_id.into()],
        ))
        .await
        // schema-gate:on
        .expect("insert pre-migration-shaped api key row");

    db.run_remaining_migrations()
        .await
        .expect("apply scopes migration");

    let row = ScopesRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT scopes FROM custos.api_keys WHERE id = $1",
        [key_id.into()],
    ))
    .one(db.conn())
    .await
    .expect("query scopes")
    .expect("row must exist");

    let mut got = row.scopes.clone();
    got.sort();
    let mut want: Vec<String> = ALL_TWENTY
        .iter()
        .map(|legacy| legacy_to_canonical(legacy))
        .collect();
    want.sort();

    assert_eq!(
        got, want,
        "a key that existed before the scopes migration must be grandfathered to all 20 \
         capabilities, stored in the canonical wire form"
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

    let principal_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at) \
             VALUES ($1, 'agent', 'post-migration-key', NULL)",
            [principal_id.into()],
        ))
        .await
        .expect("seed agent principal");
    let key_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO custos.api_keys (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, is_global, principal_id) \
             VALUES ($1, NULL, $2, 'post-migration-key', 'post-migration-hash', 'agent', now(), false, $3)",
            [key_id.into(), user.id.0.into(), principal_id.into()],
        ))
        .await
        .expect("insert row omitting scopes");

    let row = ScopesRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT scopes FROM custos.api_keys WHERE id = $1",
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
