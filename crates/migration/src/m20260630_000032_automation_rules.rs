use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260630_000032_automation_rules"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE automation_rules (
                id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                workspace_id        UUID NOT NULL
                                    REFERENCES workspaces(id) ON DELETE CASCADE,
                name                TEXT NOT NULL,
                is_active           BOOLEAN NOT NULL DEFAULT true,
                trigger_event_type  TEXT NOT NULL
                                    CHECK (trigger_event_type LIKE 'external.%'),
                trigger_filter      JSONB NULL,
                project_id          UUID NULL
                                    REFERENCES projects(id) ON DELETE CASCADE,
                action_type         TEXT NOT NULL
                                    CHECK (action_type IN ('create_task')),
                action_params       JSONB NOT NULL,
                created_by_user_id  UUID NOT NULL
                                    REFERENCES users(id),
                created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at          TIMESTAMPTZ NULL
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            "CREATE INDEX automation_rules_ws_event_idx \
             ON automation_rules (workspace_id, trigger_event_type) \
             WHERE is_active = true AND deleted_at IS NULL",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS automation_rules CASCADE")
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260630_000032_automation_rules");
    }
}
