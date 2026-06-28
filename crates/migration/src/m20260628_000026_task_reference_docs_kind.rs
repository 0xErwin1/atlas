use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260628_000026_task_reference_docs_kind"
    }
}

/// Adds the `docs` reference kind: a link from a task to a documentation note
/// that is not its spec. Like `spec`, it targets a document. Both CHECK
/// constraints that enumerate the kinds must be widened: the kind whitelist and
/// the kind-to-target rule.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            ALTER TABLE task_references
                DROP CONSTRAINT IF EXISTS task_references_kind_check,
                ADD CONSTRAINT task_references_kind_check
                    CHECK (kind IN ('relates', 'blocks', 'parent', 'spec', 'docs'))
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE task_references
                DROP CONSTRAINT IF EXISTS task_references_kind_target_check,
                ADD CONSTRAINT task_references_kind_target_check
                    CHECK (
                        (kind IN ('spec', 'docs') AND target_document_id IS NOT NULL)
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
            r#"
            ALTER TABLE task_references
                DROP CONSTRAINT IF EXISTS task_references_kind_target_check,
                ADD CONSTRAINT task_references_kind_target_check
                    CHECK (
                        (kind = 'spec' AND target_document_id IS NOT NULL)
                        OR (kind IN ('relates', 'blocks', 'parent') AND target_task_id IS NOT NULL)
                    )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE task_references
                DROP CONSTRAINT IF EXISTS task_references_kind_check,
                ADD CONSTRAINT task_references_kind_check
                    CHECK (kind IN ('relates', 'blocks', 'parent', 'spec'))
            "#,
        )
        .await?;

        Ok(())
    }
}
