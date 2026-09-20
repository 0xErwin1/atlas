//! Container-backed tests for the `custos.sessions.root_reason` migration
//! (`m20260920_000057`) — the break-glass justification slice
//! (`v2-e4-s3c-root-reason`).
//!
//! The coverage seeds a pre-migration-shaped database (the migration prefix
//! stops right after `m20260919_000056`), inserts root and non-root rows in
//! their pre-migration shape, then applies the remaining migrations and
//! asserts the migration's three effects:
//!
//! 1. the `custos.sessions.root_reason` column exists (nullable TEXT);
//! 2. every existing session belonging to a root user is revoked — a dated
//!    root session carries no stated reason, so it must not survive the
//!    switch to justified root access;
//! 3. every existing *personal* API key whose creator is root is revoked (a
//!    personal key has no session and therefore no justification), while a
//!    root user's agent keys and every non-root row stay untouched.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

/// Number of migrations to apply so that the named Custos migration is the
/// last one applied (counted inclusively, after the frozen historical block).
/// Pinned by name so appending further Custos migrations cannot silently
/// shift these prefixes.
fn steps_through(migration_name: &str) -> u32 {
    let historical = migration::Migrator::migrations().len();
    let custos = atlas_custos_postgres::migrations::custos_new();
    let mut steps = historical as u32;
    for m in &custos {
        steps += 1;
        if m.name() == migration_name {
            return steps;
        }
    }
    panic!("custos migration {migration_name} not found in custos_new()");
}

const ROOT_REASON_MIGRATION: &str = "m20260920_000057_custos_session_root_reason";
const PREDECESSOR_MIGRATION: &str = "m20260919_000056_custos_principal_owner";

/// Migration prefix that stops immediately before the root-reason migration,
/// so a test can seed rows in their pre-migration shape and then apply the
/// remaining migrations to exercise the back-fill against real rows.
fn steps_before_root_reason_migration() -> u32 {
    assert_eq!(
        steps_through(PREDECESSOR_MIGRATION) + 1,
        steps_through(ROOT_REASON_MIGRATION),
        "the root-reason migration must directly follow its named predecessor"
    );
    steps_through(ROOT_REASON_MIGRATION) - 1
}

async fn db_before_root_reason_migration() -> atlas_test_db::TestDb {
    atlas_test_db::TestDb::create_with_migration_steps(Some(steps_before_root_reason_migration()))
        .await
        .expect("TestDb::create_with_migration_steps")
}

async fn exec(db: &atlas_test_db::TestDb, sql: String) {
    db.conn()
        .execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .expect("execute statement");
}

#[derive(Debug, FromQueryResult)]
struct RevokedRow {
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Reads `revoked_at` for one row of `table` by id.
async fn revoked_at_of(
    db: &atlas_test_db::TestDb,
    table: &str,
    id: Uuid,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let rows = RevokedRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!("SELECT revoked_at FROM custos.{table} WHERE id = '{id}'"),
    ))
    .all(db.conn())
    .await
    .expect("query revoked_at");

    assert_eq!(rows.len(), 1, "{table} row {id} must exist");
    rows.into_iter().next().unwrap().revoked_at
}

/// Seeds one `custos.users` row plus its backing `user` principal (the mirror
/// every writer maintains: `principals.id = users.id`), and returns the user id.
async fn seed_user(db: &atlas_test_db::TestDb, username: &str, is_root: bool) -> Uuid {
    let id = Uuid::now_v7();
    let root_flag = if is_root { "true" } else { "false" };
    exec(
        db,
        format!(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at, \
             created_at, updated_at) \
             VALUES ('{id}', 'user', '{username}', NULL, now(), now())"
        ),
    )
    .await;
    exec(
        db,
        format!(
            "INSERT INTO custos.users (id, username, display_name, email, password_hash, \
             is_root, is_system_admin, disabled_at, activated_at, created_at, updated_at, \
             principal_id) \
             VALUES ('{id}', '{username}', '{username}', NULL, NULL, {root_flag}, false, \
             NULL, now(), now(), now(), '{id}')"
        ),
    )
    .await;
    id
}

