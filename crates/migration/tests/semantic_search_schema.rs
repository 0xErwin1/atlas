use migration::{
    m20260708_000039_search_embeddings::{self, Migration as HistoricalMigration},
    m20260804_000045_repair_search_embeddings::Migration as RepairMigration,
    m20260808_000046_repair_search_embeddings::Migration as SecondRepairMigration,
};
use sea_orm_migration::prelude::MigrationName;

#[test]
fn semantic_search_embeddings_migration_name_follows_apikey_scopes() {
    assert_eq!(
        HistoricalMigration.name(),
        "m20260708_000039_search_embeddings"
    );
}

#[test]
fn semantic_search_embeddings_schema_contains_required_pgvector_shape() {
    let schema = m20260708_000039_search_embeddings::up_sql();

    assert!(schema.contains("pg_available_extensions WHERE name = 'vector'"));
    assert!(schema.contains("CREATE EXTENSION IF NOT EXISTS vector"));
    assert!(schema.contains("CREATE TABLE IF NOT EXISTS search_embeddings"));
    assert!(schema.contains("embedding vector(1536)"));
    assert!(schema.contains("UNIQUE (workspace_id, resource_kind, resource_id, source_field, chunk_ordinal, model, dimensions)"));
    assert!(schema.contains("CHECK (resource_kind IN ('document', 'task'))"));
    assert!(schema.contains("search_embeddings_workspace_resource_idx"));
    assert!(schema.contains("search_embeddings_model_dimensions_stale_idx"));
    assert!(schema.contains("USING ivfflat (embedding vector_cosine_ops)"));
    assert!(schema.contains("skipping optional semantic search embedding schema"));
}

#[test]
fn semantic_search_schema_repair_reuses_the_idempotent_schema_definition() {
    assert_eq!(
        RepairMigration.name(),
        "m20260804_000045_repair_search_embeddings"
    );
    assert_eq!(
        migration::m20260804_000045_repair_search_embeddings::up_sql(),
        m20260708_000039_search_embeddings::up_sql()
    );
}

#[test]
fn second_semantic_search_schema_repair_reuses_the_idempotent_schema_definition() {
    assert_eq!(
        SecondRepairMigration.name(),
        "m20260808_000046_repair_search_embeddings"
    );
    assert_eq!(
        migration::m20260808_000046_repair_search_embeddings::up_sql(),
        m20260708_000039_search_embeddings::up_sql()
    );
}
