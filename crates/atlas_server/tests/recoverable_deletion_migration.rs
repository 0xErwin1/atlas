#![allow(clippy::expect_used)]

mod support;

use migration::Migrator;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::MigratorTrait;
use support::TestDb;

const PRE_RECOVERABLE_DELETION_MIGRATION_STEPS: u32 = 42;

async fn relation_exists(conn: &sea_orm::DatabaseConnection, relation: &str) -> bool {
    conn.query_one_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT to_regclass($1) IS NOT NULL AS exists",
        [relation.into()],
    ))
    .await
    .expect("query relation")
    .expect("relation existence row")
    .try_get::<bool>("", "exists")
    .expect("relation existence")
}

#[tokio::test]
async fn recoverable_deletion_migration_creates_and_removes_durable_purge_schema() {
    let db = TestDb::create_with_migration_steps(Some(PRE_RECOVERABLE_DELETION_MIGRATION_STEPS))
        .await
        .expect("create database before recoverable deletion migration");

    Migrator::up(db.conn(), Some(1))
        .await
        .expect("apply recoverable deletion migration");

    for relation in [
        "purge_operations",
        "purge_operation_digests",
        "purge_operations_workspace_deleted_idx",
        "projects_workspace_deleted_idx",
        "folders_workspace_deleted_idx",
        "documents_workspace_deleted_idx",
        "comments_workspace_deleted_idx",
        "attachments_workspace_deleted_idx",
    ] {
        assert!(
            relation_exists(db.conn(), relation).await,
            "migration must create {relation}"
        );
    }

    Migrator::down(db.conn(), Some(1))
        .await
        .expect("rollback recoverable deletion migration");

    for relation in ["purge_operations", "purge_operation_digests"] {
        assert!(
            !relation_exists(db.conn(), relation).await,
            "rollback must remove {relation}"
        );
    }

    db.teardown().await;
}
