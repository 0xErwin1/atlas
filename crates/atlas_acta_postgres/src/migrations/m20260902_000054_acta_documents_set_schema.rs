//! S4 PR12: moves the ten documents-group tables — `property_definitions`,
//! `projects`, `folders`, `documents`, `document_revisions`,
//! `document_links`, `attachments`, `attachment_write_intents`,
//! `comment_attachment_drafts`, `comment_attachment_draft_uploads` — into the
//! `acta` schema via `ALTER TABLE ... SET SCHEMA`, one statement per table
//! (design §D1 batch 2, §D3).
//!
//! This is the second of five Acta `SET SCHEMA` batches, ordered after
//! `m20260901_000053_acta_identity_workspaces_set_schema` in `acta_new()`.
//! The remaining 24 Acta tables (boards/tasks, comments/events/tags,
//! search/attachments/lifecycle) stay in `public` until their own batch
//! migration lands in a later PR.
//!
//! `SET SCHEMA` moves a table without dropping or recreating it, so every
//! index, constraint, and inbound foreign key survives untouched — Postgres
//! binds a foreign key to the referenced table's OID, not its qualified
//! name. `documents` carries a `search_vector TSVECTOR GENERATED ALWAYS AS
//! (...)` column (`crates/migration/src/m20260612_000003_documents.rs`);
//! `GENERATED` expressions are table-local (they read only the row's own
//! columns) and are unaffected by which schema the table lives in, so no
//! extra step is needed for it here.
//!
//! **PL/pgSQL function-body audit (T12.1, design "Open Questions")**: `SET
//! SCHEMA` does not rewrite the body of any function or trigger, so an
//! unqualified reference to a moved table inside a function/trigger body
//! would silently break after the move. Queried `pg_proc`/
//! `information_schema.triggers` on a live, fully-migrated test database
//! (Postgres 17 + pgvector): the only application-authored function/trigger
//! in the entire migration history is `atlas_notify_event()` /
//! `events_outbox_notify` (`crates/migration/src/m20260702_000036_events_outbox_notify.rs`),
//! whose body only reads `NEW.payload` — no reference, qualified or
//! otherwise, to any of the ten documents-group tables. Every other
//! `pg_proc` entry on that database belongs to the `pgvector` extension
//! (`vector`/`halfvec`/`sparsevec` C functions), not application code. Result:
//! **zero offending routines found**; no fix-up migration step is required.
//!
//! No `search_path` change accompanies this migration (mirrors
//! `m20260830_000051_custos_set_schema` and
//! `m20260901_000053_acta_identity_workspaces_set_schema`): every caller
//! must qualify its own SQL with `acta.`, which is what the generalized
//! `schema_qualification_gate` (design §D5) enforces from PR11 onward, now
//! extended to cover these ten tables too.
//!
//! Deployment contract: like every other `SET SCHEMA` migration in this
//! chain, this is a single stop-the-world migration. Atlas deploys as a
//! single instance whose binary starts only after its migrator has run, so
//! no old/new binary ever runs concurrently against a mixed schema; a
//! rollback must pair with this migration's `down()`, which moves all ten
//! tables back to `public` unchanged (`SET SCHEMA public`, clean — no data
//! transformation, every index, check and foreign key survives regardless of
//! direction).

use sea_orm_migration::prelude::*;

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

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260902_000054_acta_documents_set_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_DOCUMENTS_TABLES {
            conn.execute_unprepared(&format!("ALTER TABLE {table} SET SCHEMA acta"))
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_DOCUMENTS_TABLES {
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
            "m20260902_000054_acta_documents_set_schema"
        );
    }
}
