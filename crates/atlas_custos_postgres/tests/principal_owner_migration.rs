//! Container-backed tests for the `custos.principals.owner_user_id`
//! migration (`m20260919_000056`) and the live agent-principal create path
//! that must keep the new CHECK satisfied.
//!
//! The back-fill coverage seeds a pre-migration-shaped database (the
//! migration prefix stops right after `m20260918_000055`), inserts an
//! `agent` principal plus its api key the way pre-migration writers left
//! them (no owner column exists yet), then applies the remaining migrations
//! and asserts the principal gained its owner from the key's
//! `created_by_user_id`. This is deliberately NOT run against a freshly
//! migrated database: a fresh database has no agent principals at all, so
//! the back-fill could be a no-op and every assertion would still pass.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use atlas_custos::entities::identity::NewApiKey;
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo};
use atlas_test_db::TestDb;
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

const OWNER_MIGRATION: &str = "m20260919_000056_custos_principal_owner";
const PREDECESSOR_MIGRATION: &str = "m20260918_000055_custos_scope_wire_form";

/// Migration prefix that stops immediately before the owner migration, so a
/// test can seed rows in their pre-migration shape and then apply the
/// remaining migrations to exercise the back-fill against real rows.
fn steps_before_owner_migration() -> u32 {
    assert_eq!(
        steps_through(PREDECESSOR_MIGRATION) + 1,
        steps_through(OWNER_MIGRATION),
        "the owner migration must directly follow its named predecessor"
    );
    steps_through(OWNER_MIGRATION) - 1
}

async fn db_before_owner_migration() -> TestDb {
    TestDb::create_with_migration_steps(Some(steps_before_owner_migration()))
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
        .expect("execute statement");
}

#[derive(Debug, FromQueryResult)]
struct OwnerRow {
    owner_user_id: Option<Uuid>,
}

/// Reads `custos.principals.owner_user_id` for one principal id.
async fn owner_of(db: &TestDb, principal_id: Uuid) -> Option<Uuid> {
    let rows = OwnerRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!("SELECT owner_user_id FROM custos.principals WHERE id = '{principal_id}'"),
    ))
    .all(db.conn())
    .await
    .expect("query principal owner");

    assert_eq!(rows.len(), 1, "principal row must exist");
    rows.into_iter().next().unwrap().owner_user_id
}

/// Seeds one `custos.users` row plus its backing `user` principal (the
/// mirror every writer maintains: `principals.id = users.id`), and returns
/// the user id.
async fn seed_user(db: &TestDb, username: &str) -> Uuid {
    let id = Uuid::now_v7();
    exec(
        db,
        &format!(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at, \
             created_at, updated_at) \
             VALUES ('{id}', 'user', '{username}', NULL, now(), now())"
        ),
    )
    .await;
    exec(
        db,
        &format!(
            "INSERT INTO custos.users (id, username, display_name, email, password_hash, \
             is_root, is_system_admin, disabled_at, activated_at, created_at, updated_at, \
             principal_id) \
             VALUES ('{id}', '{username}', '{username}', NULL, NULL, false, false, \
             NULL, NULL, now(), now(), '{id}')"
        ),
    )
    .await;
    id
}

/// Seeds one pre-migration agent principal plus its api key row, exactly the
/// way every writer before `000056` left them: the principal carries no
/// owner (the column does not exist yet) and the key carries its human
/// creator in `created_by_user_id`. Returns the principal id.
async fn seed_premigration_agent_with_key(db: &TestDb, creator: Uuid, name: &str) -> Uuid {
    let principal_id = Uuid::now_v7();
    let key_id = Uuid::now_v7();
    exec(
        db,
        &format!(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at, \
             created_at, updated_at) \
             VALUES ('{principal_id}', 'agent', '{name}', NULL, now(), now())"
        ),
    )
    .await;
    exec(
        db,
        &format!(
            "INSERT INTO custos.api_keys (id, workspace_id, created_by_user_id, name, \
             token_hash, type, expires_at, last_used_at, revoked_at, created_at, is_global, \
             scopes, principal_id) \
             VALUES ('{key_id}', NULL, '{creator}', '{name}', 'hash-{name}', 'agent', \
             NULL, NULL, NULL, now(), false, '{{}}', '{principal_id}')"
        ),
    )
    .await;
    principal_id
}

