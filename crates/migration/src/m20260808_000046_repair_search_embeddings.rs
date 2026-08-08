use sea_orm_migration::prelude::*;

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260808_000046_repair_search_embeddings"
    }
}

/// Re-applies the semantic search schema on deployments that ran every earlier
/// attempt before pgvector was installable.
///
/// The schema definition skips itself when `pg_available_extensions` does not
/// offer `vector`, and a skipped run is still recorded as applied, so neither
/// `m20260708_000039_search_embeddings` nor `m20260804_000045_repair_search_embeddings`
/// can create the table once they have been consumed. The statement is fully
/// idempotent, so this is a no-op wherever the schema already exists.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(up_sql())
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

pub fn up_sql() -> &'static str {
    crate::m20260708_000039_search_embeddings::up_sql()
}
