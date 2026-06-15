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

/// Confirms that `documents.search_vector` and `tasks.search_vector` are GENERATED
/// ALWAYS STORED columns (not plain text columns). The REQ-9 freshness contract
/// depends on this: a plain TEXT column would pass an existence check but would
/// never auto-update on document/task writes.
#[tokio::test]
async fn search_vector_columns_exist_and_are_generated() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    // pg_attribute.attgenerated = 's' means STORED generated column.
    // A plain column (or a virtual/computed column that is not STORED) would
    // return an empty string here and fail the assertion below.
    #[derive(Debug, FromQueryResult)]
    struct ColRow {
        table_name: String,
        is_stored_generated: String,
    }

    let rows = ColRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        r#"SELECT c.relname::text AS table_name,
                  a.attgenerated::text AS is_stored_generated
           FROM pg_attribute a
           JOIN pg_class c ON c.oid = a.attrelid
           JOIN pg_namespace n ON n.oid = c.relnamespace
           WHERE n.nspname = 'public'
             AND c.relname IN ('documents', 'tasks')
             AND a.attname = 'search_vector'
             AND NOT a.attisdropped"#,
        [],
    ))
    .all(db.conn())
    .await
    .expect("pg_attribute query");

    assert_eq!(
        rows.len(),
        2,
        "both documents.search_vector and tasks.search_vector must exist; found: {rows:?}"
    );

    for row in &rows {
        assert_eq!(
            row.is_stored_generated, "s",
            "search_vector on table '{}' must be a STORED generated column \
             (pg_attribute.attgenerated = 's'); got '{}'",
            row.table_name, row.is_stored_generated
        );
    }

    let tables: Vec<&str> = rows.iter().map(|r| r.table_name.as_str()).collect();
    assert!(tables.contains(&"documents"), "documents.search_vector missing");
    assert!(tables.contains(&"tasks"), "tasks.search_vector missing");

    db.teardown().await;
}