#[tokio::test]
async fn premigration_agent_principal_gains_its_owner_from_the_keys_creator() {
    let db = db_before_owner_migration().await;

    let creator = seed_user(&db, "owner-backfill-creator").await;
    let other_creator = seed_user(&db, "owner-backfill-other").await;
    let agent = seed_premigration_agent_with_key(&db, creator, "backfill-agent").await;
    let other_agent =
        seed_premigration_agent_with_key(&db, other_creator, "backfill-agent-2").await;

    db.run_remaining_migrations()
        .await
        .expect("apply remaining migrations");

    assert_eq!(
        owner_of(&db, agent).await,
        Some(creator),
        "the back-fill must take the agent principal's owner from its key's created_by_user_id"
    );
    assert_eq!(
        owner_of(&db, other_agent).await,
        Some(other_creator),
        "each agent principal must be paired with its own key's creator, not an arbitrary user"
    );

    db.teardown().await.expect("teardown test database");
}

#[tokio::test]
async fn user_principals_keep_a_null_owner_after_the_migration() {
    let db = db_before_owner_migration().await;

    let user = seed_user(&db, "owner-backfill-user").await;

    db.run_remaining_migrations()
        .await
        .expect("apply remaining migrations");

    assert_eq!(
        owner_of(&db, user).await,
        None,
        "a user principal must have no owner: the CHECK pairs owner with kind = agent only"
    );

    db.teardown().await.expect("teardown test database");
}

// ---------------------------------------------------------------------------
// The live create path (E4-S3a W2): every writer that mints an agent
// principal must carry the owner, or the CHECK rejects the whole insert.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn creating_an_api_key_sets_its_agent_principals_owner_to_the_creator() {
    let db = TestDb::create().await.expect("TestDb::create");

    let owner = seed_user(&db, "create-path-owner").await;
    let repo = PgApiKeyRepo {
        conn: db.conn().clone(),
    };
    let key = repo
        .create_for_user(
            atlas_custos::ids::UserId(owner),
            NewApiKey {
                name: "create-path-agent".to_string(),
                token_hash: "hash-create-path".to_string(),
                type_: atlas_custos::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: Vec::new(),
            },
        )
        .await
        .expect("create api key");

    let principal_rows = OwnerRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT owner_user_id FROM custos.principals WHERE id = (\
             SELECT principal_id FROM custos.api_keys WHERE id = '{}')",
            key.id.0
        ),
    ))
    .all(db.conn())
    .await
    .expect("query key principal owner");

    assert_eq!(
        principal_rows.len(),
        1,
        "the key's agent principal must exist"
    );

    assert_eq!(
        principal_rows.into_iter().next().unwrap().owner_user_id,
        Some(owner),
        "the live create path must set the agent principal's owner to the key's creator, \
         or the CHECK would reject every new agent principal"
    );

    db.teardown().await.expect("teardown test database");
}

#[tokio::test]
async fn the_check_rejects_an_agent_principal_without_an_owner() {
    let db = TestDb::create().await.expect("TestDb::create");

    let result = db
        .conn()
        .execute_unprepared(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at, \
             created_at, updated_at) \
             VALUES ('00000000-0000-7000-8000-000000000001', 'agent', 'ownerless', NULL, \
             now(), now())",
        )
        .await;

    let error = result.expect_err("an ownerless agent principal must violate the CHECK");
    let message = error.to_string();
    assert!(
        message.contains("custos_principals_kind_owner_check"),
        "the failure must be the kind/owner CHECK, not something else: {message}"
    );

    db.teardown().await.expect("teardown test database");
}

#[tokio::test]
async fn the_check_rejects_a_user_principal_with_an_owner() {
    let db = TestDb::create().await.expect("TestDb::create");

    let user = seed_user(&db, "owner-check-user").await;

    let result = db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO custos.principals (id, kind, display_name, deactivated_at, \
             owner_user_id, created_at, updated_at) \
             VALUES ('00000000-0000-7000-8000-000000000002', 'user', 'owned-user', NULL, \
             '{user}', now(), now())"
        ))
        .await;

    let error = result.expect_err("a user principal with an owner must violate the CHECK");
    let message = error.to_string();
    assert!(
        message.contains("custos_principals_kind_owner_check"),
        "the failure must be the kind/owner CHECK, not something else: {message}"
    );

    db.teardown().await.expect("teardown test database");
}
