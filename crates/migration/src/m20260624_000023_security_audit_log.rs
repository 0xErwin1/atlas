use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260624_000023_security_audit_log"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE security_audit_log (
                id               UUID PRIMARY KEY,
                workspace_id     UUID NULL
                                 REFERENCES workspaces(id) ON DELETE SET NULL,
                actor_user_id    UUID NULL
                                 REFERENCES users(id) ON DELETE SET NULL,
                actor_api_key_id UUID NULL
                                 REFERENCES api_keys(id) ON DELETE SET NULL,
                action           TEXT NOT NULL,
                target_type      TEXT NOT NULL,
                target_id        UUID NULL,
                metadata         JSONB NOT NULL DEFAULT '{}',
                created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT sal_actor_atmost_one
                    CHECK (num_nonnulls(actor_user_id, actor_api_key_id) <= 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX sal_ws_created_idx \
             ON security_audit_log (workspace_id, created_at DESC, id DESC) \
             WHERE workspace_id IS NOT NULL",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX sal_platform_created_idx \
             ON security_audit_log (created_at DESC, id DESC) \
             WHERE workspace_id IS NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS security_audit_log CASCADE")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260624_000023_security_audit_log");
    }
}
