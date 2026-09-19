//! Container-backed characterization tests for the `custos.principals`
//! migration (E4-S1) and the `PgPrincipalRepo` adapter. Runs against a
//! disposable Postgres named by `ATLAS_TEST_DATABASE_URL` (see
//! `atlas_test_db`); compile-only where no such database is reachable.
//!
//! Back-fill coverage seeds pre-migration-shaped `custos.users` and
//! `custos.api_keys` rows against a migration prefix that stops right
//! before the principals migration, then applies the remaining migrations
//! and asserts the back-filled principals mirror their source rows.
//! PR2's not-null migration (`m20260918_000054`) is covered by a separate
//! window test that stops right after the principals migration, where
//! `principal_id` exists but is still nullable.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use atlas_test_db::TestDb;
use chrono::{DateTime, Utc};
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

const PRINCIPALS_MIGRATION: &str = "m20260917_000053_custos_principals";

/// Migration prefix that stops immediately before the `custos.principals`
/// migration, so a test can seed rows in their pre-migration shape and then
/// apply the remaining migrations to exercise the back-fill.
fn steps_before_principals_migration() -> u32 {
    steps_through(PRINCIPALS_MIGRATION) - 1
}

async fn db_before_principals_migration() -> TestDb {
    TestDb::create_with_migration_steps(Some(steps_before_principals_migration()))
        .await
        .expect("TestDb::create_with_migration_steps")
}

async fn exec(db: &TestDb, sql: &str) {
    db.conn()
        .execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            sql.to_owned(),
        ))
        .await
        .expect("execute seed statement");
}

/// Inserts a `custos.users` row with no `principal_id` value: that is the
/// pre-migration shape (the column does not exist yet) and, after the
/// principals migration, the nullable-window shape a writer unaware of the
/// mirror produces.
async fn seed_premigration_user(
    db: &TestDb,
    username: &str,
    display_name: &str,
    disabled_at: Option<DateTime<Utc>>,
) -> Uuid {
    let id = Uuid::now_v7();
    let disabled = disabled_at
        .map(|t| format!("'{t}'"))
        .unwrap_or_else(|| "NULL".to_string());
    exec(
        db,
        &format!(
            "INSERT INTO custos.users \
                (id, username, display_name, email, password_hash, is_root, is_system_admin, \
                 disabled_at, activated_at, created_at, updated_at) \
             VALUES ('{id}', '{username}', '{display_name}', NULL, NULL, false, false, \
                 {disabled}, now(), now(), now())"
        ),
    )
    .await;
    id
}

/// Inserts a `custos.api_keys` row with no `principal_id` value: that is
/// the pre-migration shape (the column does not exist yet) and, after the
/// principals migration, the nullable-window shape a writer unaware of the
/// mirror produces.
async fn seed_premigration_api_key(
    db: &TestDb,
    user_id: Uuid,
    name: &str,
    revoked_at: Option<DateTime<Utc>>,
) -> Uuid {
    let id = Uuid::now_v7();
    let revoked = revoked_at
        .map(|t| format!("'{t}'"))
        .unwrap_or_else(|| "NULL".to_string());
    exec(
        db,
        &format!(
            "INSERT INTO custos.api_keys \
                (id, workspace_id, created_by_user_id, name, token_hash, type, expires_at, \
                 last_used_at, revoked_at, created_at, is_global, scopes) \
             VALUES ('{id}', NULL, '{user_id}', '{name}', 'hash-{name}', 'agent', NULL, \
                 NULL, {revoked}, now(), false, '{{}}')"
        ),
    )
    .await;
    id
}

#[derive(FromQueryResult)]
struct PrincipalRow {
    id: Uuid,
    kind: String,
    display_name: String,
    deactivated_at: Option<DateTime<Utc>>,
}

async fn principals(db: &TestDb, where_clause: &str) -> Vec<PrincipalRow> {
    let conn = db.conn();
    let rows = conn
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            format!(
                "SELECT id, kind, display_name, deactivated_at \
                 FROM custos.principals {where_clause}"
            ),
        ))
        .await
        .expect("query principals");
    rows.iter()
        .map(|row| PrincipalRow::from_query_result(row, "").expect("principal row"))
        .collect()
}

