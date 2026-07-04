use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260702_000036_events_outbox_notify"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    /// Installs an `AFTER INSERT` trigger on `events_outbox` that publishes the
    /// new row's `payload` on the `atlas_events` `NOTIFY` channel, so an
    /// in-process `LISTEN` consumer can fan events out to live clients without
    /// polling.
    ///
    /// `pg_notify` caps a payload at 8 KB. That limit is safe here because an
    /// event envelope is small metadata (a handful of ids plus a few scalar
    /// fields), never a document body or attachment content.
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE OR REPLACE FUNCTION atlas_notify_event() RETURNS trigger AS $$
            BEGIN
              PERFORM pg_notify('atlas_events', NEW.payload::text);
              RETURN NEW;
            END;
            $$ LANGUAGE plpgsql;
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            CREATE TRIGGER events_outbox_notify AFTER INSERT ON events_outbox
              FOR EACH ROW EXECUTE FUNCTION atlas_notify_event()
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TRIGGER IF EXISTS events_outbox_notify ON events_outbox")
            .await?;

        conn.execute_unprepared("DROP FUNCTION IF EXISTS atlas_notify_event()")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260702_000036_events_outbox_notify");
    }
}
