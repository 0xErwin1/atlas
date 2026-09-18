//! Container-backed tests for the scope wire-form migration
//! (`m20260918_000055_custos_scope_wire_form`, E4-S2b). The migration
//! rewrites every stored `custos.api_keys.scopes` entry from the legacy
//! `<family>:<action>` spelling to the canonical
//! `<product>::<kind>::<action>` form, mirroring exactly what the stored
//! reader (`capabilities_from_stored`) already accepts: unknown or malformed
//! entries are dropped fail-closed, never coerced and never defaulted.
//!
//! The R1 property is pinned here: a key whose stored scope set was EMPTY
//! before the migration must still be empty (deny-all) after it — the
//! mandatory-ceiling work must never backfill pre-existing dead credentials
//! into read-only grants.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use atlas_test_db::TestDb;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};
use sea_orm_migration::MigratorTrait;
use sea_orm_migration::prelude::SchemaManager;
use uuid::Uuid;

/// Migration prefix that stops immediately before the scope wire-form
/// migration: the last entry of `custos_new()`. Deriving the stop from the
/// list's tail keeps the prefix correct as further Custos migrations are
/// appended (the same name-pinned pattern as
/// `principals_repo_characterization.rs`, expressed against the list tail
/// because the migration under test is by definition the newest one).
fn steps_before_scope_wire_form_migration() -> u32 {
    let historical = migration::Migrator::migrations().len() as u32;
    let custos = atlas_custos_postgres::migrations::custos_new();
    historical + custos.len() as u32 - 1
}

/// The scope wire-form migration, resolved from `custos_new()` so the tests
/// exercise exactly the migration that ships in the composed migrator.
fn scope_wire_form_migration() -> Box<dyn sea_orm_migration::prelude::MigrationTrait> {
    atlas_custos_postgres::migrations::custos_new()
        .pop()
        .expect("custos_new() is non-empty")
}

async fn db_before_scope_wire_form_migration() -> TestDb {
    TestDb::create_with_migration_steps(Some(steps_before_scope_wire_form_migration()))
        .await
        .expect("TestDb at the pre-rewrite migration prefix")
}

/// Seeds the owner row (one user principal + one user) shared by every key
/// fixture, returning the owner id.
async fn seed_owner(db: &TestDb, username: &str) -> Uuid {
    let owner_id = Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at) \
             VALUES ('{owner_id}', 'user', 'Owner {username}', NULL)"
        ))
        .await
        .expect("seed owner principal");
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO custos.users \
                (id, username, display_name, email, password_hash, is_root, is_system_admin, \
                 disabled_at, activated_at, created_at, updated_at, principal_id) \
             VALUES ('{owner_id}', '{username}', 'Owner {username}', NULL, NULL, \
                 false, false, NULL, now(), now(), now(), '{owner_id}')"
        ))
        .await
        .expect("seed owner user");
    owner_id
}

/// Seeds one agent principal plus one `custos.api_keys` row whose `scopes`
/// array is the exact pre-migration stored bytes under test.
async fn seed_key(db: &TestDb, owner_id: Uuid, name: &str, scopes_sql: &str) -> Uuid {
    let principal_id = Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at) \
             VALUES ('{principal_id}', 'agent', '{name}', NULL)"
        ))
        .await
        .expect("seed agent principal");
    let key_id = Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO custos.api_keys \
                (id, workspace_id, created_by_user_id, name, token_hash, type, created_at, \
                 is_global, principal_id, scopes) \
             VALUES ('{key_id}', NULL, '{owner_id}', '{name}', 'hash-{name}', 'agent', now(), \
                 false, '{principal_id}', {scopes_sql})"
        ))
        .await
        .expect("seed api key row");
    key_id
}

#[derive(Debug, FromQueryResult)]
struct ScopesRow {
    scopes: Vec<String>,
}

async fn stored_scopes(db: &TestDb, key_id: Uuid) -> Vec<String> {
    ScopesRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT scopes FROM custos.api_keys WHERE id = $1",
        [key_id.into()],
    ))
    .one(db.conn())
    .await
    .expect("query scopes")
    .expect("key row must exist")
    .scopes
}

