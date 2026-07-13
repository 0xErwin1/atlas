use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260713_000040_comment_freedom"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            ALTER TABLE attachments ADD COLUMN comment_id UUID REFERENCES comments(id) ON DELETE CASCADE;
            ALTER TABLE attachments DROP CONSTRAINT attachments_owner_check;
            ALTER TABLE attachments ADD CONSTRAINT attachments_owner_check
                CHECK (num_nonnulls(document_id, task_id, comment_id) = 1);

            CREATE TABLE attachment_write_intents (
                id UUID PRIMARY KEY,
                digest TEXT NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );
            CREATE UNIQUE INDEX attachment_write_intents_digest_idx
                ON attachment_write_intents (digest);

            CREATE TABLE comment_links (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES workspaces(id),
                comment_id UUID NOT NULL REFERENCES comments(id) ON DELETE CASCADE,
                target_document_id UUID REFERENCES documents(id) ON DELETE CASCADE,
                target_task_id UUID REFERENCES tasks(id) ON DELETE CASCADE,
                target_attachment_id UUID REFERENCES attachments(id) ON DELETE CASCADE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
                CONSTRAINT comment_links_target_check
                    CHECK (num_nonnulls(target_document_id, target_task_id, target_attachment_id) = 1)
            );
            CREATE UNIQUE INDEX comment_links_document_unique
                ON comment_links (comment_id, target_document_id)
                WHERE target_document_id IS NOT NULL;
            CREATE UNIQUE INDEX comment_links_task_unique
                ON comment_links (comment_id, target_task_id)
                WHERE target_task_id IS NOT NULL;
            CREATE UNIQUE INDEX comment_links_attachment_unique
                ON comment_links (comment_id, target_attachment_id)
                WHERE target_attachment_id IS NOT NULL;
            "#,
        )
        .await
        .map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(
            r#"
            DROP TABLE IF EXISTS comment_links CASCADE;
            DROP TABLE IF EXISTS attachment_write_intents CASCADE;
            ALTER TABLE attachments DROP CONSTRAINT IF EXISTS attachments_owner_check;
            ALTER TABLE attachments DROP COLUMN IF EXISTS comment_id;
            ALTER TABLE attachments ADD CONSTRAINT attachments_owner_check
                CHECK (num_nonnulls(document_id, task_id) = 1);
            "#,
        )
        .await
        .map(|_| ())
    }
}
