#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Cross-schema foreign key inventory (design §D3's permitted-FK table,
//! spec "Cross-schema FK inventory holds after the move" requirement).
//!
//! Enumerates every foreign key in a freshly migrated database
//! (`ComposedTestMigrator` = `historical() ++ custos_new() ++ acta_new()`,
//! via `atlas_test_db::TestDb`) whose constrained table and referenced table
//! live in different schemas, and asserts the result set matches
//! `PERMITTED_CROSS_SCHEMA_FKS` exactly — no unexpected addition, no
//! unexpected removal, and in particular no `custos.*` -> `acta.*` edge in
//! either direction (spec Scenario "No new cross-schema FK introduced").
//!
//! The list below was captured by running the same live query this test
//! runs against a fully-migrated test database (post-PR15, all 36 D1 tables
//! moved) rather than transcribed from design §D3's illustrative table,
//! which named a representative subset, not the complete inventory. Two
//! corrections against that design table, both already documented by prior
//! PRs and re-confirmed here:
//!
//! - `purge_operations.commit_audit_id -> custos.security_audit_log` does
//!   not exist live. S3's O1 migration (`m20260830_000050_grant_resource_ref`)
//!   dropped `purge_operations_commit_audit_id_fkey` in its own `up()`,
//!   before PR15 branched (PR15 T15.7's own finding, re-verified here).
//! - `boards`/`tasks`/etc. carry both a `created_by_user_id -> custos.users`
//!   and a `created_by_api_key_id -> custos.api_keys` edge, present on every
//!   one of the fourteen actor-column tables, not just the couple design
//!   §D3 named as examples.
//!
//! One additional edge outside the Acta/Custos pair: `platform.ui_state`
//! (PR9, D4) carries `user_id -> custos.users`, the constraint retaining its
//! pre-rename name `user_ui_state_user_id_fkey`. It is a legitimate
//! platform->custos edge, not an Acta one, and is listed separately below.

use atlas_test_db::TestDb;
use sea_orm::{FromQueryResult, Statement};

#[derive(Debug, FromQueryResult, PartialEq, Eq, PartialOrd, Ord)]
struct CrossSchemaFk {
    constraint_name: String,
    src_schema: String,
    src_table: String,
    src_column: String,
    dst_schema: String,
    dst_table: String,
}

impl CrossSchemaFk {
    fn new(
        constraint_name: &str,
        src_schema: &str,
        src_table: &str,
        src_column: &str,
        dst_schema: &str,
        dst_table: &str,
    ) -> Self {
        Self {
            constraint_name: constraint_name.to_owned(),
            src_schema: src_schema.to_owned(),
            src_table: src_table.to_owned(),
            src_column: src_column.to_owned(),
            dst_schema: dst_schema.to_owned(),
            dst_table: dst_table.to_owned(),
        }
    }
}

