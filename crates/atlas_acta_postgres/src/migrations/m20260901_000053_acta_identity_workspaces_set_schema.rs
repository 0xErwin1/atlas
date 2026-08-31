//! S4 PR11: moves the two identity/workspaces-batch tables — `workspaces`,
//! `workspace_memberships` — into the `acta` schema via `ALTER TABLE ... SET
//! SCHEMA`, one statement per table (design §D1 batch 1, §D3).
//!
//! This is the first of five Acta `SET SCHEMA` batches, ordered after
//! `m20260831_000052_acta_platform_ui_state` in `acta_new()`. The remaining
//! 34 Acta tables (documents, boards/tasks, comments/events/tags,
//! search/attachments/lifecycle) stay in `public` until their own batch
//! migration lands in a later PR.
//!
//! `SET SCHEMA` moves a table without dropping or recreating it, so every
//! index, constraint, and inbound foreign key survives untouched — Postgres
//! binds a foreign key to the referenced table's OID, not its qualified
//! name. `workspaces` has 30 inbound FKs from other Acta tables (`projects`,
//! `boards`, `comments`, `search_embeddings`, etc., all still `public` in
//! this PR) plus `workspace_memberships`'s own `workspace_id` FK; none of
//! them are re-created here, they keep resolving to the same OID. The four
//! Custos-outbound cascades onto `workspaces` the proposal names
//! (`permission_grants.workspace_id`, `groups.workspace_id`,
//! `api_keys.workspace_id`, `security_audit_log.workspace_id`) were already
//! dropped by S3's `m20260830_000050_grant_resource_ref` (see
//! `dead_cascade_evidence.rs`'s T6.1 proof that no hard delete relies on
//! them) — there is nothing left on the Custos side referencing `workspaces`
//! for this migration to preserve. No `search_path` change accompanies this
//! migration (mirrors `m20260830_000051_custos_set_schema`): every caller
//! must qualify its own SQL with `acta.`, which is what the generalized
//! `schema_qualification_gate` (design §D5) enforces from this PR onward.
//!
//! Deployment contract: like `m20260830_000051_custos_set_schema` and
//! `m20260831_000052_acta_platform_ui_state`, this is a single
//! stop-the-world migration. Atlas deploys as a single instance whose binary
//! starts only after its migrator has run, so no old/new binary ever runs
//! concurrently against a mixed schema; a rollback must pair with this
//! migration's `down()`, which moves both tables back to `public` unchanged
//! (`SET SCHEMA public`, clean — no data transformation, every index, check
//! and foreign key survives regardless of direction).

use sea_orm_migration::prelude::*;

const ACTA_IDENTITY_WORKSPACES_TABLES: &[&str] = &["workspaces", "workspace_memberships"];

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260901_000053_acta_identity_workspaces_set_schema"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("CREATE SCHEMA IF NOT EXISTS acta")
            .await?;

        for table in ACTA_IDENTITY_WORKSPACES_TABLES {
            conn.execute_unprepared(&format!("ALTER TABLE {table} SET SCHEMA acta"))
                .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for table in ACTA_IDENTITY_WORKSPACES_TABLES {
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
            "m20260901_000053_acta_identity_workspaces_set_schema"
        );
    }
}
