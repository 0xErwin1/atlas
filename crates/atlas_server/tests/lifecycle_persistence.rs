#![allow(clippy::expect_used)]

mod support;

use atlas_domain::{
    entities::lifecycle::{PurgeStatus, RestoreTarget, TrashKind},
    ids::{SecurityAuditId, UserId},
};
use atlas_server::persistence::repos::{NewPurgeOperation, PgPurgeOperationRepo};
use sea_orm::{ConnectionTrait, Statement};
use support::TestDb;

async fn seed_commit_audit(
    conn: &sea_orm::DatabaseConnection,
    workspace_id: uuid::Uuid,
    user_id: uuid::Uuid,
) -> SecurityAuditId {
    let audit_id = SecurityAuditId::new();
    conn.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO security_audit_log (id, workspace_id, actor_user_id, action, target_type, metadata, created_at) \
         VALUES ($1, $2, $3, 'resource.purge_committed', 'document', '{}'::jsonb, now())",
        [audit_id.0.into(), workspace_id.into(), user_id.into()],
    ))
    .await
    .expect("seed purge commit audit");

    audit_id
}

#[tokio::test]
async fn purge_repo_maps_durable_status_and_attempts() {
    let db = TestDb::create().await.expect("create test database");
    let (workspace, user) = support::seed_workspace(&db, "purge-status-mapping").await;
    let audit_id = seed_commit_audit(db.conn(), workspace.id.0, user.id.0).await;
    let target = RestoreTarget {
        kind: TrashKind::Document,
        target_id: uuid::Uuid::now_v7(),
    };

    let repo = PgPurgeOperationRepo;
    let created = repo
        .create_in(
            db.conn(),
            NewPurgeOperation {
                workspace_id: workspace.id,
                target: target.clone(),
                original_actor_user_id: UserId(user.id.0),
                commit_audit_id: audit_id,
            },
        )
        .await
        .expect("create purge operation");

    assert_eq!(created.status, PurgeStatus::DbCommitted);
    assert_eq!(created.attempts, 0);
    assert_eq!(created.target.kind, TrashKind::Document);

    let updated = repo
        .record_attempt_in(
            db.conn(),
            created.id,
            PurgeStatus::CleanupFailed,
            "resource.purge_cleanup_failed",
            "system",
            Some("object storage unavailable".into()),
        )
        .await
        .expect("persist failed cleanup attempt");

    assert_eq!(updated.status, PurgeStatus::CleanupFailed);
    assert_eq!(updated.attempts, 1);
    assert_eq!(updated.last_executor, "system");
    assert_eq!(
        updated.last_error.as_deref(),
        Some("object storage unavailable")
    );

    let digest = repo
        .create_digest_in(db.conn(), created.id, "shared-digest".into())
        .await
        .expect("create purge digest");

    assert_eq!(digest.status, PurgeStatus::DbCommitted);
    assert_eq!(digest.attempts, 0);

    let retried_digest = repo
        .record_digest_attempt_in(
            db.conn(),
            created.id,
            "shared-digest",
            PurgeStatus::CleanupPending,
            None,
        )
        .await
        .expect("persist retryable digest attempt");

    assert_eq!(retried_digest.status, PurgeStatus::CleanupPending);
    assert_eq!(retried_digest.attempts, 1);
    assert!(retried_digest.last_error.is_none());

    db.teardown().await;
}