#[tokio::test]
async fn user_backfill_creates_one_matching_principal_per_user() {
    let db = db_before_principals_migration().await;
    let disabled_at = DateTime::parse_from_rfc3339("2026-09-01T10:00:00Z")
        .expect("fixed timestamp")
        .with_timezone(&Utc);
    let active = seed_premigration_user(&db, "active-user", "Active User", None).await;
    let disabled =
        seed_premigration_user(&db, "disabled-user", "Disabled User", Some(disabled_at)).await;

    db.run_remaining_migrations()
        .await
        .expect("apply principals migration");

    let active_principal = principals(&db, &format!("WHERE id = '{active}'")).await;
    assert_eq!(
        active_principal.len(),
        1,
        "one principal for the active user"
    );
    assert_eq!(
        active_principal[0].id, active,
        "user principal id equals users.id"
    );
    assert_eq!(active_principal[0].kind, "user");
    assert_eq!(active_principal[0].display_name, "Active User");
    assert_eq!(active_principal[0].deactivated_at, None);

    let disabled_principal = principals(&db, &format!("WHERE id = '{disabled}'")).await;
    assert_eq!(
        disabled_principal.len(),
        1,
        "one principal for the disabled user"
    );
    assert_eq!(disabled_principal[0].kind, "user");
    assert_eq!(disabled_principal[0].display_name, "Disabled User");
    assert_eq!(
        disabled_principal[0].deactivated_at,
        Some(disabled_at),
        "principal deactivated_at mirrors users.disabled_at"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn user_backfill_links_users_principal_id_to_its_principal() {
    let db = db_before_principals_migration().await;
    let user_id = seed_premigration_user(&db, "linked-user", "Linked User", None).await;

    db.run_remaining_migrations()
        .await
        .expect("apply principals migration");

    let link = user_principal_links(&db, user_id).await;
    assert_eq!(link.len(), 1);
    assert_eq!(link[0], user_id, "users.principal_id = users.id");

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn api_key_backfill_creates_one_agent_principal_per_key_with_a_fresh_v7_id() {
    let db = db_before_principals_migration().await;
    let owner = seed_premigration_user(&db, "key-owner", "Key Owner", None).await;
    let revoked_at = DateTime::parse_from_rfc3339("2026-09-02T12:00:00Z")
        .expect("fixed timestamp")
        .with_timezone(&Utc);
    let live_key = seed_premigration_api_key(&db, owner, "live-key", None).await;
    let revoked_key = seed_premigration_api_key(&db, owner, "revoked-key", Some(revoked_at)).await;

    db.run_remaining_migrations()
        .await
        .expect("apply principals migration");

    let live_principals = principals(&db, "WHERE display_name = 'live-key'").await;
    assert_eq!(live_principals.len(), 1, "one principal per api key");
    assert_ne!(
        live_principals[0].id, live_key,
        "agent principal id is fresh, not the key id"
    );
    assert_eq!(live_principals[0].kind, "agent");
    assert_eq!(live_principals[0].deactivated_at, None);
    assert_eq!(
        live_principals[0].id.get_version_num(),
        7,
        "backfilled agent principal id is a UUIDv7"
    );

    let revoked_principals = principals(&db, "WHERE display_name = 'revoked-key'").await;
    assert_eq!(revoked_principals.len(), 1);
    assert_eq!(revoked_principals[0].kind, "agent");
    assert_eq!(
        revoked_principals[0].deactivated_at,
        Some(revoked_at),
        "principal deactivated_at mirrors api_keys.revoked_at"
    );

    for (key_id, principal_id) in [
        (live_key, live_principals[0].id),
        (revoked_key, revoked_principals[0].id),
    ] {
        let link = api_key_principal_links(&db, key_id).await;
        assert_eq!(link.len(), 1, "api key {key_id} is linked");
        assert_eq!(link[0], principal_id);
    }

    db.teardown().await.expect("teardown");
}

// ---------------------------------------------------------------------------
// PR2 — NOT NULL constraint on principal_id + nullable-window back-fill
// ---------------------------------------------------------------------------

/// PR2 applies `NOT NULL` on `custos.users.principal_id` and
/// `custos.api_keys.principal_id`; PR1 deliberately left both nullable, so
/// this assertion only becomes true once `m20260918_000054` is registered.
#[tokio::test]
async fn the_principal_id_columns_are_not_null_after_the_full_migration_set() {
    let db = TestDb::create().await.expect("TestDb::create");

    for table in ["users", "api_keys"] {
        let columns = principal_id_nullability(&db, table).await;
        assert_eq!(
            columns.len(),
            1,
            "one principal_id column on custos.{table}"
        );
        assert_eq!(
            columns[0].is_nullable, "NO",
            "custos.{table}.principal_id must be NOT NULL after the full migration set"
        );
    }

    db.teardown().await.expect("teardown");
}

#[derive(FromQueryResult)]
struct ColumnNullability {
    is_nullable: String,
}

async fn principal_id_nullability(db: &TestDb, table: &str) -> Vec<ColumnNullability> {
    ColumnNullability::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT is_nullable FROM information_schema.columns \
             WHERE table_schema = 'custos' AND table_name = '{table}' \
               AND column_name = 'principal_id'"
        ),
    ))
    .all(db.conn())
    .await
    .expect("query column nullability")
}

