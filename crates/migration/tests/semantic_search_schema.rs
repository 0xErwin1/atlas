use migration::m20260708_000039_search_embeddings::Migration;
use sea_orm_migration::prelude::MigrationName;

#[test]
fn semantic_search_embeddings_migration_name_follows_apikey_scopes() {
    assert_eq!(Migration.name(), "m20260708_000039_search_embeddings");
}

#[test]
fn semantic_search_embeddings_schema_contains_required_pgvector_shape() {
    let schema = migration::m20260708_000039_search_embeddings::up_sql();

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