/// Every foreign key whose constrained table and referenced table live in
/// different schemas, in a fully-migrated database (all 36 D1 tables in
/// `acta`, `platform.ui_state` in `platform`, the eight Custos tables in
/// `custos`). Discovered live, not guessed — see the module doc for the two
/// corrections against design §D3's illustrative table.
fn permitted_cross_schema_fks() -> Vec<CrossSchemaFk> {
    vec![
        CrossSchemaFk::new(
            "attachments_created_by_api_key_id_fkey",
            "acta",
            "attachments",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "attachments_created_by_user_id_fkey",
            "acta",
            "attachments",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "automation_rules_created_by_user_id_fkey",
            "acta",
            "automation_rules",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "board_columns_created_by_api_key_id_fkey",
            "acta",
            "board_columns",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "board_columns_created_by_user_id_fkey",
            "acta",
            "board_columns",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "boards_created_by_api_key_id_fkey",
            "acta",
            "boards",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "boards_created_by_user_id_fkey",
            "acta",
            "boards",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "comment_attachment_drafts_created_by_api_key_id_fkey",
            "acta",
            "comment_attachment_drafts",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "comment_attachment_drafts_created_by_user_id_fkey",
            "acta",
            "comment_attachment_drafts",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "comments_created_by_api_key_id_fkey",
            "acta",
            "comments",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "comments_created_by_user_id_fkey",
            "acta",
            "comments",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "document_revisions_created_by_api_key_id_fkey",
            "acta",
            "document_revisions",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "document_revisions_created_by_user_id_fkey",
            "acta",
            "document_revisions",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "documents_created_by_api_key_id_fkey",
            "acta",
            "documents",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "documents_created_by_user_id_fkey",
            "acta",
            "documents",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "folders_created_by_api_key_id_fkey",
            "acta",
            "folders",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "folders_created_by_user_id_fkey",
            "acta",
            "folders",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "integration_configs_created_by_user_id_fkey",
            "acta",
            "integration_configs",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "integration_configs_integration_api_key_id_fkey",
            "acta",
            "integration_configs",
            "integration_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "projects_created_by_api_key_id_fkey",
            "acta",
            "projects",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "projects_created_by_user_id_fkey",
            "acta",
            "projects",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "property_definitions_created_by_api_key_id_fkey",
            "acta",
            "property_definitions",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "property_definitions_created_by_user_id_fkey",
            "acta",
            "property_definitions",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "purge_operations_original_actor_user_id_fkey",
            "acta",
            "purge_operations",
            "original_actor_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "saved_searches_owner_api_key_id_fkey",
            "acta",
            "saved_searches",
            "owner_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "saved_searches_owner_user_id_fkey",
            "acta",
            "saved_searches",
            "owner_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "tags_created_by_api_key_id_fkey",
            "acta",
            "tags",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "tags_created_by_user_id_fkey",
            "acta",
            "tags",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "task_activity_created_by_api_key_id_fkey",
            "acta",
            "task_activity",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "task_activity_created_by_user_id_fkey",
            "acta",
            "task_activity",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "task_assignees_assigned_by_api_key_id_fkey",
            "acta",
            "task_assignees",
            "assigned_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "task_assignees_assigned_by_user_id_fkey",
            "acta",
            "task_assignees",
            "assigned_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "task_assignees_assignee_api_key_id_fkey",
            "acta",
            "task_assignees",
            "assignee_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "task_assignees_assignee_user_id_fkey",
            "acta",
            "task_assignees",
            "assignee_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "task_checklist_items_created_by_api_key_id_fkey",
            "acta",
            "task_checklist_items",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "task_checklist_items_created_by_user_id_fkey",
            "acta",
            "task_checklist_items",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "task_references_created_by_api_key_id_fkey",
            "acta",
            "task_references",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "task_references_created_by_user_id_fkey",
            "acta",
            "task_references",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "task_views_owner_api_key_id_fkey",
            "acta",
            "task_views",
            "owner_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "task_views_owner_user_id_fkey",
            "acta",
            "task_views",
            "owner_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "tasks_created_by_api_key_id_fkey",
            "acta",
            "tasks",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "tasks_created_by_user_id_fkey",
            "acta",
            "tasks",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "webhook_subscriptions_created_by_api_key_id_fkey",
            "acta",
            "webhook_subscriptions",
            "created_by_api_key_id",
            "custos",
            "api_keys",
        ),
        CrossSchemaFk::new(
            "webhook_subscriptions_created_by_user_id_fkey",
            "acta",
            "webhook_subscriptions",
            "created_by_user_id",
            "custos",
            "users",
        ),
        CrossSchemaFk::new(
            "workspace_memberships_user_id_fkey",
            "acta",
            "workspace_memberships",
            "user_id",
            "custos",
            "users",
        ),
        // Not an Acta->Custos edge: `platform.ui_state` (PR9, D4) referencing
        // `custos.users`. The constraint kept its pre-rename name from when
        // the table was still `user_ui_state`.
        CrossSchemaFk::new(
            "user_ui_state_user_id_fkey",
            "platform",
            "ui_state",
            "user_id",
            "custos",
            "users",
        ),
    ]
}

