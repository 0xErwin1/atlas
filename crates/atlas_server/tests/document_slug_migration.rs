#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use sea_orm::ConnectionTrait;

#[tokio::test]
async fn slug_column_exists_after_migration() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let result = db
        .conn()
        .execute_unprepared("SELECT slug FROM documents WHERE false")
        .await;

    assert!(
        result.is_ok(),
        "slug column must exist in documents table after migration"
    );

    db.teardown().await;
}

#[tokio::test]
async fn partial_unique_index_allows_same_slug_for_deleted_rows() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "slug-deleted-1").await;

    let ws_id = ws.id;
    let user_id = user.id;

    db.conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents (id, workspace_id, title, slug, content, frontmatter,
               current_revision_seq, created_by_user_id, created_at, updated_at, deleted_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', 'Deleted Doc A', 'same-slug', '',
                '{{}}', 0, '{user_id}', now(), now(), now()),
               (gen_random_uuid(), '{ws_id}', 'Deleted Doc B', 'same-slug', '',
                '{{}}', 0, '{user_id}', now(), now(), now())"#
        ))
        .await
        .expect("two deleted rows with same slug must not violate the partial unique index");

    db.teardown().await;
}

#[tokio::test]
async fn partial_unique_index_rejects_same_slug_for_live_rows() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "slug-live-1").await;

    let ws_id = ws.id;
    let user_id = user.id;

    db.conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents (id, workspace_id, title, slug, content, frontmatter,
               current_revision_seq, created_by_user_id, created_at, updated_at)
               VALUES (gen_random_uuid(), '{ws_id}', 'Live Doc A', 'conflict-slug', '',
               '{{}}', 0, '{user_id}', now(), now())"#
        ))
        .await
        .expect("first live row with slug must succeed");

    let second = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents (id, workspace_id, title, slug, content, frontmatter,
               current_revision_seq, created_by_user_id, created_at, updated_at)
               VALUES (gen_random_uuid(), '{ws_id}', 'Live Doc B', 'conflict-slug', '',
               '{{}}', 0, '{user_id}', now(), now())"#
        ))
        .await;

    assert!(
        second.is_err(),
        "second live row with same slug must violate the partial unique index"
    );

    db.teardown().await;
}
