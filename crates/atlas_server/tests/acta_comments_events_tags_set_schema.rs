#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! S4 PR14 `SET SCHEMA acta` batch 4 migration (design §D1 batch 4, §D3) —
//! fourth of the five Acta SET SCHEMA batches, moving the
//! comments/events/tags-group tables.

mod support;

use std::time::Duration;

use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::events::DomainEvent;
use atlas_acta::entities::events::TaskCreatedPayload;
use atlas_acta::ids::BoardId;
use atlas_acta::ids::ColumnId;
use atlas_acta::ids::ProjectId;
use atlas_acta::ids::TaskId;
use atlas_server::live::LiveEventHub;
use atlas_server::persistence::migrator::ComposedMigrator;
use atlas_server::persistence::repos::PgOutboxRepo;
use sea_orm::TransactionTrait;
use sea_orm::{FromQueryResult, Statement};
use sea_orm_migration::prelude::MigratorTrait;
use tokio::sync::watch;

const ACTA_COMMENTS_EVENTS_TAGS_TABLES: &[&str] = &[
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
];

/// All eleven comments/events/tags-group tables must live in the `acta`
/// schema after the migration; `workspaces`/`boards` (PR11/PR13, already
/// moved) stay `acta`, and `purge_operations` (batch 5, still unbatched as of
/// this PR) stays in `public`.
#[tokio::test]
async fn comments_events_tags_group_tables_move_to_the_acta_schema() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        table_name: String,
        table_schema: String,
    }

    let all_names: Vec<String> = ACTA_COMMENTS_EVENTS_TAGS_TABLES
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

    for table in ACTA_COMMENTS_EVENTS_TAGS_TABLES
        .iter()
        .chain(["workspaces", "boards"].iter())
    {
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
/// recreating it, so every inbound and outbound foreign key must survive
/// unchanged and still resolve to the moved table. Picks five representative
/// FKs (discovered live against a fully-migrated test database, not
/// guessed): `events_outbox_workspace_id_fkey` (internal to this batch),
/// `comments_task_id_fkey` (this batch referencing PR13's already-moved
/// `acta.tasks`), `webhook_delivery_log_outbox_event_id_fkey` (a live edge
/// between two tables both moved by this same batch), and the two FKs the
/// brief enumerates by name —
/// `integration_configs_integration_api_key_id_fkey` and
/// `integration_configs_created_by_user_id_fkey` (both cross-schema edges to
/// `custos.api_keys`/`custos.users`) — rather than re-asserting every
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
              'events_outbox_workspace_id_fkey',
              'comments_task_id_fkey',
              'webhook_delivery_log_outbox_event_id_fkey',
              'integration_configs_integration_api_key_id_fkey',
              'integration_configs_created_by_user_id_fkey'
          )
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_constraint for representative foreign keys");

    let expectations = [
        (
            "events_outbox_workspace_id_fkey",
            "acta",
            "events_outbox",
            "acta",
            "workspaces",
        ),
        ("comments_task_id_fkey", "acta", "comments", "acta", "tasks"),
        (
            "webhook_delivery_log_outbox_event_id_fkey",
            "acta",
            "webhook_delivery_log",
            "acta",
            "events_outbox",
        ),
        (
            "integration_configs_integration_api_key_id_fkey",
            "acta",
            "integration_configs",
            "custos",
            "api_keys",
        ),
        (
            "integration_configs_created_by_user_id_fkey",
            "acta",
            "integration_configs",
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
        applied.contains(&"m20260904_000056_acta_comments_events_tags_set_schema".to_string()),
        "expected the SET SCHEMA migration to be part of the applied set"
    );

    db.teardown().await;
}

