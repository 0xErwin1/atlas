use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260628_000027_task_activity_document_mentioned"
    }
}

/// Adds the `document_mentioned` activity verb, recorded when a `[[wikilink]]` to
/// a document first appears in a task description. Widens the activity-kind CHECK
/// whitelist to accept it.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            ALTER TABLE task_activity
                DROP CONSTRAINT IF EXISTS task_activity_kind_check,
                ADD CONSTRAINT task_activity_kind_check CHECK (
                    kind IN (
                        'created', 'moved', 'assigned', 'unassigned',
                        'field_changed', 'reference_added', 'reference_removed',
                        'checklist_added', 'checklist_updated', 'checklist_removed',
                        'checklist_promoted', 'document_mentioned', 'deleted'
                    )
                )
            "#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            ALTER TABLE task_activity
                DROP CONSTRAINT IF EXISTS task_activity_kind_check,
                ADD CONSTRAINT task_activity_kind_check CHECK (
                    kind IN (
                        'created', 'moved', 'assigned', 'unassigned',
                        'field_changed', 'reference_added', 'reference_removed',
                        'checklist_added', 'checklist_updated', 'checklist_removed',
                        'checklist_promoted', 'deleted'
                    )
                )
            "#,
        )
        .await?;

        Ok(())
    }
}
