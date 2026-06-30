use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260630_000029_webhook_subscriptions"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE webhook_subscriptions (
                id                      UUID PRIMARY KEY,
                workspace_id            UUID NOT NULL
                                        REFERENCES workspaces(id) ON DELETE CASCADE,
                target_url              TEXT NOT NULL,
                event_types             TEXT[] NOT NULL,
                scope_type              TEXT NOT NULL
                                        CHECK (scope_type IN ('workspace','project','board')),
                scope_id                UUID NULL,
                encrypted_secret        BYTEA NOT NULL,
                secret_nonce            BYTEA NOT NULL,
                is_active               BOOLEAN NOT NULL DEFAULT true,
                label                   TEXT NULL,
                created_by_user_id      UUID NULL
                                        REFERENCES users(id) ON DELETE SET NULL,
                created_by_api_key_id   UUID NULL
                                        REFERENCES api_keys(id) ON DELETE SET NULL,
                created_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at              TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at              TIMESTAMPTZ NULL,
                CONSTRAINT ws_event_types_nonempty
                    CHECK (array_length(event_types, 1) >= 1),
                CONSTRAINT ws_scope_workspace_has_no_id
                    CHECK ((scope_type = 'workspace') = (scope_id IS NULL)),
                CONSTRAINT ws_creator_exactly_one
                    CHECK (num_nonnulls(created_by_user_id, created_by_api_key_id) = 1)
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX ws_workspace_active_idx \
             ON webhook_subscriptions (workspace_id) \
             WHERE is_active = true AND deleted_at IS NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS webhook_subscriptions CASCADE")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260630_000029_webhook_subscriptions");
    }
}