/// Seeds one pre-migration session row (no `root_reason` column yet) for the
/// user, active (never revoked, not expired).
async fn seed_active_session(db: &atlas_test_db::TestDb, user_id: Uuid, tag: &str) -> Uuid {
    let id = Uuid::now_v7();
    exec(
        db,
        format!(
            "INSERT INTO custos.sessions (id, user_id, token_hash, expires_at, \
             last_used_at, revoked_at, created_at) \
             VALUES ('{id}', '{user_id}', 'hash-{tag}', now() + interval '1 day', \
             NULL, NULL, now())"
        ),
    )
    .await;
    id
}

/// Seeds one pre-migration api key row of the given principal kind for the
/// user, active (never revoked). The key links to its own principal row of
/// `kind` (`user` for a personal key, `agent` for an agent key).
async fn seed_active_api_key(
    db: &atlas_test_db::TestDb,
    creator: Uuid,
    kind: &str,
    tag: &str,
) -> Uuid {
    let principal_id = Uuid::now_v7();
    let key_id = Uuid::now_v7();
    let (owner_col, owner_values) = if kind == "agent" {
        ("owner_user_id, ", format!("'{creator}', "))
    } else {
        ("", String::new())
    };
    exec(
        db,
        format!(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at, \
             {owner_col}created_at, updated_at) \
             VALUES ('{principal_id}', '{kind}', '{tag}', NULL, {owner_values}now(), now())"
        ),
    )
    .await;
    exec(
        db,
        format!(
            "INSERT INTO custos.api_keys (id, workspace_id, created_by_user_id, name, \
             token_hash, type, expires_at, last_used_at, revoked_at, created_at, is_global, \
             scopes, principal_id) \
             VALUES ('{key_id}', NULL, '{creator}', '{tag}', 'hash-{tag}', 'agent', \
             NULL, NULL, NULL, now(), false, '{{}}', '{principal_id}')"
        ),
    )
    .await;
    key_id
}

#[tokio::test]
async fn migration_adds_the_nullable_root_reason_column() {
    let db = db_before_root_reason_migration().await;

    db.run_remaining_migrations()
        .await
        .expect("apply remaining migrations");

    let result = db
        .conn()
        .execute_unprepared("SELECT root_reason FROM custos.sessions LIMIT 1")
        .await;
    assert!(
        result.is_ok(),
        "custos.sessions.root_reason must exist after the migration: {:?}",
        result.err().map(|e| e.to_string())
    );

    db.teardown().await.expect("teardown test database");
}

#[tokio::test]
async fn migration_revokes_every_existing_root_session_and_keeps_non_root_sessions() {
    let db = db_before_root_reason_migration().await;

    let root = seed_user(&db, "root-reason-mig-root", true).await;
    let plain = seed_user(&db, "root-reason-mig-plain", false).await;
    let root_session = seed_active_session(&db, root, "root-session").await;
    let plain_session = seed_active_session(&db, plain, "plain-session").await;

    db.run_remaining_migrations()
        .await
        .expect("apply remaining migrations");

    assert!(
        revoked_at_of(&db, "sessions", root_session).await.is_some(),
        "a pre-migration root session carries no stated reason and must be revoked"
    );
    assert!(
        revoked_at_of(&db, "sessions", plain_session)
            .await
            .is_none(),
        "a non-root session needs no justification and must stay active"
    );

    db.teardown().await.expect("teardown test database");
}

#[tokio::test]
async fn migration_revokes_root_personal_keys_only() {
    let db = db_before_root_reason_migration().await;

    let root = seed_user(&db, "root-reason-mig-keys-root", true).await;
    let plain = seed_user(&db, "root-reason-mig-keys-plain", false).await;
    let root_personal = seed_active_api_key(&db, root, "user", "root-personal").await;
    let root_agent = seed_active_api_key(&db, root, "agent", "root-agent").await;
    let plain_personal = seed_active_api_key(&db, plain, "user", "plain-personal").await;

    db.run_remaining_migrations()
        .await
        .expect("apply remaining migrations");

    assert!(
        revoked_at_of(&db, "api_keys", root_personal)
            .await
            .is_some(),
        "a root personal key has no session and therefore no justification; it must be revoked"
    );
    assert!(
        revoked_at_of(&db, "api_keys", root_agent).await.is_none(),
        "a root user's agent key is not a personal credential; it must stay active"
    );
    assert!(
        revoked_at_of(&db, "api_keys", plain_personal)
            .await
            .is_none(),
        "a non-root personal key is unaffected"
    );

    db.teardown().await.expect("teardown test database");
}
