use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260630_000031_integration_configs"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE integration_configs (
                id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                workspace_id            UUID NOT NULL
                                        REFERENCES workspaces(id) ON DELETE CASCADE,
                integration             TEXT NOT NULL
                                        CHECK (integration IN ('github')),
                encrypted_secret        BYTEA NOT NULL,
                secret_nonce            BYTEA NOT NULL,
                integration_api_key_id  UUID NOT NULL
                                        REFERENCES api_keys(id) ON DELETE RESTRICT,
                is_active               BOOLEAN NOT NULL DEFAULT true,
                created_by_user_id      UUID NOT NULL
                                        REFERENCES users(id),
                created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at              TIMESTAMPTZ NULL
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE UNIQUE INDEX integration_configs_active_uniq \
             ON integration_configs (workspace_id, integration) \
             WHERE deleted_at IS NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS integration_configs CASCADE")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260630_000031_integration_configs");
    }
}