/// The permanent regression form of this PR's PL/pgSQL function-body audit
/// (mirrors T12.1/`acta_documents_set_schema.rs`): queries `pg_proc` on a
/// live, fully-migrated database and asserts no application-authored
/// `plpgsql` routine's body contains an unqualified reference to any of the
/// eleven comments/events/tags-group tables. Filters out `pg_catalog`/
/// `information_schema` (built-in) and non-`plpgsql` routines (the
/// `pgvector` extension's C-language support functions never reference
/// application tables by name). `atlas_notify_event()` itself is exempt from
/// this pattern match, not from the audit: its body reads only
/// `NEW.payload` and names no table at all, so it can never trip this check
/// in the first place.
#[tokio::test]
async fn no_plpgsql_routine_references_a_comments_events_tags_group_table_unqualified() {
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
        for table in ACTA_COMMENTS_EVENTS_TAGS_TABLES {
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

/// Special-attention item from the brief: `events_outbox` carries the only
/// live application trigger in the database, `events_outbox_notify`, firing
/// `atlas_notify_event()` (reads `NEW.payload`, calls `pg_notify`). A
/// trigger binds to its table by OID, exactly like a foreign key, so `SET
/// SCHEMA` should carry it across unchanged — this test proves that live,
/// on two levels: (a) `pg_trigger`/`pg_class`/`pg_namespace` show the
/// trigger still bound to `acta.events_outbox` and enabled, and (b) a
/// committed insert through `PgOutboxRepo::insert_in` still fires a live
/// `NOTIFY` delivered end to end to a `LISTEN` subscriber, exercising the
/// exact same path `live_events.rs` already covers pre-move.
#[tokio::test]
async fn events_outbox_notify_trigger_survives_the_schema_move_and_still_fires() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct TriggerRow {
        tgname: String,
        relname: String,
        nspname: String,
        tgenabled: String,
    }

    let triggers = TriggerRow::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT t.tgname, c.relname, n.nspname, t.tgenabled::text
        FROM pg_trigger t
        JOIN pg_class c ON c.oid = t.tgrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE t.tgname = 'events_outbox_notify'
          AND NOT t.tgisinternal
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_trigger for events_outbox_notify");

    let trigger = triggers
        .first()
        .expect("events_outbox_notify trigger must still exist after the schema move");
    assert_eq!(trigger.tgname, "events_outbox_notify");
    assert_eq!(
        trigger.nspname, "acta",
        "events_outbox_notify must be bound to acta.events_outbox by OID after the move"
    );
    assert_eq!(trigger.relname, "events_outbox");
    assert_ne!(
        trigger.tgenabled, "D",
        "events_outbox_notify must not be disabled after the move"
    );

    // Live fire proof: insert through the same repo path production code
    // uses, and confirm a LISTEN subscriber receives the NOTIFY.
    let (ws, user) = support::seed_workspace(&db, "acta-comments-events-tags-trigger").await;
    let ctx = WorkspaceCtx::new(
        ws.id,
        Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
    );

    let pool = db.conn().get_postgres_connection_pool().clone();
    let hub = LiveEventHub::new(16);
    let mut subscriber = hub.subscribe();

    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = tokio::spawn(atlas_server::live::run_listener(pool, hub, shutdown_rx));

    // NOTIFY only reaches a session that has already issued LISTEN, and the
    // event fires on commit. Give the listener a moment to subscribe before
    // the committing insert so the notification is delivered, not missed.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let event = DomainEvent::TaskCreated(TaskCreatedPayload {
        task_id: TaskId::new(),
        title: "Trigger-move live task".into(),
        project_id: ProjectId::new(),
        board_id: BoardId::new(),
        column_id: ColumnId::new(),
    });
    let event_type = event.event_type();

    let txn = db.conn().begin().await.expect("begin");
    PgOutboxRepo::insert_in(&txn, &ctx, None, None, event)
        .await
        .expect("insert_in");
    txn.commit().await.expect("commit");

    let received = tokio::time::timeout(Duration::from_secs(5), subscriber.recv())
        .await
        .expect("live event within timeout")
        .expect("broadcast recv");

    assert_eq!(
        received.workspace_id, ws.id.0,
        "the trigger's NOTIFY payload must still carry the workspace id after the move"
    );
    assert_eq!(received.event_type, event_type);

    handle.abort();
    db.teardown().await;
}
