#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! S4 PR11 `SET SCHEMA acta` batch 1 migration (design §D1 batch 1, §D3,
//! §D5) — first of the five Acta SET SCHEMA batches.

mod support;

use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm::{FromQueryResult, Statement};
use sea_orm_migration::prelude::MigratorTrait;

const ACTA_IDENTITY_WORKSPACES_TABLES: &[&str] = &["workspaces", "workspace_memberships"];

/// `workspaces` and `workspace_memberships` must live in the `acta` schema
/// after the migration. `documents` (S4 PR12, batch 2) and `boards` (S4 PR13,
/// batch 3) have since moved to `acta` too, so neither is part of this
/// test's "stays public" list anymore; `purge_operations` (batch 5) remains
/// unbatched as of this PR and stays in `public`.
#[tokio::test]
async fn identity_workspaces_tables_move_to_the_acta_schema() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        table_name: String,
        table_schema: String,
    }

    let all_names: Vec<String> = ACTA_IDENTITY_WORKSPACES_TABLES
        .iter()
        .chain(["purge_operations"].iter())
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

    for table in ACTA_IDENTITY_WORKSPACES_TABLES {
        let schema = rows
            .iter()
            .find(|r| r.table_name == *table)
            .unwrap_or_else(|| panic!("table {table} not found"))
            .table_schema
            .clone();
        assert_eq!(schema, "acta", "expected {table} to live in acta");
    }

    let table = "purge_operations";
    let schema = rows
        .iter()
        .find(|r| r.table_name == table)
        .unwrap_or_else(|| panic!("table {table} not found"))
        .table_schema
        .clone();
    assert_eq!(
        schema, "public",
        "expected {table} to stay in public until its own batch lands"
    );

    db.teardown().await;
}

/// `ALTER TABLE ... SET SCHEMA` moves a table by OID without dropping or
/// recreating it, so every inbound foreign key must survive unchanged and
/// still resolve to the moved table. Picks three representative FKs:
/// `workspace_memberships_workspace_id_fkey` (the batch's own internal FK,
/// both sides moved together), `documents_workspace_id_fkey` (originally an
/// Acta-owned table still in `public` referencing the now-moved
/// `acta.workspaces`; `documents` itself has since moved to `acta` too in S4
/// PR12, so this now also proves the PR11 move survives a later batch's
/// migration landing on top of it), and `workspace_memberships_user_id_fkey`
/// (the pre-existing, already-qualified inbound edge to `custos.users`, S3),
/// rather than re-asserting all 31 inbound FKs the live schema carries.
#[tokio::test]
async fn inbound_foreign_keys_survive_the_schema_move() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        conname: String,
        table_schema: String,
        table_name: String,
        ref_schema: String,
        ref_table: String,
    }

    // Schema and name come from explicit pg_class/pg_namespace joins on both
    // sides: `regclass::text` omits any schema on the search_path, so its
    // rendering flips between bare and qualified depending on connection
    // settings and cannot back a stable expectation table.
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
              'workspace_memberships_workspace_id_fkey',
              'documents_workspace_id_fkey',
              'workspace_memberships_user_id_fkey'
          )
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_constraint for representative inbound FKs");

    let expectations = [
        (
            "workspace_memberships_workspace_id_fkey",
            ("acta", "workspace_memberships"),
            ("acta", "workspaces"),
        ),
        (
            "documents_workspace_id_fkey",
            ("acta", "documents"),
            ("acta", "workspaces"),
        ),
        (
            "workspace_memberships_user_id_fkey",
            ("acta", "workspace_memberships"),
            ("custos", "users"),
        ),
    ];

    for (conname, (table_schema, table_name), (ref_schema, ref_table)) in expectations {
        let row = rows
            .iter()
            .find(|r| r.conname == conname)
            .unwrap_or_else(|| panic!("constraint {conname} not found after the schema move"));
        assert_eq!(row.table_schema, table_schema);
        assert_eq!(row.table_name, table_name);
        assert_eq!(row.ref_schema, ref_schema);
        assert_eq!(row.ref_table, ref_table);
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
        applied.contains(&"m20260901_000053_acta_identity_workspaces_set_schema".to_string()),
        "expected the SET SCHEMA migration to be part of the applied set"
    );

    db.teardown().await;
}
