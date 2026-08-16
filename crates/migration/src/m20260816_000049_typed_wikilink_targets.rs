use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260816_000049_typed_wikilink_targets"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        // Typed wikilinks let a document or a task description point at a task
        // or an attachment, which the table could not record: it only had
        // `target_document_id`. The target side becomes polymorphic the same way
        // the source side already is, except that zero targets stays legal — an
        // unresolved link is stored pending, not dropped.
        conn.execute_unprepared(
            r#"ALTER TABLE document_links ADD COLUMN target_task_id UUID REFERENCES tasks(id) ON DELETE CASCADE"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links ADD COLUMN target_attachment_id UUID REFERENCES attachments(id) ON DELETE CASCADE"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"
            ALTER TABLE document_links
                ADD CONSTRAINT document_links_target_check
                    CHECK (num_nonnulls(target_document_id, target_task_id, target_attachment_id) <= 1)
            "#,
        )
        .await?;

        // The old index keyed a source's links by display title alone, which
        // now collides: `[[task:ATL-1|Fix]]` and `[[note:fix-plan|Fix]]` are
        // different links that share a title. Keying by target identity as well
        // keeps the one-row-per-link intent without forbidding that.
        conn.execute_unprepared(r#"DROP INDEX IF EXISTS document_links_source_title_uidx"#)
            .await?;

        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX document_links_source_target_uidx
                ON document_links (
                    COALESCE(source_document_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    COALESCE(source_task_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    COALESCE(target_document_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    COALESCE(target_task_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    COALESCE(target_attachment_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    target_title
                )
            "#,
        )
        .await?;

        conn.execute_unprepared(
            r#"CREATE INDEX document_links_target_task_idx ON document_links (target_task_id) WHERE target_task_id IS NOT NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(r#"DROP INDEX IF EXISTS document_links_target_task_idx"#)
            .await?;

        conn.execute_unprepared(r#"DROP INDEX IF EXISTS document_links_source_target_uidx"#)
            .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links DROP CONSTRAINT IF EXISTS document_links_target_check"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links DROP COLUMN IF EXISTS target_attachment_id"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"ALTER TABLE document_links DROP COLUMN IF EXISTS target_task_id"#,
        )
        .await?;

        // Restoring the pre-typed-link index can fail on rows the new schema
        // allowed, so the reverted table keeps the wider key rather than the
        // migration failing halfway through a rollback.
        conn.execute_unprepared(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS document_links_source_title_uidx
                ON document_links (
                    COALESCE(source_document_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    COALESCE(source_task_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    COALESCE(target_document_id, '00000000-0000-0000-0000-000000000000'::uuid),
                    target_title
                )
            "#,
        )
        .await?;

        Ok(())
    }
}
