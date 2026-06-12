#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use sea_orm::ConnectionTrait;

/// Inserts a row with no actor on `documents` and expects the DB to reject it.
/// The CHECK constraint must enforce exactly-one (num_nonnulls = 1), not at-most-one (<= 1).
#[tokio::test]
async fn document_requires_exactly_one_actor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "actor-check-doc").await;

    let ws_id = ws.id.0;
    let user_id = user.id.0;

    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents
               (id, workspace_id, title, content, current_revision_seq,
                created_by_user_id, created_by_api_key_id, created_at, updated_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', 'No Actor', '', 0,
                NULL, NULL, now(), now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "inserting a document with no actor must be rejected by the DB"
    );

    let result2 = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents
               (id, workspace_id, title, content, current_revision_seq,
                created_by_user_id, created_by_api_key_id, created_at, updated_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', 'Both Actors', '', 0,
                '{user_id}', '{user_id}', now(), now())"#
        ))
        .await;

    assert!(
        result2.is_err(),
        "inserting a document with both actors must be rejected by the DB"
    );

    db.teardown().await;
}

/// Same constraint on `document_revisions`.
#[tokio::test]
async fn document_revision_requires_exactly_one_actor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "actor-check-rev").await;

    let ws_id = ws.id.0;
    let user_id = user.id.0;

    let doc_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents
               (id, workspace_id, title, content, current_revision_seq,
                created_by_user_id, created_at, updated_at)
               VALUES ('{doc_id}', '{ws_id}', 'Rev Parent', '', 0, '{user_id}', now(), now())"#
        ))
        .await
        .expect("parent doc insert");

    let rev_id = uuid::Uuid::now_v7();
    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO document_revisions
               (id, workspace_id, document_id, seq, snapshot, is_anchor,
                created_by_user_id, created_by_api_key_id, created_at)
               VALUES
               ('{rev_id}', '{ws_id}', '{doc_id}', 1, 'snap', true,
                NULL, NULL, now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "inserting a revision with no actor must be rejected by the DB"
    );

    db.teardown().await;
}

/// Same constraint on `attachments`.
#[tokio::test]
async fn attachment_requires_exactly_one_actor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "actor-check-att").await;

    let ws_id = ws.id.0;
    let user_id = user.id.0;

    let doc_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents
               (id, workspace_id, title, content, current_revision_seq,
                created_by_user_id, created_at, updated_at)
               VALUES ('{doc_id}', '{ws_id}', 'Att Parent', '', 0, '{user_id}', now(), now())"#
        ))
        .await
        .expect("parent doc insert");

    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO attachments
               (id, workspace_id, document_id, file_name, content_type, size_bytes, sha256,
                created_by_user_id, created_by_api_key_id, created_at, updated_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', '{doc_id}', 'f.txt', 'text/plain', 10, 'abc',
                NULL, NULL, now(), now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "inserting an attachment with no actor must be rejected by the DB"
    );

    db.teardown().await;
}

/// Same constraint on `property_definitions`.
#[tokio::test]
async fn property_definition_requires_exactly_one_actor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, _user) = support::seed_workspace(&db, "actor-check-prop").await;

    let ws_id = ws.id.0;

    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO property_definitions
               (id, workspace_id, key, name, kind,
                created_by_user_id, created_by_api_key_id, created_at, updated_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', 'tag', 'Tag', 'text',
                NULL, NULL, now(), now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "inserting a property_definition with no actor must be rejected by the DB"
    );

    db.teardown().await;
}

/// Same constraint on `projects`.
#[tokio::test]
async fn project_requires_exactly_one_actor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, _user) = support::seed_workspace(&db, "actor-check-proj").await;

    let ws_id = ws.id.0;

    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO projects
               (id, workspace_id, name, slug, task_prefix,
                created_by_user_id, created_by_api_key_id, created_at, updated_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', 'No Actor', 'no-actor', 'NAC',
                NULL, NULL, now(), now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "inserting a project with no actor must be rejected by the DB"
    );

    db.teardown().await;
}

/// Same constraint on `folders`.
#[tokio::test]
async fn folder_requires_exactly_one_actor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, _user) = support::seed_workspace(&db, "actor-check-fold").await;

    let ws_id = ws.id.0;

    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO folders
               (id, workspace_id, name,
                created_by_user_id, created_by_api_key_id, created_at, updated_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', 'No Actor',
                NULL, NULL, now(), now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "inserting a folder with no actor must be rejected by the DB"
    );

    db.teardown().await;
}

/// Inserting a document with NULL frontmatter must be rejected by the DB.
/// The column must be NOT NULL with DEFAULT '{}'.
#[tokio::test]
async fn document_frontmatter_is_not_null() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "fm-null-check").await;

    let ws_id = ws.id.0;
    let user_id = user.id.0;

    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents
               (id, workspace_id, title, content, frontmatter, current_revision_seq,
                created_by_user_id, created_at, updated_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', 'NULL FM', '', NULL, 0,
                '{user_id}', now(), now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "inserting a document with NULL frontmatter must be rejected by the DB"
    );

    db.teardown().await;
}

/// `boards` requires exactly one actor (created_by_user_id NOT NULL).
#[tokio::test]
async fn board_requires_actor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "actor-check-board").await;

    let ws_id = ws.id.0;
    let user_id = user.id.0;

    let proj_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO projects
               (id, workspace_id, name, slug, task_prefix,
                created_by_user_id, created_at, updated_at)
               VALUES ('{proj_id}', '{ws_id}', 'P', 'p-board-actor', 'BA', '{user_id}', now(), now())"#
        ))
        .await
        .expect("seed project");

    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO boards
               (id, workspace_id, project_id, name,
                created_by_user_id, created_at, updated_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', '{proj_id}', 'No Actor',
                NULL, now(), now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "inserting a board with no actor must be rejected by the DB"
    );

    db.teardown().await;
}
