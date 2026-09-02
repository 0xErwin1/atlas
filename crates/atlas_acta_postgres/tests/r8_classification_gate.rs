#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! R8 classification gate (design §D1, spec "Search/attachment table
//! ownership inventory precedes their move" requirement).
//!
//! Enumerates every functional table in a freshly migrated database
//! (`ComposedTestMigrator` = `historical() ++ custos_new() ++ acta_new()`,
//! via `atlas_test_db::TestDb`) and asserts each one is classified into
//! exactly one of four dispositions: `acta`, `custos`, `platform`, or
//! `historical`. An unclassified table — one that appears in `pg_tables` but
//! in none of the four lists below — fails the gate (spec Scenario
//! "Unclassified table blocks the batch"). This turns D1's inventory into
//! something CI enforces rather than a claim in a design document.
//!
//! Zero SET SCHEMA migrations land in this PR: `CLASSIFIED_ACTA_TABLES`
//! records the full D1 36-table inventory. At the time this gate first
//! landed (PR10), every one of those 36 tables still lived in `public`, so
//! the gate temporarily accepted Acta-classified tables in either `public`
//! or `acta`. PR11–PR15 have since moved all 36 tables into `acta` in five
//! batches, so PR16 tightens `Disposition::Acta`'s expected schema down to
//! `acta` only — the permissive `public` fallback is retired now that the
//! move it accommodated is complete, matching the epic-level "no functional
//! table outside an owned schema" success criterion. Custos- and
//! platform-classified tables — already moved by S3 and PR9 respectively —
//! must already live in their owning schema. `seaql_migrations` is the one
//! `historical` table: sea-orm's own bookkeeping table, owned by no product.

use atlas_test_db::TestDb;
use sea_orm::{ConnectionTrait, FromQueryResult, Statement};

/// Where a classified table is expected to live at this point in the
/// migration history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// Owned by Acta, moved into `acta` by the PR11–PR15 SET SCHEMA
    /// batches.
    Acta,
    /// Owned by Custos, moved to `custos` by S3.
    Custos,
    /// Owned by the platform component, moved to `platform` by PR9.
    Platform,
    /// Framework bookkeeping, owned by no product, stays `public`.
    Historical,
}

/// The full D1 36-table Acta inventory, grounded in `crates/migration`'s
/// `CREATE TABLE` statements (design §D1's five SET-SCHEMA batches).
///
/// R8 dispositions, recorded next to the two contested entries:
///
/// - `search_embeddings` (`m20260708_000039`) and `search_index_queue`
///   (`m20260808_000047`) both carry `workspace_id UUID NOT NULL REFERENCES
///   workspaces(id) ON DELETE CASCADE` and a `resource_kind` column CHECKed
///   to `('document', 'task')` — both the FK target and the resource
///   vocabulary are Acta's own. No `Module` type exists for a neutral home
///   (SHELL-CAP Module extraction is out of E2 scope). Acta-owned.
/// - `platform_status_templates` (`m20260725_000044`) has no `workspace_id`
///   column and no FK at all, unlike its sibling `workspace_status_templates`
///   (which does carry `workspace_id`) — but its type already lives in the
///   `atlas_acta` crate, and its route seeds `workspace_status_templates` on
///   board creation (`persistence/repos/boards_tasks.rs::create_board`). It
///   is Acta seed data despite the platform-sounding name; the rename is
///   deferred to E3.
const CLASSIFIED_ACTA_TABLES: &[&str] = &[
    // Batch 1 — identity/workspaces
    "workspaces",
    "workspace_memberships",
    // Batch 2 — documents
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
    // Batch 3 — boards/tasks
    "boards",
    "board_columns",
    "tasks",
    "task_references",
    "task_assignees",
    "task_checklist_items",
    "task_activity",
    "workspace_status_templates",
    "platform_status_templates",
    // Batch 4 — comments/events/tags
    "comments",
    "comment_links",
    "comment_link_events",
    "tags",
    "events_outbox",
    "webhook_subscriptions",
    "webhook_delivery_log",
    "automation_rules",
    "integration_configs",
    "saved_searches",
    "task_views",
    // Batch 5 — search/attachments/lifecycle
    "search_embeddings",
    "search_index_queue",
    "purge_operations",
    "purge_operation_digests",
];

