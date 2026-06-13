use sea_orm_migration::prelude::*;

/// Backfills `slug` for rows that lack one, approximating the application
/// `slugify` + `resolve_collision`: empty normalizations coalesce to `untitled`,
/// the result is capped at 80 characters, and live rows whose titles normalize to
/// the same slug get deterministic `-2`, `-3`, … suffixes so the partial unique
/// index `(workspace_id, slug) WHERE deleted_at IS NULL` cannot be violated.
///
/// Divergence from the application `slugify`: this normalization is ASCII-only
/// (`[^a-zA-Z0-9]+` strips any non-ASCII character), whereas the application uses
/// Unicode `char::is_alphanumeric()` and so preserves accented and non-Latin
/// letters. Titles relying on those characters would yield a different slug here.
/// This is accepted because the migration adds the `slug` column in the same `up`
/// and the table is freshly all-NULL at backfill time; the "idempotent, only
/// touches `slug IS NULL`" property holds only against such an all-NULL column.
pub const BACKFILL_SLUG_SQL: &str = r#"WITH normalized AS (
       SELECT
           id,
           COALESCE(
               NULLIF(
                   RTRIM(
                       LEFT(
                           LOWER(
                               REGEXP_REPLACE(
                                   REGEXP_REPLACE(title, '[^a-zA-Z0-9]+', '-', 'g'),
                                   '^-|-$',
                                   '',
                                   'g'
                               )
                           ),
                           80
                       ),
                       '-'
                   ),
                   ''
               ),
               'untitled'
           ) AS base_slug,
           workspace_id,
           created_at
       FROM documents
       WHERE slug IS NULL
         AND deleted_at IS NULL
   ),
   ranked AS (
       SELECT
           id,
           base_slug,
           ROW_NUMBER() OVER (
               PARTITION BY workspace_id, base_slug
               ORDER BY created_at, id
           ) AS rn
       FROM normalized
   )
   UPDATE documents d
   SET slug = CASE
       WHEN r.rn = 1 THEN r.base_slug
       ELSE r.base_slug || '-' || r.rn::text
   END
   FROM ranked r
   WHERE d.id = r.id"#;

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

        conn.execute_unprepared(BACKFILL_SLUG_SQL).await?;

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