#[tokio::test]
async fn migration_rewrites_legacy_scopes_to_canonical_preserving_order() {
    let db = db_before_scope_wire_form_migration().await;
    let owner = seed_owner(&db, "legacy-rewrite").await;
    let key = seed_key(
        &db,
        owner,
        "legacy-rewrite-key",
        "ARRAY['docs:update','tasks:read','custos::grants::read']",
    )
    .await;

    db.run_remaining_migrations()
        .await
        .expect("apply the scope wire-form migration");

    assert_eq!(
        stored_scopes(&db, key).await,
        vec![
            "acta::docs::update".to_owned(),
            "acta::tasks::read".to_owned(),
            "custos::grants::read".to_owned(),
        ],
        "every legacy entry must be rewritten to its canonical spelling, in stored order"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn migration_leaves_already_canonical_entries_untouched() {
    let db = db_before_scope_wire_form_migration().await;
    let owner = seed_owner(&db, "canonical-passthrough").await;
    let key = seed_key(
        &db,
        owner,
        "canonical-key",
        "ARRAY['acta::tasks::read','custos::grants::read']",
    )
    .await;

    db.run_remaining_migrations()
        .await
        .expect("apply the scope wire-form migration");

    assert_eq!(
        stored_scopes(&db, key).await,
        vec![
            "acta::tasks::read".to_owned(),
            "custos::grants::read".to_owned(),
        ],
        "canonical entries must pass through byte-identical (idempotency by construction)"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn migration_drops_unknown_and_malformed_entries_fail_closed() {
    let db = db_before_scope_wire_form_migration().await;
    let owner = seed_owner(&db, "garbage-drop").await;
    let key = seed_key(
        &db,
        owner,
        "garbage-key",
        "ARRAY['tasks:manage','nonsense','','acta::tasks::read::extra',\
               'custos::grants::create','grants:create','tasks:read']",
    )
    .await;

    db.run_remaining_migrations()
        .await
        .expect("apply the scope wire-form migration");

    let scopes = stored_scopes(&db, key).await;
    assert_eq!(
        scopes,
        vec!["acta::tasks::read".to_owned()],
        "unknown or malformed entries must be dropped, never coerced; only the one valid \
         entry may survive"
    );
    for raw in &scopes {
        assert!(
            raw.parse::<atlas_custos::capability::Capability>().is_ok(),
            "no surviving entry may rely on legacy tolerance: {raw}"
        );
    }

    db.teardown().await.expect("teardown");
}

/// R1: the empty stored set is the deny-all credential. The mandatory-ceiling
/// rule means the ceiling is always present for NEW keys (the create path
/// falls back to `Capability::DEFAULT_READ_ONLY`); it must never be applied
/// retroactively to pre-existing empty rows, which must keep denying.
#[tokio::test]
async fn pre_migration_empty_scope_key_stays_empty_and_denies_after_migration() {
    let db = db_before_scope_wire_form_migration().await;
    let owner = seed_owner(&db, "r1-empty").await;
    let key = seed_key(&db, owner, "empty-scope-key", "'{}'::text[]").await;

    db.run_remaining_migrations()
        .await
        .expect("apply the scope wire-form migration");

    let scopes = stored_scopes(&db, key).await;
    assert!(
        scopes.is_empty(),
        "a genuinely empty pre-migration scope set must stay empty after the migration, \
         got: {scopes:?}"
    );
    let effective = scopes
        .iter()
        .filter_map(|raw| raw.parse::<atlas_custos::capability::Capability>().ok())
        .count();
    assert_eq!(
        effective, 0,
        "the migrated empty key must still resolve to zero capabilities (deny-all)"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn migration_is_idempotent_when_applied_twice() {
    let db = db_before_scope_wire_form_migration().await;
    let owner = seed_owner(&db, "idempotent").await;
    let legacy = seed_key(
        &db,
        owner,
        "idempotent-legacy",
        "ARRAY['tasks:read','docs:create','grants:read']",
    )
    .await;
    let garbage = seed_key(
        &db,
        owner,
        "idempotent-garbage",
        "ARRAY['bogus','tasks:update']",
    )
    .await;

    db.run_remaining_migrations()
        .await
        .expect("apply the scope wire-form migration");
    let after_first = (
        stored_scopes(&db, legacy).await,
        stored_scopes(&db, garbage).await,
    );

    scope_wire_form_migration()
        .up(&SchemaManager::new(db.conn()))
        .await
        .expect("re-apply the migration");

    assert_eq!(
        (
            stored_scopes(&db, legacy).await,
            stored_scopes(&db, garbage).await
        ),
        after_first,
        "a second application must be a no-op: canonical entries pass through, dropped \
         entries stay dropped"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn down_restores_the_legacy_spelling() {
    let db = db_before_scope_wire_form_migration().await;
    let owner = seed_owner(&db, "down-restore").await;
    let legacy = seed_key(
        &db,
        owner,
        "down-legacy",
        "ARRAY['tasks:read','grants:read']",
    )
    .await;
    let empty = seed_key(&db, owner, "down-empty", "'{}'::text[]").await;

    db.run_remaining_migrations()
        .await
        .expect("apply the scope wire-form migration");

    scope_wire_form_migration()
        .down(&SchemaManager::new(db.conn()))
        .await
        .expect("revert the migration");

    assert_eq!(
        stored_scopes(&db, legacy).await,
        vec!["tasks:read".to_owned(), "grants:read".to_owned()],
        "down() must restore the legacy <family>:<action> spelling"
    );
    assert!(
        stored_scopes(&db, empty).await.is_empty(),
        "down() must not invent entries for an empty scope set"
    );

    db.teardown().await.expect("teardown");
}