/// The eight Custos-owned tables, moved to `custos` by S3
/// (`m20260830_000051_custos_set_schema`).
const CLASSIFIED_CUSTOS_TABLES: &[&str] = &[
    "users",
    "sessions",
    "user_activation_tokens",
    "api_keys",
    "groups",
    "group_members",
    "permission_grants",
    "security_audit_log",
];

/// Platform-owned tables. `user_ui_state` was moved and renamed to
/// `platform.ui_state` by PR9 (design §D4) — it is genuinely platform-scoped
/// (per-user UI preferences, no workspace or Acta resource reference) and is
/// deliberately absent from `CLASSIFIED_ACTA_TABLES`.
const CLASSIFIED_PLATFORM_TABLES: &[&str] = &["ui_state", "idempotency_keys"];

/// Framework bookkeeping tables, owned by no product.
const CLASSIFIED_HISTORICAL_TABLES: &[&str] = &["seaql_migrations"];

fn classify(table_name: &str) -> Option<Disposition> {
    if CLASSIFIED_ACTA_TABLES.contains(&table_name) {
        return Some(Disposition::Acta);
    }
    if CLASSIFIED_CUSTOS_TABLES.contains(&table_name) {
        return Some(Disposition::Custos);
    }
    if CLASSIFIED_PLATFORM_TABLES.contains(&table_name) {
        return Some(Disposition::Platform);
    }
    if CLASSIFIED_HISTORICAL_TABLES.contains(&table_name) {
        return Some(Disposition::Historical);
    }
    None
}

fn expected_schemas(disposition: Disposition) -> &'static [&'static str] {
    match disposition {
        Disposition::Acta => &["acta"],
        Disposition::Custos => &["custos"],
        Disposition::Platform => &["platform"],
        Disposition::Historical => &["public"],
    }
}

#[derive(Debug, FromQueryResult)]
struct TableRow {
    schemaname: String,
    tablename: String,
}

async fn live_tables(db: &TestDb) -> Vec<TableRow> {
    TableRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT schemaname, tablename
        FROM pg_tables
        WHERE schemaname NOT IN ('pg_catalog', 'information_schema')
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_tables")
}

/// Runs the classification gate over a set of `(schema, table)` rows,
/// returning one violation string per unclassified or misplaced table.
fn classification_violations(rows: &[TableRow]) -> Vec<String> {
    let mut violations = Vec::new();

    for row in rows {
        match classify(&row.tablename) {
            None => violations.push(format!(
                "{}.{}: table is not classified acta/custos/platform/historical \
                 in CLASSIFIED_*_TABLES",
                row.schemaname, row.tablename
            )),
            Some(disposition) => {
                let allowed = expected_schemas(disposition);
                if !allowed.contains(&row.schemaname.as_str()) {
                    violations.push(format!(
                        "{}.{}: classified as {disposition:?} but lives in schema `{}`, \
                         expected one of {allowed:?}",
                        row.schemaname, row.tablename, row.schemaname
                    ));
                }
            }
        }
    }

    violations
}

/// Every functional table in a freshly migrated database is classified
/// acta/custos/platform/historical, and each one currently lives in a
/// schema its disposition permits (spec Scenario "Unclassified table blocks
/// the batch", run in the positive direction).
#[tokio::test]
async fn every_live_table_is_classified() {
    let db = TestDb::create().await.expect("TestDb::create");

    let rows = live_tables(&db).await;
    assert!(
        !rows.is_empty(),
        "expected at least one table in the freshly migrated database"
    );

    let violations = classification_violations(&rows);

    assert!(
        violations.is_empty(),
        "R8 classification gate failed:\n{}",
        violations.join("\n")
    );

    db.teardown().await.expect("teardown");
}

/// An unclassified table — one absent from every `CLASSIFIED_*_TABLES`
/// list — must fail the gate. Proves the gate is not vacuously true by
/// introducing a table the classification map cannot know about ahead of
/// time.
#[tokio::test]
async fn an_unclassified_table_fails_the_gate() {
    let db = TestDb::create().await.expect("TestDb::create");

    db.conn()
        .execute_unprepared("CREATE TABLE public.unclassified_probe_table (id INT)")
        .await
        .expect("create an unclassified probe table");

    let rows = live_tables(&db).await;
    let violations = classification_violations(&rows);

    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("unclassified_probe_table")),
        "expected the gate to flag `unclassified_probe_table` as unclassified, got:\n{}",
        violations.join("\n")
    );

    db.teardown().await.expect("teardown");
}

