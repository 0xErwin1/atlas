use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260613_000006_document_slug"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(r#"ALTER TABLE documents ADD COLUMN slug TEXT"#)
            .await?;

        conn.execute_unprepared(
            r#"CREATE UNIQUE INDEX documents_workspace_slug_idx
               ON documents (workspace_id, slug)
               WHERE deleted_at IS NULL"#,
        )
        .await?;

        conn.execute_unprepared(
            r#"UPDATE documents
               SET slug = LOWER(
                   REGEXP_REPLACE(
                       REGEXP_REPLACE(title, '[^a-zA-Z0-9]+', '-', 'g'),
                       '^-|-$',
                       '',
                       'g'
                   )
               )
               WHERE slug IS NULL"#,
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        conn.execute_unprepared(r#"DROP INDEX IF EXISTS documents_workspace_slug_idx"#)
            .await?;

        conn.execute_unprepared(r#"ALTER TABLE documents DROP COLUMN IF EXISTS slug"#)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260613_000006_document_slug");
    }
}