/// Between the principals migration (which added `principal_id` nullable and
/// back-filled existing rows) and PR2's not-null migration, the app could
/// still create users and api keys whose `principal_id` stayed NULL, because
/// no writer populated the mirror yet. This test reproduces exactly that
/// state (prefix stops right after the principals migration), seeds both
/// shapes of orphan row, applies PR2's migration, and asserts the back-fill
/// repairs them before the `NOT NULL` constraint lands. A bare `SET NOT
/// NULL` on these rows fails with "column contains null values", which is
/// why the back-fill and the constraint share one migration.
#[tokio::test]
async fn the_not_null_migration_backfills_rows_created_during_the_nullable_window() {
    let db = TestDb::create_with_migration_steps(Some(steps_through(PRINCIPALS_MIGRATION)))
        .await
        .expect("TestDb::create_with_migration_steps");

    // Rows created between the two migrations: the column exists but no
    // writer populated it, so both rows carry a NULL `principal_id`.
    let window_user = seed_premigration_user(&db, "window-user", "Window User", None).await;
    let window_key = seed_premigration_api_key(&db, window_user, "window-key", None).await;

    db.run_remaining_migrations()
        .await
        .expect("apply the not-null migration");

    // The user's principal was already its own id; the back-fill only had
    // to re-point the NULL column.
    let user_links = user_principal_links(&db, window_user).await;
    assert_eq!(user_links.len(), 1, "window user is linked");
    assert_eq!(
        user_links[0], window_user,
        "window user's principal_id is back-filled to users.id"
    );

    // The window key gets a fresh `agent` principal mirroring its name.
    let key_principals = principals(&db, "WHERE display_name = 'window-key'").await;
    assert_eq!(
        key_principals.len(),
        1,
        "one agent principal per window key"
    );
    assert_eq!(key_principals[0].kind, "agent");
    assert_ne!(
        key_principals[0].id, window_key,
        "agent principal id is fresh, not the key id"
    );
    assert_eq!(
        key_principals[0].id.get_version_num(),
        7,
        "window agent principal id is a UUIDv7"
    );
    assert_eq!(key_principals[0].deactivated_at, None);

    let key_links = api_key_principal_links(&db, window_key).await;
    assert_eq!(key_links.len(), 1, "window api key is linked");
    assert_eq!(
        key_links[0], key_principals[0].id,
        "window api key's principal_id points at the agent principal created for it"
    );

    for table in ["users", "api_keys"] {
        let columns = principal_id_nullability(&db, table).await;
        assert_eq!(
            columns[0].is_nullable, "NO",
            "custos.{table}.principal_id must be NOT NULL once the window rows are back-filled"
        );
    }

    db.teardown().await.expect("teardown");
}

async fn user_principal_links(db: &TestDb, user_id: Uuid) -> Vec<Uuid> {
    query_single_uuid_column(
        db,
        &format!("SELECT principal_id FROM custos.users WHERE id = '{user_id}'"),
    )
    .await
}

