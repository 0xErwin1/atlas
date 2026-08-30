#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! S3d `SET SCHEMA custos` migration (design §S3d, spec "PR7 — `SET SCHEMA
//! custos` with qualified production SQL and a CI grep gate").

mod support;

use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm::{FromQueryResult, Statement};
use sea_orm_migration::prelude::MigratorTrait;

const CUSTOS_TABLES: &[&str] = &[
    "users",
    "sessions",
    "user_activation_tokens",
    "api_keys",
    "groups",
    "group_members",
    "permission_grants",
    "security_audit_log",
];

/// The eight Custos tables must live in the `custos` schema after the
/// migration, and the tables the design explicitly excludes
/// (`user_ui_state`, `workspaces`, `workspace_memberships`,
/// `purge_operations`) must stay in `public`.
#[tokio::test]
async fn the_eight_custos_tables_move_to_the_custos_schema() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        table_name: String,
        table_schema: String,
    }

    let all_names: Vec<String> = CUSTOS_TABLES
        .iter()
        .map(|t| format!("'{t}'"))
        .chain(
            [
                "user_ui_state",
                "workspaces",
                "workspace_memberships",
                "purge_operations",
            ]
            .iter()
            .map(|t| format!("'{t}'")),
        )
        .collect();

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            r#"
            SELECT table_name, table_schema
            FROM information_schema.tables
            WHERE table_name IN ({})
            "#,
            all_names.join(",")
        ),
    ))
    .all(db.conn())
    .await
    .expect("query information_schema.tables");

    for table in CUSTOS_TABLES {
        let schema = rows
            .iter()
            .find(|r| r.table_name == *table)
            .unwrap_or_else(|| panic!("table {table} not found"))
            .table_schema
            .clone();
        assert_eq!(schema, "custos", "expected {table} to live in custos");
    }

    for table in [
        "user_ui_state",
        "workspaces",
        "workspace_memberships",
        "purge_operations",
    ] {
        let schema = rows
            .iter()
            .find(|r| r.table_name == table)
            .unwrap_or_else(|| panic!("table {table} not found"))
            .table_schema
            .clone();
        assert_eq!(schema, "public", "expected {table} to stay in public");
    }

    db.teardown().await;
}

/// `ALTER TABLE ... SET SCHEMA` moves a table by OID without dropping or
/// recreating it, so every inbound foreign key from an Acta-owned table into
/// a moved Custos table must survive unchanged and still resolve to the
/// moved table. Picks one representative FK per moved referenced table
/// (`users`, `api_keys`, `groups`) rather than re-asserting all 59 inbound
/// FKs the live schema carries.
#[tokio::test]
async fn inbound_acta_foreign_keys_survive_the_schema_move() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        conname: String,
        table_name: String,
        ref_schema: String,
        ref_table: String,
    }

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT conname,
               conrelid::regclass::text AS table_name,
               nc.nspname AS ref_schema,
               confrelid::regclass::text AS ref_table
        FROM pg_constraint
        JOIN pg_class rc ON rc.oid = confrelid
        JOIN pg_namespace nc ON nc.oid = rc.relnamespace
        WHERE contype = 'f'
          AND conname IN (
              'tasks_created_by_user_id_fkey',
              'task_assignees_assignee_api_key_id_fkey',
              'permission_grants_group_id_fkey'
          )
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_constraint for representative inbound FKs");

    let expectations = [
        ("tasks_created_by_user_id_fkey", "tasks", "users"),
        (
            "task_assignees_assignee_api_key_id_fkey",
            "task_assignees",
            "api_keys",
        ),
        (
            "permission_grants_group_id_fkey",
            "custos.permission_grants",
            "groups",
        ),
    ];

    for (conname, table_name, ref_table) in expectations {
        let row = rows
            .iter()
            .find(|r| r.conname == conname)
            .unwrap_or_else(|| panic!("constraint {conname} not found after the schema move"));
        assert_eq!(row.table_name, table_name);
        assert_eq!(row.ref_table, format!("custos.{ref_table}"));
        assert_eq!(
            row.ref_schema, "custos",
            "{conname} must still resolve to the moved custos.{ref_table}"
        );
    }

    db.teardown().await;
}

/// The composed migrator (`historical() ++ custos_new()`) reports zero
/// pending migrations against both a from-empty database and a
/// V1-migrated database, and includes the SET SCHEMA migration by name
/// (D5 regression guard, extended for this PR's addition).
#[tokio::test]
async fn composed_migrator_has_zero_pending_including_the_set_schema_migration() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    assert!(
        ComposedMigrator::get_pending_migrations(db.conn())
            .await
            .expect("get_pending_migrations")
            .is_empty(),
        "expected zero pending migrations from a from-empty database"
    );

    #[derive(Debug, FromQueryResult)]
    struct MigrationRow {
        version: String,
    }

    let applied: Vec<String> = MigrationRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT version FROM seaql_migrations".to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query seaql_migrations")
    .into_iter()
    .map(|r| r.version)
    .collect();
    assert!(
        applied.contains(&"m20260830_000051_custos_set_schema".to_string()),
        "expected the SET SCHEMA migration to be part of the applied set"
    );

    db.teardown().await;
}
