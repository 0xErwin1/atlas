#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use sea_orm::{FromQueryResult, Statement};

#[derive(Debug, FromQueryResult)]
struct IndexRow {
    indexname: String,
}

/// Confirms that the E02 GIN indexes on `documents` and `tasks` exist and that
/// the `search_vector` generated columns are present. This test guards against
/// accidental regression of the FTS substrate that E06 depends on.
#[tokio::test]
async fn e02_gin_indexes_exist_on_documents_and_tasks() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let expected_indexes = [
        "documents_search_vector_gin",
        "tasks_search_vector_gin",
        "documents_frontmatter_gin",
        "tasks_labels_gin",
        "tasks_properties_gin",
    ];

    let rows = IndexRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        r#"SELECT indexname::text
           FROM pg_indexes
           WHERE tablename IN ('documents', 'tasks')
             AND indexname = ANY($1)"#,
        [expected_indexes
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into()],
    ))
    .all(db.conn())
    .await
    .expect("pg_indexes query");

    let found: Vec<String> = rows.into_iter().map(|r| r.indexname).collect();

    for idx in &expected_indexes {
        assert!(
            found.iter().any(|f| f == idx),
            "expected GIN index '{idx}' to exist but it was not found in pg_indexes; \
             found: {found:?}"
        );
    }

    db.teardown().await;
}

/// Confirms that `documents.search_vector` and `tasks.search_vector` exist as
/// generated columns in the schema (they are the FTS substrate for E06).
#[tokio::test]
async fn search_vector_columns_exist_and_are_generated() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct ColRow {
        table_name: String,
        #[allow(dead_code)]
        column_name: String,
    }

    let rows = ColRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        r#"SELECT table_name::text, column_name::text
           FROM information_schema.columns
           WHERE table_name IN ('documents', 'tasks')
             AND column_name = 'search_vector'"#,
        [],
    ))
    .all(db.conn())
    .await
    .expect("information_schema query");

    assert_eq!(
        rows.len(),
        2,
        "both documents.search_vector and tasks.search_vector must exist; found: {rows:?}"
    );

    let tables: Vec<&str> = rows.iter().map(|r| r.table_name.as_str()).collect();
    assert!(tables.contains(&"documents"), "documents.search_vector missing");
    assert!(tables.contains(&"tasks"), "tasks.search_vector missing");

    db.teardown().await;
}
