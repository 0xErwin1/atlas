//! E11-S8 D-S8-7 / design D2: adds the three principal-column partial
//! indexes the reverse-grant discovery query needs.
//!
//! §0.1 measured every existing index on `permission_grants` and found each
//! one workspace-leading: `permission_grants_uq (workspace_id, user_id,
//! api_key_id, group_id, resource_ref)`, `permission_grants_resource_idx
//! (workspace_id, resource_ref)`, `permission_grants_user_ws_idx
//! (workspace_id, user_id)`, `permission_grants_api_key_ws_idx (workspace_id,
//! api_key_id)`, `permission_grants_group_ws_idx (workspace_id, group_id)
//! WHERE group_id IS NOT NULL`. A cross-workspace `WHERE user_id = $1` (or
//! `api_key_id = $1` / `group_id = $1`) cannot use a workspace-leading B-tree,
//! so this migration is required rather than optional.
//!
//! Each index is partial on its own column being non-null: the
//! `permission_grants_principal_xor` CHECK constraint guarantees exactly one
//! of `user_id`/`api_key_id`/`group_id` is non-null per row, so a partial
//! index keeps each one small and never indexes rows it cannot match.
//!
//! Runs after `m20260830_000051_custos_set_schema`, so `permission_grants`
//! already lives in the `custos` schema; this migration references it
//! schema-qualified. Not `CONCURRENTLY` — Atlas deploys as a single instance
//! whose binary starts only after its migrator has run (same deployment
//! contract as the other Custos-owned migrations in this crate), so no
//! concurrent traffic needs to be protected from an index build lock.
//!
//! `crates/migration` stays byte-frozen (INV-MIGRATION-SCOPE): this index
//! lives in `atlas_custos_postgres`'s own migration list, spliced into
//! `ComposedMigrator`/`ComposedTestMigrator` by `custos_new()`.

use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260906_000052_grant_principal_idx"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "CREATE INDEX permission_grants_user_idx \
             ON custos.permission_grants (user_id) \
             WHERE user_id IS NOT NULL",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX permission_grants_api_key_idx \
             ON custos.permission_grants (api_key_id) \
             WHERE api_key_id IS NOT NULL",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX permission_grants_group_idx \
             ON custos.permission_grants (group_id) \
             WHERE group_id IS NOT NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP INDEX custos.permission_grants_user_idx")
            .await?;
        conn.execute_unprepared("DROP INDEX custos.permission_grants_api_key_idx")
            .await?;
        conn.execute_unprepared("DROP INDEX custos.permission_grants_group_idx")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260906_000052_grant_principal_idx");
    }
}
