use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260725_000044_platform_status_templates"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            CREATE TABLE platform_status_templates (
                id UUID PRIMARY KEY,
                name TEXT NOT NULL,
                color TEXT,
                position_key TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                deleted_at TIMESTAMPTZ
            )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX platform_status_templates_position_idx
               ON platform_status_templates (position_key)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared("DROP TABLE IF EXISTS platform_status_templates CASCADE")
            .await?;

        Ok(())
    }
}
