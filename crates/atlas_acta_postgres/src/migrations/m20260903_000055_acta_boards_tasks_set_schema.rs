//! S4 PR13: moves the nine boards/tasks-group tables — `boards`,
//! `board_columns`, `tasks`, `task_references`, `task_assignees`,
//! `task_checklist_items`, `task_activity`, `workspace_status_templates`,
//! `platform_status_templates` — into the `acta` schema via
//! `ALTER TABLE ... SET SCHEMA`, one statement per table (design §D1
//! batch 3, §D3).
//!
//! This is the third of five Acta `SET SCHEMA` batches, ordered after
//! `m20260902_000054_acta_documents_set_schema` in `acta_new()`. The
//! remaining 15 Acta tables (comments/events/tags, search/attachments/
//! lifecycle) stay in `public` until their own batch migration lands in a
//! later PR.
//!
//! `SET SCHEMA` moves a table without dropping or recreating it, so every
//! index, constraint, and inbound/outbound foreign key survives untouched —
//! Postgres binds a foreign key to the referenced table's OID, not its
//! qualified name. This batch carries the first live cross-schema FKs this
//! slice introduces into a still-unmoved sibling on the Custos side
//! (`task_assignees.assignee_api_key_id` / `assigned_by_api_key_id` →
//! `custos.api_keys`, plus the `created_by_user_id`/`assignee_user_id`-style
//! columns across this batch → `custos.users`), none of which are affected
//! by `SET SCHEMA` for the same OID-binding reason.
//!
//! **PL/pgSQL function-body audit (mirrors T12.1, design "Open
//! Questions")**: `SET SCHEMA` does not rewrite the body of any function or
//! trigger, so an unqualified reference to a moved table inside a
//! function/trigger body would silently break after the move. Queried
//! `pg_proc`/`information_schema.triggers` on a live, fully-migrated test
//! database (Postgres 17 + pgvector): the only application-authored
//! function/trigger in the entire migration history remains
//! `atlas_notify_event()` / `events_outbox_notify`
//! (`crates/migration/src/m20260702_000036_events_outbox_notify.rs`), which
//! fires on `events_outbox` only and whose body reads only `NEW.payload` —
//! no reference, qualified or otherwise, to any of the nine boards/tasks-group
//! tables. Every other `pg_proc` entry on that database belongs to the
//! `pgvector` extension (`vector`/`halfvec`/`sparsevec` C functions), not
//! application code. Result: **zero offending routines found**; no fix-up
//! migration step is required. The permanent regression form of this audit
//! lives in `acta_boards_tasks_set_schema.rs`'s
//! `no_plpgsql_routine_references_a_boards_tasks_group_table_unqualified`
//! test.
//!
//! No `search_path` change accompanies this migration (mirrors every prior
//! `SET SCHEMA` migration in this chain): every caller must qualify its own
//! SQL with `acta.`, which is what the generalized `schema_qualification_gate`
//! (design §D5) enforces from PR11 onward, now extended to cover these nine
//! tables too.
//!
//! Deployment contract: like every other `SET SCHEMA` migration in this
//! chain, this is a single stop-the-world migration. Atlas deploys as a
//! single instance whose binary starts only after its migrator has run, so
//! no old/new binary ever runs concurrently against a mixed schema; a
//! rollback must pair with this migration's `down()`, which moves all nine
//! tables back to `public` unchanged (`SET SCHEMA public`, clean — no data
//! transformation, every index, check and foreign key survives regardless of
//! direction).

use sea_orm_migration::prelude::*;

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

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260903_000055_acta_boards_tasks_set_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_BOARDS_TASKS_TABLES {
            conn.execute_unprepared(&format!("ALTER TABLE {table} SET SCHEMA acta"))
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_BOARDS_TASKS_TABLES {
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
            "m20260903_000055_acta_boards_tasks_set_schema"
        );
    }
}
