use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260630_000028_events_outbox"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE events_outbox (
                id               UUID PRIMARY KEY,
                workspace_id     UUID NOT NULL
                                 REFERENCES workspaces(id) ON DELETE CASCADE,
                event_type       TEXT NOT NULL,
                event_version    INT  NOT NULL,
                source           TEXT NOT NULL DEFAULT 'internal',
                project_id       UUID NULL,
                board_id         UUID NULL,
                aggregate_type   TEXT NOT NULL,
                aggregate_id     UUID NOT NULL,
                payload          JSONB NOT NULL,
                occurred_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
                status           TEXT NOT NULL DEFAULT 'pending'
                                 CHECK (status IN ('pending','delivering','delivered','dead')),
                attempt_count    INT  NOT NULL DEFAULT 0,
                next_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
                locked_until     TIMESTAMPTZ NULL,
                last_error       TEXT NULL,
                created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX eo_pending_next_attempt_idx \
             ON events_outbox (next_attempt_at) \
             WHERE status = 'pending'",
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX eo_delivering_locked_until_idx \
             ON events_outbox (status, locked_until) \
             WHERE status = 'delivering'",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS events_outbox CASCADE")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260630_000028_events_outbox");
    }
}