async fn live_cross_schema_fks(db: &TestDb) -> Vec<CrossSchemaFk> {
    let mut rows = CrossSchemaFk::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT
            con.conname AS constraint_name,
            ns_src.nspname AS src_schema,
            cls_src.relname AS src_table,
            att.attname AS src_column,
            ns_dst.nspname AS dst_schema,
            cls_dst.relname AS dst_table
        FROM pg_constraint con
        JOIN pg_class cls_src ON cls_src.oid = con.conrelid
        JOIN pg_namespace ns_src ON ns_src.oid = cls_src.relnamespace
        JOIN pg_class cls_dst ON cls_dst.oid = con.confrelid
        JOIN pg_namespace ns_dst ON ns_dst.oid = cls_dst.relnamespace
        JOIN unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord) ON true
        JOIN pg_attribute att
            ON att.attrelid = con.conrelid AND att.attnum = k.attnum
        WHERE con.contype = 'f'
          AND ns_src.nspname <> ns_dst.nspname
        ORDER BY src_schema, src_table, src_column
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query cross-schema foreign keys");

    rows.sort();
    rows
}

/// The live cross-schema FK set matches `permitted_cross_schema_fks()`
/// exactly: no unexpected addition, no unexpected removal (spec Scenario
/// "No new cross-schema FK introduced").
#[tokio::test]
async fn cross_schema_fk_set_matches_the_permitted_inventory_exactly() {
    let db = TestDb::create().await.expect("TestDb::create");

    let mut live = live_cross_schema_fks(&db).await;
    live.sort();

    let mut permitted = permitted_cross_schema_fks();
    permitted.sort();

    let unexpected: Vec<&CrossSchemaFk> =
        live.iter().filter(|fk| !permitted.contains(fk)).collect();
    let missing: Vec<&CrossSchemaFk> = permitted.iter().filter(|fk| !live.contains(fk)).collect();

    assert!(
        unexpected.is_empty() && missing.is_empty(),
        "live cross-schema FK set does not match the permitted inventory exactly.\n\
         Unexpected (live but not permitted): {unexpected:#?}\n\
         Missing (permitted but not live): {missing:#?}"
    );

    db.teardown().await.expect("teardown");
}

/// No Custos table carries a foreign key into an Acta or platform table, in
/// either the live inventory or the permitted list — the direction the
/// design's forbidden-edge rule protects is `acta -> custos`/`platform ->
/// custos` only, never the reverse.
#[tokio::test]
async fn no_custos_table_references_an_acta_or_platform_table() {
    let db = TestDb::create().await.expect("TestDb::create");

    let live = live_cross_schema_fks(&db).await;

    let reverse_edges: Vec<&CrossSchemaFk> =
        live.iter().filter(|fk| fk.src_schema == "custos").collect();

    assert!(
        reverse_edges.is_empty(),
        "found a custos -> non-custos foreign key, which must never exist: {reverse_edges:#?}"
    );

    db.teardown().await.expect("teardown");
}

/// `purge_operations.commit_audit_id -> custos.security_audit_log`, named in
/// design §D3's illustrative table, does not exist live — S3's O1 migration
/// dropped it before PR15 branched (PR15 T15.7's finding, re-confirmed
/// here so the permitted inventory above does not silently drift back to
/// including it).
#[tokio::test]
async fn purge_operations_commit_audit_id_carries_no_live_cross_schema_fk() {
    let db = TestDb::create().await.expect("TestDb::create");

    let live = live_cross_schema_fks(&db).await;

    let commit_audit_edge = live
        .iter()
        .find(|fk| fk.src_table == "purge_operations" && fk.src_column == "commit_audit_id");

    assert!(
        commit_audit_edge.is_none(),
        "expected no live cross-schema FK on purge_operations.commit_audit_id, found: {commit_audit_edge:?}"
    );

    db.teardown().await.expect("teardown");
}
