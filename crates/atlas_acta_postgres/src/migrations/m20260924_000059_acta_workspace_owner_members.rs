//! V2-E7 S4: workspace owner metadata and the membership projection
//! (ACTA-WS-2, ACTA-WS-4).
//!
//! `acta.workspaces.owner_principal_id` names the principal the V1 `owner`
//! membership maps to; it is nullable because existing workspaces are not
//! backfilled here. `acta.workspace_members` is the projection the access
//! dual-write maintains beside `workspace_memberships`: one row per
//! `(workspace_id, principal_id)` with the V1 role and the write that
//! produced it. Principal ids are FK-free on purpose: the projection
//! outlives V1's `users` join once the cutover reads it.

use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260924_000059_acta_workspace_owner_members"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            "ALTER TABLE acta.workspaces ADD COLUMN IF NOT EXISTS owner_principal_id UUID NULL",
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TABLE IF NOT EXISTS acta.workspace_members (
                workspace_id   UUID NOT NULL,
                principal_id   UUID NOT NULL,
                role           TEXT NOT NULL
                               CHECK (role IN ('owner', 'admin', 'member')),
                source         TEXT NOT NULL,
                updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
                PRIMARY KEY (workspace_id, principal_id)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS workspace_members_principal_id_idx \
             ON acta.workspace_members (principal_id)",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS acta.workspace_members CASCADE")
            .await?;

        conn.execute_unprepared(
            "ALTER TABLE acta.workspaces DROP COLUMN IF EXISTS owner_principal_id",
        )
        .await?;

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
            "m20260924_000059_acta_workspace_owner_members"
        );
    }
}
