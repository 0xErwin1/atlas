#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use sea_orm::{ConnectionTrait, FromQueryResult, Statement};
use uuid::Uuid;

/// Inserts a live document row with a NULL slug, bypassing the application layer
/// so the migration backfill can be exercised against pre-slug data.
async fn insert_doc_without_slug(
    db: &support::TestDb,
    workspace_id: Uuid,
    user_id: Uuid,
    title: &str,
    created_at: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            r#"INSERT INTO documents
               (id, workspace_id, title, content, slug, current_revision_seq,
                created_by_user_id, created_at, updated_at)
               VALUES ($1, $2, $3, '', NULL, 0, $4, $5::timestamptz, $5::timestamptz)"#,
            [
                id.into(),
                workspace_id.into(),
                title.into(),
                user_id.into(),
                created_at.into(),
            ],
        ))
        .await
        .expect("insert doc without slug");
    id
}

#[derive(FromQueryResult)]
struct SlugRow {
    slug: Option<String>,
}

async fn slug_of(db: &support::TestDb, id: Uuid) -> Option<String> {
    SlugRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT slug FROM documents WHERE id = $1",
        [id.into()],
    ))
    .one(db.conn())
    .await
    .expect("query slug")
    .expect("row exists")
    .slug
}

#[tokio::test]
async fn backfill_dedupes_colliding_titles_and_coalesces_empty() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "slug-backfill").await;

    let first = insert_doc_without_slug(
        &db,
        ws.id.0,
        user.id.0,
        "Hello World",
        "2026-01-01T00:00:00Z",
    )
    .await;
    let second = insert_doc_without_slug(
        &db,
        ws.id.0,
        user.id.0,
        "hello  world!",
        "2026-01-02T00:00:00Z",
    )
    .await;
    let third = insert_doc_without_slug(
        &db,
        ws.id.0,
        user.id.0,
        "HELLO-WORLD",
        "2026-01-03T00:00:00Z",
    )
    .await;
    let symbols =
        insert_doc_without_slug(&db, ws.id.0, user.id.0, "!!!", "2026-01-04T00:00:00Z").await;

    db.conn()
        .execute_unprepared(migration::m20260613_000006_document_slug::BACKFILL_SLUG_SQL)
        .await
        .expect("run backfill");

    let s1 = slug_of(&db, first).await.expect("slug 1");
    let s2 = slug_of(&db, second).await.expect("slug 2");
    let s3 = slug_of(&db, third).await.expect("slug 3");
    let s_sym = slug_of(&db, symbols).await.expect("slug symbols");

    assert_eq!(
        s1, "hello-world",
        "earliest colliding doc keeps the base slug"
    );

    let mut colliding = vec![s1.clone(), s2.clone(), s3.clone()];
    colliding.sort();
    colliding.dedup();
    assert_eq!(
        colliding.len(),
        3,
        "all three colliding docs must receive distinct slugs, got duplicates"
    );

    for s in [&s2, &s3] {
        assert!(
            s.starts_with("hello-world-"),
            "later colliding docs must be suffixed, got: {s}"
        );
    }

    assert_eq!(
        s_sym, "untitled",
        "an all-symbol title must coalesce to 'untitled'"
    );

    db.teardown().await;
}

#[tokio::test]
async fn backfill_is_safe_on_empty_table() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    db.conn()
        .execute_unprepared(migration::m20260613_000006_document_slug::BACKFILL_SLUG_SQL)
        .await
        .expect("backfill on empty table must not error");

    db.teardown().await;
}
