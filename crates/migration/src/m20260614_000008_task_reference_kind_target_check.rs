use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260614_000008_task_reference_kind_target_check"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            ALTER TABLE task_references
                ADD CONSTRAINT task_references_kind_target_check
                    CHECK (
                        (kind = 'spec' AND target_document_id IS NOT NULL)
                        OR (kind IN ('relates', 'blocks', 'parent') AND target_task_id IS NOT NULL)
                    )
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"ALTER TABLE task_references DROP CONSTRAINT IF EXISTS task_references_kind_target_check"#,
        )
        .await?;

        Ok(())
    }
}
