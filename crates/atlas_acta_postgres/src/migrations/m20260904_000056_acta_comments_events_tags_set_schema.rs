//! S4 PR14: moves the eleven comments/events/tags-group tables — `comments`,
//! `comment_links`, `comment_link_events`, `tags`, `events_outbox`,
//! `webhook_subscriptions`, `webhook_delivery_log`, `automation_rules`,
//! `integration_configs`, `saved_searches`, `task_views` — into the `acta`
//! schema via `ALTER TABLE ... SET SCHEMA`, one statement per table (design
//! §D1 batch 4, §D3).
//!
//! This is the fourth of five Acta `SET SCHEMA` batches, ordered after
//! `m20260903_000055_acta_boards_tasks_set_schema` in `acta_new()`. The
//! remaining four tables (search/attachments/lifecycle) stay in `public`
//! until their own batch migration lands in a later PR.
//!
//! `SET SCHEMA` moves a table without dropping or recreating it, so every
//! index, constraint, and inbound/outbound foreign key survives untouched —
//! Postgres binds a foreign key to the referenced table's OID, not its
//! qualified name. This batch carries the enumerated
//! `integration_configs.integration_api_key_id → custos.api_keys` and
//! `integration_configs.created_by_user_id → custos.users` cross-schema FKs,
//! neither of which are affected by `SET SCHEMA` for the same OID-binding
//! reason.
//!
//! **Special attention — `events_outbox` carries the only live application
//! trigger in the database**: `events_outbox_notify` (`AFTER INSERT ON
//! events_outbox`, `crates/migration/src/m20260702_000036_events_outbox_notify.rs`)
//! fires `atlas_notify_event()`, which reads `NEW.payload` and calls
//! `pg_notify('atlas_events', ...)` for the in-process `LISTEN` consumer
//! (`atlas_server::live::run_listener`). A trigger binds to its table by
//! OID, exactly like a foreign key, so `SET SCHEMA` carries it across
//! unchanged — but this migration does not rely on that fact alone: the
//! per-batch regression test in `acta_comments_events_tags_set_schema.rs`
//! asserts, against a live database, both that `pg_trigger` still reports
//! `events_outbox_notify` as bound to `acta.events_outbox` and enabled, and
//! that inserting a row still fires a live `NOTIFY` delivered to a `LISTEN`
//! subscriber end to end — the same live path `live_events.rs` already
//! exercises through `PgOutboxRepo::insert_in`.
//!
//! **PL/pgSQL function-body audit (mirrors T12.1, design "Open
//! Questions")**: `SET SCHEMA` does not rewrite the body of any function or
//! trigger, so an unqualified reference to a moved table inside a
//! function/trigger body would silently break after the move. Queried
//! `pg_proc`/`information_schema.triggers` on a live, fully-migrated test
//! database (Postgres 17 + pgvector): the only application-authored
//! function/trigger in the entire migration history remains
//! `atlas_notify_event()` / `events_outbox_notify` itself, whose body reads
//! only `NEW.payload` — it references no table by name at all, qualified or
//! otherwise, so the move cannot break its body. Every other `pg_proc` entry
//! on that database belongs to the `pgvector` extension (`vector`/`halfvec`/
//! `sparsevec` C functions), not application code. Result: **zero offending
//! routines found**; no fix-up migration step is required beyond confirming
//! the trigger's own binding survives, which the live proof above does. The
//! permanent regression form of this audit lives in
//! `acta_comments_events_tags_set_schema.rs`'s
//! `no_plpgsql_routine_references_a_comments_events_tags_group_table_unqualified`
//! test.
//!
//! No `search_path` change accompanies this migration (mirrors every prior
//! `SET SCHEMA` migration in this chain): every caller must qualify its own
//! SQL with `acta.`, which is what the generalized `schema_qualification_gate`
//! (design §D5) enforces from PR11 onward, now extended to cover these eleven
//! tables too.
//!
//! Deployment contract: like every other `SET SCHEMA` migration in this
//! chain, this is a single stop-the-world migration. Atlas deploys as a
//! single instance whose binary starts only after its migrator has run, so
//! no old/new binary ever runs concurrently against a mixed schema; a
//! rollback must pair with this migration's `down()`, which moves all eleven
//! tables back to `public` unchanged (`SET SCHEMA public`, clean — no data
//! transformation, every index, check, foreign key, and the `events_outbox`
//! trigger survive regardless of direction).

use sea_orm_migration::prelude::*;

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

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260904_000056_acta_comments_events_tags_set_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_COMMENTS_EVENTS_TAGS_TABLES {
            conn.execute_unprepared(&format!("ALTER TABLE {table} SET SCHEMA acta"))
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_COMMENTS_EVENTS_TAGS_TABLES {
            conn.execute_unprepared(&format!("ALTER TABLE acta.{table} SET SCHEMA public"))
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(
            Migration.name(),
            "m20260904_000056_acta_comments_events_tags_set_schema"
        );
    }
}
