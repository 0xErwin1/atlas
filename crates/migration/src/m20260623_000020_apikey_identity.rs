use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260623_000020_apikey_identity"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Step 1: add the `type` column with a default and a check constraint.
        conn.execute_unprepared(
            "ALTER TABLE api_keys ADD COLUMN type TEXT NOT NULL DEFAULT 'agent' \
             CONSTRAINT api_keys_type_check CHECK (type IN ('agent','cli','bot','integration'))",
        )
        .await?;

        // Step 2: back-fill workspace-scope permission_grants for every non-revoked key that
        // already has a workspace_id and does not yet have a workspace-scope grant.
        // This preserves access after the FK-based gate is removed, and also fixes Bug 2
        // for the live dogfooding key.
        //
        // Ordering: grants MUST be inserted before workspace_id is made nullable so the
        // old FK-based gate keeps working for the duration of this migration transaction.
        conn.execute_unprepared(
            r#"
            INSERT INTO permission_grants
                (id, workspace_id, api_key_id, role, created_by_user_id, created_at, updated_at)
            SELECT
                gen_random_uuid(),
                k.workspace_id,
                k.id,
                'editor',
                k.created_by_user_id,
                now(),
                now()
            FROM api_keys k
            WHERE k.workspace_id IS NOT NULL
              AND k.revoked_at IS NULL
              AND NOT EXISTS (
                SELECT 1 FROM permission_grants g
                WHERE g.api_key_id = k.id
                  AND g.workspace_id = k.workspace_id
                  AND num_nonnulls(g.project_id, g.folder_id, g.document_id, g.board_id) = 0
              )
            "#,
        )
        .await?;

        // Step 3: relax the NOT NULL constraint on workspace_id. Existing rows keep their
        // non-null value; new keys created after this migration will insert NULL.
        conn.execute_unprepared("ALTER TABLE api_keys ALTER COLUMN workspace_id DROP NOT NULL")
            .await?;

        // Drop the workspace index — it was a hot-path lookup for the old FK-based gate
        // and is no longer needed.
        conn.execute_unprepared("DROP INDEX IF EXISTS api_keys_workspace_idx")
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Re-add the index before restoring NOT NULL.
        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS api_keys_workspace_idx ON api_keys (workspace_id)",
        )
        .await?;

        // Restore NOT NULL. This will fail if any row has workspace_id = NULL (acceptable
        // for a development/staging roll-back; document this in the PR).
        conn.execute_unprepared("ALTER TABLE api_keys ALTER COLUMN workspace_id SET NOT NULL")
            .await?;

        // Drop the type column.
        conn.execute_unprepared("ALTER TABLE api_keys DROP COLUMN IF EXISTS type")
            .await?;

        // The back-filled permission_grants rows are intentionally left in place on rollback:
        // they are data (not schema) and deleting them risks access loss if the rollback
        // is applied to a live database.

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260623_000020_apikey_identity");
    }
}