async fn api_key_principal_links(db: &TestDb, key_id: Uuid) -> Vec<Uuid> {
    query_single_uuid_column(
        db,
        &format!("SELECT principal_id FROM custos.api_keys WHERE id = '{key_id}'"),
    )
    .await
}

async fn query_single_uuid_column(db: &TestDb, sql: &str) -> Vec<Uuid> {
    #[derive(FromQueryResult)]
    struct Row {
        principal_id: Uuid,
    }
    Row::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        sql.to_owned(),
    ))
    .all(db.conn())
    .await
    .expect("query principal_id column")
    .into_iter()
    .map(|row| row.principal_id)
    .collect()
}

// ---------------------------------------------------------------------------
// E4-S1 W6 — real constraint inspection for the additive-introduction guard
// ---------------------------------------------------------------------------

/// The five outbound-FK-holding tables whose constraints E4-S1 must not touch:
/// the principals introduction is additive (new table + new columns on
/// `users`/`api_keys`) and repoints no existing FK.
const FK_HOLDING_TABLES: &[&str] = &[
    "permission_grants",
    "sessions",
    "security_audit_log",
    "group_members",
    "user_activation_tokens",
];

#[derive(FromQueryResult, Debug, PartialEq, Eq, Clone)]
struct FkConstraint {
    table: String,
    name: String,
    definition: String,
}

async fn fk_constraints_on(db: &TestDb, tables: &[&str]) -> Vec<FkConstraint> {
    let table_list = tables
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");
    FkConstraint::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT ns.nspname || '.' || c.relname AS table, \
                    con.conname AS name, \
                    pg_get_constraintdef(con.oid) AS definition \
             FROM pg_constraint con \
             JOIN pg_class c ON c.oid = con.conrelid \
             JOIN pg_namespace ns ON ns.oid = c.relnamespace \
             WHERE con.contype = 'f' \
               AND ns.nspname = 'custos' \
               AND c.relname = ANY(ARRAY[{table_list}]) \
             ORDER BY 1, 2"
        ),
    ))
    .all(db.conn())
    .await
    .expect("query outbound FK constraints")
}

/// Inspects the catalog itself instead of guessing from DDL text: the set of
/// outbound FK constraints on the five FK-holding tables must be identical
/// before and after the principals migration.
#[tokio::test]
async fn the_principals_migration_leaves_the_fk_holding_tables_outbound_fks_unchanged() {
    let db = db_before_principals_migration().await;

    let before = fk_constraints_on(&db, FK_HOLDING_TABLES).await;
    for table in FK_HOLDING_TABLES {
        assert!(
            before
                .iter()
                .any(|fk| fk.table == format!("custos.{table}")),
            "the guard must observe at least one outbound FK on custos.{table} \
             before the migration, got: {before:?}"
        );
    }

    db.run_remaining_migrations()
        .await
        .expect("apply principals migration");

    let after = fk_constraints_on(&db, FK_HOLDING_TABLES).await;
    assert_eq!(
        before, after,
        "E4-S1 is additive: the outbound FKs on the FK-holding tables must \
         not be repointed or altered by the principals migration"
    );

    db.teardown().await.expect("teardown");
}

// ---------------------------------------------------------------------------
// PgPrincipalRepo (E4-S1 W3)
// ---------------------------------------------------------------------------

use atlas_custos::entities::principals::{NewPrincipal, PrincipalKind};
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::principals::PrincipalRepo as PrincipalRepoTrait;
use atlas_custos_postgres::repos::principals::PgPrincipalRepo;

fn repo(db: &TestDb) -> PgPrincipalRepo {
    PgPrincipalRepo {
        conn: db.conn().clone(),
    }
}

fn new_principal(kind: PrincipalKind, display_name: &str) -> NewPrincipal {
    NewPrincipal {
        id: PrincipalId::new(),
        kind,
        display_name: display_name.to_string(),
        deactivated_at: None,
    }
}

