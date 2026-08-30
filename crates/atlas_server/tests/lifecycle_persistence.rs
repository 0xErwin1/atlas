#![allow(clippy::expect_used)]

mod support;

use atlas_acta::entities::lifecycle::PurgeExecutor;
use atlas_acta::entities::lifecycle::PurgeStatus;
use atlas_acta::entities::lifecycle::RestoreTarget;
use atlas_acta::entities::lifecycle::SecurityAuditRef;
use atlas_acta::entities::lifecycle::TrashKind;
use atlas_core::principal::UserId;
use atlas_custos::ids::SecurityAuditId;
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
        "INSERT INTO custos.security_audit_log (id, workspace_id, actor_user_id, action, target_type, metadata, created_at) \
         VALUES ($1, $2, $3, 'resource.purge_committed', 'document', '{}'::jsonb, now())",
        [audit_id.0.into(), workspace_id.into(), user_id.into()],
    ))
    .await
    .expect("seed purge commit audit");

    audit_id
}

#[tokio::test]
async fn purge_repo_persists_each_status_with_its_derived_audit_action() {
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
                commit_audit_id: SecurityAuditRef(audit_id.0),
            },
        )
        .await
        .expect("create purge operation");

    assert_eq!(created.status, PurgeStatus::DbCommitted);
    assert_eq!(created.attempts, 0);
    assert_eq!(created.target.kind, TrashKind::Document);
    assert_eq!(created.last_action.as_str(), "resource.purge_committed");
    assert_eq!(created.last_executor, PurgeExecutor::User);
    // Characterizes the `SecurityAuditId` -> `SecurityAuditRef` -> DB -> `SecurityAuditRef`
    // round trip introduced by the crate split: the stored uuid must survive unchanged.
    assert_eq!(created.commit_audit_id, SecurityAuditRef(audit_id.0));

    let pending = repo
        .record_attempt_in(
            db.conn(),
            created.id,
            PurgeStatus::CleanupPending,
            PurgeExecutor::System,
            None,
        )
        .await
        .expect("persist pending cleanup attempt");

    assert_eq!(pending.status, PurgeStatus::CleanupPending);
    assert_eq!(pending.attempts, 1);
    assert_eq!(
        pending.last_action.as_str(),
        "resource.purge_cleanup_pending"
    );
    assert_eq!(pending.last_executor, PurgeExecutor::System);

    let failed = repo
        .record_attempt_in(
            db.conn(),
            created.id,
            PurgeStatus::CleanupFailed,
            PurgeExecutor::System,
            Some("object storage unavailable".into()),
        )
        .await
        .expect("persist failed cleanup attempt");

    assert_eq!(failed.status, PurgeStatus::CleanupFailed);
    assert_eq!(failed.attempts, 2);
    assert_eq!(failed.last_action.as_str(), "resource.purge_cleanup_failed");
    assert_eq!(failed.last_executor, PurgeExecutor::System);
    assert_eq!(
        failed.last_error.as_deref(),
        Some("object storage unavailable")
    );

    let complete = repo
        .record_attempt_in(
            db.conn(),
            created.id,
            PurgeStatus::Complete,
            PurgeExecutor::System,
            None,
        )
        .await
        .expect("persist completed cleanup attempt");

    assert_eq!(complete.status, PurgeStatus::Complete);
    assert_eq!(complete.attempts, 3);
    assert_eq!(complete.last_action.as_str(), "resource.purge_completed");
    assert_eq!(complete.last_executor, PurgeExecutor::System);

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

#[tokio::test]
async fn purge_operation_rejects_contradictory_status_and_action() {
    let db = TestDb::create().await.expect("create test database");
    let (workspace, user) = support::seed_workspace(&db, "purge-status-constraint").await;
    let audit_id = seed_commit_audit(db.conn(), workspace.id.0, user.id.0).await;
    let repo = PgPurgeOperationRepo;
    let operation = repo
        .create_in(
            db.conn(),
            NewPurgeOperation {
                workspace_id: workspace.id,
                target: RestoreTarget {
                    kind: TrashKind::Document,
                    target_id: uuid::Uuid::now_v7(),
                },
                original_actor_user_id: UserId(user.id.0),
                commit_audit_id: SecurityAuditRef(audit_id.0),
            },
        )
        .await
        .expect("create purge operation");

    let result = db
        .conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE purge_operations SET status = 'complete', last_action = 'resource.purge_cleanup_failed' WHERE id = $1",
            [operation.id.0.into()],
        ))
        .await;

    assert!(result.is_err());

    db.teardown().await;
}
