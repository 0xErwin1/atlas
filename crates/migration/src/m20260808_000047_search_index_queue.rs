use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260808_000047_search_index_queue"
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

/// The queue is deliberately independent of pgvector: rows accumulate even when
/// embeddings are disabled or the extension is missing, so enabling the feature
/// later does not require a separate backfill of the writes that happened in
/// between.
pub fn up_sql() -> &'static [&'static str] {
    &[
        r#"
        CREATE TABLE IF NOT EXISTS search_index_queue (
            id UUID PRIMARY KEY,
            workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            resource_kind TEXT NOT NULL CHECK (resource_kind IN ('document', 'task')),
            resource_id UUID NOT NULL,
            enqueued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
            next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            locked_until TIMESTAMPTZ,
            last_error TEXT,
            UNIQUE (workspace_id, resource_kind, resource_id)
        )
        "#,
        "CREATE INDEX IF NOT EXISTS search_index_queue_claim_idx \
         ON search_index_queue (next_attempt_at, enqueued_at)",
    ]
}

pub fn down_sql() -> &'static [&'static str] {
    &[
        "DROP INDEX IF EXISTS search_index_queue_claim_idx",
        "DROP TABLE IF EXISTS search_index_queue",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_name_is_stable() {
        assert_eq!(Migration.name(), "m20260808_000047_search_index_queue");
    }

    #[test]
    fn up_sql_coalesces_repeat_enqueues_per_resource() {
        let create_table = up_sql().first().expect("up_sql creates the table first");
        assert!(
            create_table.contains("UNIQUE (workspace_id, resource_kind, resource_id)"),
            "one pending row per resource is what makes ON CONFLICT coalescing work"
        );
    }
}
