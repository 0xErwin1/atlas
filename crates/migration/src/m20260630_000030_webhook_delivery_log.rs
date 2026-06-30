use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260630_000030_webhook_delivery_log"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE webhook_delivery_log (
                id                UUID PRIMARY KEY,
                workspace_id      UUID NOT NULL
                                  REFERENCES workspaces(id) ON DELETE CASCADE,
                subscription_id   UUID NOT NULL
                                  REFERENCES webhook_subscriptions(id) ON DELETE CASCADE,
                outbox_event_id   UUID NOT NULL
                                  REFERENCES events_outbox(id) ON DELETE CASCADE,
                attempt_no        INT  NOT NULL,
                outcome           TEXT NOT NULL
                                  CHECK (outcome IN ('success','failure')),
                status_code       INT  NULL,
                response_snippet  TEXT NULL,
                error             TEXT NULL,
                duration_ms       INT  NULL,
                created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX wdl_subscription_created_idx \
             ON webhook_delivery_log (subscription_id, created_at DESC)",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX wdl_outbox_event_idx \
             ON webhook_delivery_log (outbox_event_id)",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS webhook_delivery_log CASCADE")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260630_000030_webhook_delivery_log");
    }
}