/// Inserts the `custos.users` mirror row for an existing user principal (the
/// mirror every writer maintains: `users.id = principals.id`), so the
/// principal can serve as an agent's `owner_user_id` FK target.
async fn seed_user_mirror_row(db: &TestDb, principal_id: Uuid, username: &str) {
    exec(
        db,
        &format!(
            "INSERT INTO custos.users \
                (id, username, display_name, email, password_hash, is_root, is_system_admin, \
                 disabled_at, activated_at, created_at, updated_at, principal_id) \
             VALUES ('{principal_id}', '{username}', '{username}', NULL, NULL, false, false, \
                 NULL, now(), now(), now(), '{principal_id}')"
        ),
    )
    .await;
}

/// Inserts an `agent` principal owned by `owner_user_id`. Since the
/// kind/owner CHECK (`m20260919_000056`), an agent principal can only be
/// written together with its owning human user in the same row; the shared
/// `NewPrincipal` repo path is user-principal-only (owner NULL), so agents
/// are seeded here directly, the way `create_agent_principal_in` writes them.
async fn seed_agent_principal(db: &TestDb, owner_user_id: Uuid, display_name: &str) -> Uuid {
    let id = Uuid::now_v7();
    exec(
        db,
        &format!(
            "INSERT INTO custos.principals \
                (id, kind, display_name, deactivated_at, owner_user_id, created_at, updated_at) \
             VALUES ('{id}', 'agent', '{display_name}', NULL, '{owner_user_id}', now(), now())"
        ),
    )
    .await;
    id
}

#[tokio::test]
async fn principal_repo_creates_and_finds_a_principal_by_id() {
    let db = TestDb::create().await.expect("TestDb::create");
    let principal_repo = repo(&db);

    let created = principal_repo
        .create(new_principal(PrincipalKind::User, "Ada"))
        .await
        .expect("create principal");

    let found = principal_repo
        .find_by_id(created.id)
        .await
        .expect("find principal")
        .expect("principal exists");
    assert_eq!(found.id, created.id);
    assert_eq!(found.kind, PrincipalKind::User);
    assert_eq!(found.display_name, "Ada");
    assert_eq!(found.deactivated_at, None);

    let missing = principal_repo
        .find_by_id(PrincipalId::new())
        .await
        .expect("find principal");
    assert!(
        missing.is_none(),
        "an unknown principal id resolves to None"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn principal_repo_finds_principals_by_kind() {
    let db = TestDb::create().await.expect("TestDb::create");
    let principal_repo = repo(&db);

    let ada = principal_repo
        .create(new_principal(PrincipalKind::User, "Ada"))
        .await
        .expect("create user principal");
    seed_user_mirror_row(&db, ada.id.0, "ada").await;
    // The agent's owner is the user principal this fixture already created —
    // never a freshly invented user.
    seed_agent_principal(&db, ada.id.0, "ci-bot").await;

    let users = principal_repo
        .find_by_kind(PrincipalKind::User)
        .await
        .expect("find users");
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].kind, PrincipalKind::User);

    let agents = principal_repo
        .find_by_kind(PrincipalKind::Agent)
        .await
        .expect("find agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].display_name, "ci-bot");

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn principal_repo_rejects_a_duplicate_principal_id() {
    let db = TestDb::create().await.expect("TestDb::create");
    let principal_repo = repo(&db);

    let first = principal_repo
        .create(new_principal(PrincipalKind::User, "Ada"))
        .await
        .expect("create principal");
    let duplicate = NewPrincipal {
        id: first.id,
        // The imposter must be a user principal: since the kind/owner CHECK,
        // an agent-kind insert without an owner fails the CHECK before the
        // unique index is ever consulted, which would mask the duplicate-id
        // contract under test.
        ..new_principal(PrincipalKind::User, "imposter")
    };

    let err = principal_repo
        .create(duplicate)
        .await
        .expect_err("duplicate id must be rejected");
    // The repo layer deliberately passes raw DB errors through (see
    // `atlas_postgres::db_err`); unique-violation classification is the
    // route layer's concern. The contract here is only that the duplicate
    // insert fails instead of overwriting the existing principal.
    assert!(err.to_string().contains("duplicate key"), "got: {err}");

    db.teardown().await.expect("teardown");
}
