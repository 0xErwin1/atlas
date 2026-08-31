#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! S4 PR12 `SET SCHEMA acta` batch 2 migration (design §D1 batch 2, §D3,
//! §D5) — second of the five Acta SET SCHEMA batches, moving the
//! documents-group tables.

mod support;

use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm::{FromQueryResult, Statement};
use sea_orm_migration::prelude::MigratorTrait;

const ACTA_DOCUMENTS_TABLES: &[&str] = &[
    "property_definitions",
    "projects",
    "folders",
    "documents",
    "document_revisions",
    "document_links",
    "attachments",
    "attachment_write_intents",
    "comment_attachment_drafts",
    "comment_attachment_draft_uploads",
];

/// All ten documents-group tables must live in the `acta` schema after the
/// migration; `workspaces` (PR11), `boards` (PR13, batch 3), and
/// `purge_operations` (PR15, batch 5) have also since moved to `acta`.
#[tokio::test]
async fn documents_group_tables_move_to_the_acta_schema() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        table_name: String,
        table_schema: String,
    }

    let all_names: Vec<String> = ACTA_DOCUMENTS_TABLES
        .iter()
        .chain(["workspaces", "boards", "purge_operations"].iter())
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

    for table in ACTA_DOCUMENTS_TABLES
        .iter()
        .chain(["workspaces", "boards", "purge_operations"].iter())
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
/// guessed — see the PR11 precedent's deviation note about not trusting the
/// brief's constraint-name guesses): `documents_folder_id_fkey` (internal to
/// this batch, both sides moved together), `projects_workspace_id_fkey` (this
/// batch referencing PR11's already-moved `acta.workspaces`),
/// `boards_project_id_fkey` (an Acta table that has since moved to `acta`
/// too, S4 PR13, referencing this batch's now-moved `acta.projects`), and
/// `documents_created_by_user_id_fkey` (the pre-existing, already-qualified
/// Acta→Custos edge to `custos.users`, S3), rather than re-asserting every
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
              'documents_folder_id_fkey',
              'projects_workspace_id_fkey',
              'boards_project_id_fkey',
              'documents_created_by_user_id_fkey'
          )
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_constraint for representative foreign keys");

    let expectations = [
        (
            "documents_folder_id_fkey",
            "acta",
            "documents",
            "acta",
            "folders",
        ),
        (
            "projects_workspace_id_fkey",
            "acta",
            "projects",
            "acta",
            "workspaces",
        ),
        (
            "boards_project_id_fkey",
            "acta",
            "boards",
            "acta",
            "projects",
        ),
        (
            "documents_created_by_user_id_fkey",
            "acta",
            "documents",
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
        applied.contains(&"m20260902_000054_acta_documents_set_schema".to_string()),
        "expected the SET SCHEMA migration to be part of the applied set"
    );

    db.teardown().await;
}

/// T12.1's PL/pgSQL function-body audit (design "Open Questions"): queries
/// `pg_proc`/`information_schema.triggers` on a live, fully-migrated
/// database and asserts no application-authored routine's body contains an
/// unqualified reference to any of the ten documents-group tables. Runs
/// against the real database rather than only reading migration source, so
/// this is a genuine regression guard, not a one-time note: if a future PR
/// ever adds a trigger/function touching these tables without qualifying
/// them, this test catches it. Filters out `pg_catalog`/`information_schema`
/// (built-in) and the `pgvector` extension's C-language functions
/// (`prolang` for `plpgsql` is the only application-authored language in
/// this codebase; C-language entries are the vector/halfvec/sparsevec
/// support functions, never referencing application tables by name).
#[tokio::test]
async fn no_plpgsql_routine_references_a_documents_group_table_unqualified() {
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
        for table in ACTA_DOCUMENTS_TABLES {
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
