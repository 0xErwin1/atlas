#![allow(clippy::expect_used)]

mod support;

use atlas_domain::entities::documents::NewDocument;
use atlas_server::persistence::repos::{DocumentRepo, PgDocumentRepo};
use migration::Migrator;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::MigratorTrait;
use support::TestDb;

const COMMENT_FREEDOM_MIGRATION_STEPS: u32 = 39;

async fn seed_live_comment_owned_records(
    conn: &sea_orm::DatabaseConnection,
    workspace_id: uuid::Uuid,
    user_id: uuid::Uuid,
    document_id: uuid::Uuid,
) -> (uuid::Uuid, uuid::Uuid) {
    let comment_id = uuid::Uuid::now_v7();
    let attachment_id = uuid::Uuid::now_v7();
    conn.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO comments (id, workspace_id, document_id, body, created_by_user_id, created_at, updated_at) \
         VALUES ($1, $2, $3, 'comment attachment owner', $4, now(), now())",
        [
            comment_id.into(),
            workspace_id.into(),
            document_id.into(),
            user_id.into(),
        ],
    ))
    .await
    .expect("seed comment owner");
    conn.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO attachments (id, workspace_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) \
         VALUES ($1, $2, $3, 'live.txt', 'text/plain', 4, $4, $5, now(), now())",
        [
            attachment_id.into(),
            workspace_id.into(),
            comment_id.into(),
            "a".repeat(64).into(),
            user_id.into(),
        ],
    ))
    .await
    .expect("seed live comment-owned attachment");
    conn.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO comment_links (id, workspace_id, comment_id, target_document_id, created_at) \
         VALUES ($1, $2, $3, $4, now())",
        [
            uuid::Uuid::now_v7().into(),
            workspace_id.into(),
            comment_id.into(),
            document_id.into(),
        ],
    ))
    .await
    .expect("seed comment link");
    conn.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO comment_link_events (id, workspace_id, parent_document_id, comment_id, event_kind, target_document_id, actor_type, actor_id, created_at) \
         VALUES ($1, $2, $3, $4, 'link_added', $3, 'user', $5, now())",
        [
            uuid::Uuid::now_v7().into(),
            workspace_id.into(),
            document_id.into(),
            comment_id.into(),
            user_id.into(),
        ],
    ))
    .await
    .expect("seed comment link event");
    conn.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO attachment_write_intents (id, digest, created_at) VALUES ($1, $2, now())",
        [uuid::Uuid::now_v7().into(), "a".repeat(64).into()],
    ))
    .await
    .expect("seed attachment write intent");

    (comment_id, attachment_id)
}

#[tokio::test]
async fn comment_freedom_down_rejects_live_comment_attachment_without_destructive_changes() {
    let db = TestDb::create_with_migration_steps(Some(COMMENT_FREEDOM_MIGRATION_STEPS))
        .await
        .expect("create database before comment freedom migration");
    let (workspace, user) = support::seed_workspace(&db, "comment-freedom-down-blocked").await;
    let ctx = support::ctx(&workspace, &user);
    let document = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &ctx,
            NewDocument {
                title: "Comment attachment parent".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed parent document before migration");

    db.run_remaining_migrations()
        .await
        .expect("apply comment freedom migration");

    let (comment_id, attachment_id) =
        seed_live_comment_owned_records(db.conn(), workspace.id.0, user.id.0, document.id.0).await;

    let error = Migrator::down(db.conn(), Some(1))
        .await
        .expect_err("rollback must fail closed while a live comment attachment exists");
    assert!(
        error.to_string().contains("live comment-owned attachments"),
        "rollback must report its live comment attachment precondition: {error}"
    );

    let attachment_count = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*)::bigint AS count FROM attachments WHERE id = $1 AND comment_id = $2",
            [attachment_id.into(), comment_id.into()],
        ))
        .await
        .expect("query preserved attachment")
        .expect("attachment count row")
        .try_get::<i64>("", "count")
        .expect("attachment count");
    assert_eq!(
        attachment_count, 1,
        "failed rollback must preserve the attachment row"
    );

    let link_count = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*)::bigint AS count FROM comment_links",
        ))
        .await
        .expect("query preserved links")
        .expect("link count row")
        .try_get::<i64>("", "count")
        .expect("link count");
    assert_eq!(
        link_count, 1,
        "failed rollback must not partially drop comment links"
    );

    for table in ["comment_link_events", "attachment_write_intents"] {
        let count = db
            .conn()
            .query_one_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Postgres,
                format!("SELECT count(*)::bigint AS count FROM {table}"),
            ))
            .await
            .expect("query preserved ATL-80 data")
            .expect("ATL-80 data count row")
            .try_get::<i64>("", "count")
            .expect("ATL-80 data count");
        assert_eq!(count, 1, "failed rollback must not partially drop {table}");
    }

    db.teardown().await;
}

#[tokio::test]
async fn comment_freedom_down_restores_legacy_attachment_xor_when_schema_is_empty() {
    let db = TestDb::create_with_migration_steps(Some(COMMENT_FREEDOM_MIGRATION_STEPS))
        .await
        .expect("create database before comment freedom migration");

    db.run_remaining_migrations()
        .await
        .expect("apply comment freedom migration");
    Migrator::down(db.conn(), Some(1))
        .await
        .expect("rollback empty comment freedom migration");

    let owner_constraint = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT pg_get_constraintdef(c.oid) AS definition FROM pg_constraint c \
             JOIN pg_class t ON t.oid = c.conrelid \
             WHERE t.relname = 'attachments' AND c.conname = 'attachments_owner_check'",
        ))
        .await
        .expect("query restored attachment owner constraint")
        .expect("attachment owner constraint exists")
        .try_get::<String>("", "definition")
        .expect("attachment owner constraint definition");
    assert!(
        owner_constraint.contains("num_nonnulls(document_id, task_id) = 1"),
        "rollback must restore the legacy two-owner XOR: {owner_constraint}"
    );

    for table in [
        "comment_links",
        "comment_link_events",
        "attachment_write_intents",
    ] {
        let exists = db
            .conn()
            .query_one_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "SELECT to_regclass($1) IS NOT NULL AS exists",
                [table.into()],
            ))
            .await
            .expect("query ATL-80 table")
            .expect("table existence row")
            .try_get::<bool>("", "exists")
            .expect("table existence");
        assert!(!exists, "rollback must remove {table}");
    }

    let comment_column_exists = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
             WHERE table_name = 'attachments' AND column_name = 'comment_id') AS exists",
        ))
        .await
        .expect("query comment owner column")
        .expect("comment owner column row")
        .try_get::<bool>("", "exists")
        .expect("comment owner column existence");
    assert!(
        !comment_column_exists,
        "rollback must remove attachments.comment_id"
    );

    db.teardown().await;
}

#[tokio::test]
async fn comment_freedom_down_rejects_live_attachment_write_intent() {
    let db = TestDb::create_with_migration_steps(Some(COMMENT_FREEDOM_MIGRATION_STEPS))
        .await
        .expect("create database before comment freedom migration");

    db.run_remaining_migrations()
        .await
        .expect("apply comment freedom migration");
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO attachment_write_intents (id, digest, created_at) VALUES ($1, $2, now())",
            [uuid::Uuid::now_v7().into(), "b".repeat(64).into()],
        ))
        .await
        .expect("seed live attachment write intent");

    let error = Migrator::down(db.conn(), Some(1))
        .await
        .expect_err("rollback must fail closed while an attachment write intent exists");
    assert!(
        error.to_string().contains("attachment write intents"),
        "rollback must report its live attachment write intent precondition: {error}"
    );

    db.teardown().await;
}
