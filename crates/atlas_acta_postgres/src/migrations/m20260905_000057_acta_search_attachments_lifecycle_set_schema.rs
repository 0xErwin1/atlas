//! S4 PR15: moves the four search/attachments/lifecycle-group tables —
//! `search_embeddings`, `search_index_queue`, `purge_operations`,
//! `purge_operation_digests` — into the `acta` schema via `ALTER TABLE ...
//! SET SCHEMA`, one statement per table (design §D1 batch 5, §D3).
//!
//! This is the fifth and final of five Acta `SET SCHEMA` batches, ordered
//! after `m20260904_000056_acta_comments_events_tags_set_schema` in
//! `acta_new()`. After this migration lands, every table in the D1 36-table
//! Acta inventory lives in `acta.*` (or `platform.ui_state` for
//! `user_ui_state`, PR9); none remain in `public.*`.
//!
//! **Gate-before-move discipline (R8, PR10)**: `search_embeddings` and
//! `search_index_queue` have no obvious Acta-branded name, but PR10's
//! `CLASSIFIED_ACTA_TABLES`
//! (`crates/atlas_acta_postgres/tests/r8_classification_gate.rs`) already
//! records both as Acta-owned — each carries `workspace_id UUID NOT NULL
//! REFERENCES workspaces(id) ON DELETE CASCADE` and a `resource_kind` column
//! CHECKed to `('document', 'task')`, both Acta-owned vocabularies, with no
//! `Module` type to give them a neutral home. This migration only moves what
//! PR10 already classified; it introduces no new classification.
//!
//! **Vector index (special attention)**: `search_embeddings_ann_idx` is an
//! IVFFLAT index (`USING ivfflat (embedding vector_cosine_ops) WITH (lists =
//! 100)`, `crates/migration/src/m20260708_000039_search_embeddings.rs`), not
//! HNSW. `SET SCHEMA` moves a table by OID without dropping or recreating
//! it, and an index is bound to its table the same way, so
//! `search_embeddings_ann_idx` (along with `search_embeddings_pkey`,
//! `search_embeddings_workspace_resource_idx`,
//! `search_embeddings_model_dimensions_stale_idx`, and the compound unique
//! index on `(workspace_id, resource_kind, resource_id, source_field,
//! chunk_ordinal, model, dimensions)`) survives the move unchanged. The
//! per-batch regression test in
//! `acta_search_attachments_lifecycle_set_schema.rs` asserts, against a live
//! database, that `pg_indexes` still reports `search_embeddings_ann_idx`
//! under `acta.search_embeddings` with its `ivfflat`/`vector_cosine_ops`
//! definition intact, and that `PgSemanticSearchRepo::search` (the
//! ANN-driven similarity query) still returns correct results reading from
//! `acta.search_embeddings` post-move.
//!
//! **FK-intact proof, corrected against a live query (mirrors PR11's T11.7
//! deviation)**: design §D3's cross-schema FK table lists
//! `purge_operations.commit_audit_id → security_audit_log` as a pre-existing,
//! expected FK surviving this move. A live query against the current
//! (pre-PR15) database shows that FK constraint
//! (`purge_operations_commit_audit_id_fkey`) does **not** exist —
//! `m20260830_000050_grant_resource_ref` (S3's O1 migration) already dropped
//! it in its `up()`, the same class of stale premise PR11 found and
//! documented for `workspaces`'/`workspace_memberships`' Custos-side inbound
//! FKs. The FKs that actually exist today, confirmed live:
//! `purge_operations_original_actor_user_id_fkey → custos.users` (the
//! cross-schema Acta→Custos edge that does survive, per spec's carve-out for
//! "an Acta table referencing a Custos table that already existed before
//! S4"), `purge_operations_workspace_id_fkey → acta.workspaces` (internal,
//! already-moved by PR11), and
//! `purge_operation_digests_operation_id_fkey → purge_operations` (internal
//! to this batch, both tables move together so the FK's target schema
//! changes in lockstep with its own).
//!
//! **PL/pgSQL function-body audit (mirrors T12.1)**: queried `pg_proc` on a
//! live, fully-migrated test database for any routine referencing
//! `search_embeddings`, `search_index_queue`, `purge_operations`, or
//! `purge_operation_digests` by name — zero offending routines found. The
//! only application-authored routine in the entire migration history remains
//! `atlas_notify_event()` / `events_outbox_notify`, unrelated to these four
//! tables.
//!
//! `SET SCHEMA` moves a table without dropping or recreating it, so every
//! index, check, and foreign key survives unchanged — Postgres binds a
//! foreign key (and an index) to its target's OID, not its qualified name.
//!
//! No `search_path` change accompanies this migration (mirrors every prior
//! `SET SCHEMA` migration in this chain): every caller must qualify its own
//! SQL with `acta.`, which is what the generalized `schema_qualification_gate`
//! (design §D5) enforces from PR11 onward, now extended to cover these four
//! tables too.
//!
//! Deployment contract: like every other `SET SCHEMA` migration in this
//! chain, this is a single stop-the-world migration. Atlas deploys as a
//! single instance whose binary starts only after its migrator has run, so
//! no old/new binary ever runs concurrently against a mixed schema; a
//! rollback must pair with this migration's `down()`, which moves all four
//! tables back to `public` unchanged (`SET SCHEMA public`, clean — no data
//! transformation, every index, check, and foreign key survives regardless
//! of direction).

use sea_orm_migration::prelude::*;

const ACTA_SEARCH_ATTACHMENTS_LIFECYCLE_TABLES: &[&str] = &[
    "search_embeddings",
    "search_index_queue",
    "purge_operations",
    "purge_operation_digests",
];

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260905_000057_acta_search_attachments_lifecycle_set_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_SEARCH_ATTACHMENTS_LIFECYCLE_TABLES {
            conn.execute_unprepared(&format!("ALTER TABLE {table} SET SCHEMA acta"))
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_SEARCH_ATTACHMENTS_LIFECYCLE_TABLES {
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
            "m20260905_000057_acta_search_attachments_lifecycle_set_schema"
        );
    }
}
