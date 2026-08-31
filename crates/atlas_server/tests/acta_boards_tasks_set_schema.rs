#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! S4 PR13 `SET SCHEMA acta` batch 3 migration (design §D1 batch 3, §D3) —
//! third of the five Acta SET SCHEMA batches, moving the boards/tasks-group
//! tables.

mod support;

use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm::{FromQueryResult, Statement};
use sea_orm_migration::prelude::MigratorTrait;

const ACTA_BOARDS_TASKS_TABLES: &[&str] = &[
    "boards",
    "board_columns",
    "tasks",
    "task_references",
    "task_assignees",
    "task_checklist_items",
    "task_activity",
    "workspace_status_templates",
    "platform_status_templates",
];

/// All nine boards/tasks-group tables must live in the `acta` schema after
/// the migration; `workspaces`/`documents` (PR11/PR12, already moved) and
/// `purge_operations` (PR15, batch 5) also live in `acta`.
#[tokio::test]
async fn boards_tasks_group_tables_move_to_the_acta_schema() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        table_name: String,
        table_schema: String,
    }

    let all_names: Vec<String> = ACTA_BOARDS_TASKS_TABLES
        .iter()
        .chain(["workspaces", "documents", "purge_operations"].iter())
        .map(|t| format!("'{t}'"))
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

    for table in ACTA_BOARDS_TASKS_TABLES
        .iter()
        .chain(["workspaces", "documents", "purge_operations"].iter())
    {
        let schema = rows
            .iter()
            .find(|r| r.table_name == *table)
            .unwrap_or_else(|| panic!("table {table} not found"))
            .table_schema
            .clone();
        assert_eq!(schema, "acta", "expected {table} to live in acta");
    }

    db.teardown().await;
}

/// `ALTER TABLE ... SET SCHEMA` moves a table by OID without dropping or
/// recreating it, so every inbound and outbound foreign key must survive
/// unchanged and still resolve to the moved table. Picks four representative
/// FKs (discovered live against a fully-migrated test database, not
/// guessed): `tasks_board_id_fkey` (internal to this batch, both sides moved
/// together), `task_references_target_document_id_fkey` (this batch
/// referencing PR12's already-moved `acta.documents`),
/// `task_assignees_assignee_api_key_id_fkey` (the live cross-schema edge to
/// `custos.api_keys` the S3b1 revoke-split composition depends on), and
/// `boards_created_by_user_id_fkey` (the pre-existing, already-qualified
/// Acta→Custos edge to `custos.users`), rather than re-asserting every
/// inbound/outbound FK this batch's tables carry.
#[tokio::test]
async fn foreign_keys_survive_the_schema_move() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        conname: String,
        table_schema: String,
        table_name: String,
        ref_schema: String,
        ref_table: String,
    }

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT conname,
               nt.nspname AS table_schema,
               tc.relname AS table_name,
               nc.nspname AS ref_schema,
               rc.relname AS ref_table
        FROM pg_constraint
        JOIN pg_class tc ON tc.oid = conrelid
        JOIN pg_namespace nt ON nt.oid = tc.relnamespace
        JOIN pg_class rc ON rc.oid = confrelid
        JOIN pg_namespace nc ON nc.oid = rc.relnamespace
        WHERE contype = 'f'
          AND conname IN (
              'tasks_board_id_fkey',
              'task_references_target_document_id_fkey',
              'task_assignees_assignee_api_key_id_fkey',
              'boards_created_by_user_id_fkey'
          )
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_constraint for representative foreign keys");

    let expectations = [
        ("tasks_board_id_fkey", "acta", "tasks", "acta", "boards"),
        (
            "task_references_target_document_id_fkey",
            "acta",
            "task_references",
            "acta",
            "documents",
        ),
        (
            "task_assignees_assignee_api_key_id_fkey",
            "acta",
            "task_assignees",
            "custos",
            "api_keys",
        ),
        (
            "boards_created_by_user_id_fkey",
            "acta",
            "boards",
            "custos",
            "users",
        ),
    ];

    for (conname, table_schema, table_name, ref_schema, ref_table) in expectations {
        let row = rows
            .iter()
            .find(|r| r.conname == conname)
            .unwrap_or_else(|| panic!("constraint {conname} not found after the schema move"));
        assert_eq!(row.table_schema, table_schema, "table schema for {conname}");
        assert_eq!(row.table_name, table_name, "table name for {conname}");
        assert_eq!(row.ref_schema, ref_schema, "ref schema for {conname}");
        assert_eq!(row.ref_table, ref_table, "ref table for {conname}");
    }

    db.teardown().await;
}

/// The composed migrator (`historical() ++ custos_new() ++ acta_new()`)
/// reports zero pending migrations against a from-empty database, and
/// includes this PR's SET SCHEMA migration by name (D5 regression guard,
/// extended for this PR's addition).
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
        applied.contains(&"m20260903_000055_acta_boards_tasks_set_schema".to_string()),
        "expected the SET SCHEMA migration to be part of the applied set"
    );

    db.teardown().await;
}

/// The permanent regression form of this PR's PL/pgSQL function-body audit
/// (mirrors T12.1/`acta_documents_set_schema.rs`): queries `pg_proc` on a
/// live, fully-migrated database and asserts no application-authored
/// `plpgsql` routine's body contains an unqualified reference to any of the
/// nine boards/tasks-group tables. Filters out `pg_catalog`/
/// `information_schema` (built-in) and non-`plpgsql` routines (the
/// `pgvector` extension's C-language support functions never reference
/// application tables by name).
#[tokio::test]
async fn no_plpgsql_routine_references_a_boards_tasks_group_table_unqualified() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct FuncRow {
        proname: String,
        prosrc: String,
    }

    let funcs = FuncRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT p.proname, p.prosrc
        FROM pg_proc p
        JOIN pg_namespace n ON n.oid = p.pronamespace
        JOIN pg_language l ON l.oid = p.prolang
        WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
          AND l.lanname = 'plpgsql'
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_proc for plpgsql routines");

    let mut violations = Vec::new();
    for func in &funcs {
        for table in ACTA_BOARDS_TASKS_TABLES {
            for keyword in ["FROM", "JOIN", "INTO", "UPDATE"] {
                let unqualified = format!("{keyword} {table}");
                if func.prosrc.contains(&unqualified) {
                    violations.push(format!("{}: unqualified `{unqualified}`", func.proname));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "found a PL/pgSQL routine with an unqualified reference to a moved table:\n{}",
        violations.join("\n")
    );

    db.teardown().await;
}
