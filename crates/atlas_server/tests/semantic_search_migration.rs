#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

#[tokio::test]
async fn forward_migration_repairs_a_missing_semantic_search_schema() {
    let db = support::TestDb::create_with_migration_steps(Some(44))
        .await
        .expect("database through migration 44");
    db.conn()
        .execute_unprepared("DROP TABLE search_embeddings")
        .await
        .expect("drop semantic search table");

    db.run_remaining_migrations()
        .await
        .expect("run repair migration");

    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT to_regclass('public.search_embeddings') IS NOT NULL AS table_exists, \
             to_regclass('public.search_embeddings_workspace_resource_idx') IS NOT NULL AS resource_index_exists, \
             to_regclass('public.search_embeddings_model_dimensions_stale_idx') IS NOT NULL AS model_index_exists, \
             to_regclass('public.search_embeddings_ann_idx') IS NOT NULL AS ann_index_exists",
        ))
        .await
        .expect("query repaired schema")
        .expect("schema query row");

    for column in [
        "table_exists",
        "resource_index_exists",
        "model_index_exists",
        "ann_index_exists",
    ] {
        assert!(row.try_get::<bool>("", column).expect("boolean column"));
    }

    db.teardown().await;
}
