use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260614_000009_attachment_task_fk"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            ALTER TABLE attachments
                ADD CONSTRAINT attachments_task_id_fkey
                    FOREIGN KEY (task_id) REFERENCES tasks(id)
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"ALTER TABLE attachments DROP CONSTRAINT IF EXISTS attachments_task_id_fkey"#,
        )
        .await?;

        Ok(())
    }
}
