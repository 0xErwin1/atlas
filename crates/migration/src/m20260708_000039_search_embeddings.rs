use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260708_000039_search_embeddings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for statement in up_sql() {
            conn.execute_unprepared(statement).await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        for statement in down_sql() {
            conn.execute_unprepared(statement).await?;
        }

        Ok(())
    }
}

pub fn up_sql() -> &'static [&'static str] {
    &[
        "CREATE EXTENSION IF NOT EXISTS vector",
        r#"CREATE TABLE IF NOT EXISTS search_embeddings (
            id UUID PRIMARY KEY,
            workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            resource_kind TEXT NOT NULL CHECK (resource_kind IN ('document', 'task')),
            resource_id UUID NOT NULL,
            source_field TEXT NOT NULL CHECK (source_field IN ('title', 'content', 'comment', 'attachment_name', 'checklist', 'subtask', 'aggregate')),
            chunk_ordinal INTEGER NOT NULL CHECK (chunk_ordinal >= 0),
            content_hash TEXT NOT NULL,
            model TEXT NOT NULL,
            dimensions INTEGER NOT NULL CHECK (dimensions > 0),
            embedding vector(1536) NOT NULL,
            excerpt TEXT NOT NULL,
            token_count INTEGER CHECK (token_count IS NULL OR token_count >= 0),
            indexed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            stale_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            UNIQUE (workspace_id, resource_kind, resource_id, source_field, chunk_ordinal, model, dimensions)
        )"#,
        "CREATE INDEX IF NOT EXISTS search_embeddings_workspace_resource_idx ON search_embeddings (workspace_id, resource_kind, resource_id)",
        "CREATE INDEX IF NOT EXISTS search_embeddings_model_dimensions_stale_idx ON search_embeddings (workspace_id, model, dimensions, stale_at)",
        "CREATE INDEX IF NOT EXISTS search_embeddings_ann_idx ON search_embeddings USING ivfflat (embedding vector_cosine_ops) WITH (lists = 100)",
    ]
}

pub fn down_sql() -> &'static [&'static str] {
    &[
        "DROP INDEX IF EXISTS search_embeddings_ann_idx",
        "DROP INDEX IF EXISTS search_embeddings_model_dimensions_stale_idx",
        "DROP INDEX IF EXISTS search_embeddings_workspace_resource_idx",
        "DROP TABLE IF EXISTS search_embeddings",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260708_000039_search_embeddings");
    }
}