/// PR16: with `Disposition::Acta` tightened to `&["acta"]` only, an
/// Acta-classified table that turns up in `public` — the schema the
/// permissive pre-tightening gate used to accept — must now fail the gate.
/// Proves the tightening actually narrows the accepted schema set rather
/// than being a no-op comment change.
#[tokio::test]
async fn a_misplaced_acta_table_in_public_fails_the_tightened_gate() {
    let db = TestDb::create().await.expect("TestDb::create");

    // `workspaces` is already correctly classified and lives in `acta`; add
    // a second, wrongly-placed table with the same name in `public` to
    // simulate a regression that reintroduces an Acta table outside `acta`.
    db.conn()
        .execute_unprepared("CREATE TABLE public.workspaces (id INT)")
        .await
        .expect("create a misplaced public.workspaces table");

    let rows = live_tables(&db).await;
    let violations = classification_violations(&rows);

    assert!(
        violations
            .iter()
            .any(|violation| violation.starts_with("public.workspaces:")),
        "expected the tightened gate to flag `public.workspaces` as misplaced, got:\n{}",
        violations.join("\n")
    );

    db.teardown().await.expect("teardown");
}

/// `user_ui_state`/`platform.ui_state` is platform-scoped (moved by PR9),
/// not Acta — it must never appear in `CLASSIFIED_ACTA_TABLES`, and its
/// disposition must resolve to `Platform`.
#[test]
fn platform_ui_state_is_not_classified_as_acta() {
    assert!(
        !CLASSIFIED_ACTA_TABLES.contains(&"user_ui_state"),
        "user_ui_state is platform-scoped and must not appear in CLASSIFIED_ACTA_TABLES"
    );
    assert!(
        !CLASSIFIED_ACTA_TABLES.contains(&"ui_state"),
        "ui_state is platform-scoped and must not appear in CLASSIFIED_ACTA_TABLES"
    );
    assert_eq!(
        classify("ui_state"),
        Some(Disposition::Platform),
        "ui_state must classify as Platform"
    );
}

/// The two R8-contested tables classify as Acta despite `search_embeddings`/
/// `search_index_queue` having no obvious Acta-branded name and
/// `platform_status_templates` having a platform-sounding one.
#[test]
fn the_r8_contested_tables_classify_as_acta() {
    for table in [
        "search_embeddings",
        "search_index_queue",
        "platform_status_templates",
    ] {
        assert_eq!(
            classify(table),
            Some(Disposition::Acta),
            "{table} must classify as Acta per R8"
        );
    }
}

/// No table name is double-classified across two different dispositions —
/// each of the four `CLASSIFIED_*_TABLES` lists is disjoint from the other
/// three.
#[test]
fn no_table_name_is_classified_under_more_than_one_disposition() {
    let all_lists: [(&str, &[&str]); 4] = [
        ("acta", CLASSIFIED_ACTA_TABLES),
        ("custos", CLASSIFIED_CUSTOS_TABLES),
        ("platform", CLASSIFIED_PLATFORM_TABLES),
        ("historical", CLASSIFIED_HISTORICAL_TABLES),
    ];

    for (name_a, list_a) in all_lists {
        for (name_b, list_b) in all_lists {
            if name_a == name_b {
                continue;
            }
            for table in list_a {
                assert!(
                    !list_b.contains(table),
                    "`{table}` appears in both the {name_a} and {name_b} classification lists"
                );
            }
        }
    }
}

/// `CLASSIFIED_ACTA_TABLES` records exactly the D1 36-table inventory — a
/// regression guard against silently dropping or duplicating an entry.
#[test]
fn classified_acta_tables_has_the_full_d1_inventory_count() {
    assert_eq!(
        CLASSIFIED_ACTA_TABLES.len(),
        36,
        "expected the full D1 36-table Acta inventory"
    );
}
