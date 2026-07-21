use atlas_domain::{
    DomainError,
    entities::lifecycle::{PurgeDigest, PurgeOperation, PurgeStatus, RestoreTarget, TrashKind},
    ids::{PurgeOperationId, SecurityAuditId, UserId, WorkspaceId},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter,
};

use crate::persistence::entities::lifecycle::{purge_operation, purge_operation_digest};

pub struct NewPurgeOperation {
    pub workspace_id: WorkspaceId,
    pub target: RestoreTarget,
    pub original_actor_user_id: UserId,
    pub commit_audit_id: SecurityAuditId,
}

pub struct PgPurgeOperationRepo;

impl PgPurgeOperationRepo {
    /// Creates the durable database-commit record before object cleanup begins.
    pub async fn create_in(
        &self,
        conn: &impl ConnectionTrait,
        operation: NewPurgeOperation,
    ) -> Result<PurgeOperation, DomainError> {
        let now = Utc::now();
        let id = PurgeOperationId::new();
        let model = purge_operation::ActiveModel {
            id: Set(id.0),
            workspace_id: Set(operation.workspace_id.0),
            target_kind: Set(operation.target.kind.as_str().into()),
            target_id: Set(operation.target.target_id),
            original_actor_user_id: Set(operation.original_actor_user_id.0),
            commit_audit_id: Set(operation.commit_audit_id.0),
            status: Set(PurgeStatus::DbCommitted.as_str().into()),
            attempts: Set(0),
            last_action: Set("resource.purge_committed".into()),
            last_executor_type: Set("user".into()),
            last_executor_id: Set(Some(operation.original_actor_user_id.0)),
            last_error: Set(None),
            last_attempt_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(conn)
        .await
        .map_err(db_err)?;

        purge_operation_from(model)
    }

    /// Persists the outcome of one cleanup attempt and returns its durable state.
    pub async fn record_attempt_in(
        &self,
        conn: &impl ConnectionTrait,
        operation_id: PurgeOperationId,
        status: PurgeStatus,
        action: &str,
        executor_type: &str,
        error: Option<String>,
    ) -> Result<PurgeOperation, DomainError> {
        let model = purge_operation::Entity::find_by_id(operation_id.0)
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "purge_operation",
                id: operation_id.0,
            })?;

        let attempts = model
            .attempts
            .checked_add(1)
            .ok_or_else(|| DomainError::Internal {
                message: "purge operation attempt count overflowed".into(),
            })?;
        let now = Utc::now();
        let updated = purge_operation::ActiveModel {
            status: Set(status.as_str().into()),
            attempts: Set(attempts),
            last_action: Set(action.into()),
            last_executor_type: Set(executor_type.into()),
            last_executor_id: Set(None),
            last_error: Set(error),
            last_attempt_at: Set(Some(now)),
            updated_at: Set(now),
            ..model.into_active_model()
        }
        .update(conn)
        .await
        .map_err(db_err)?;

        purge_operation_from(updated)
    }

    /// Adds a cleanup dependency whose attempts are tracked separately from the
    /// enclosing purge operation.
    pub async fn create_digest_in(
        &self,
        conn: &impl ConnectionTrait,
        operation_id: PurgeOperationId,
        digest: String,
    ) -> Result<PurgeDigest, DomainError> {
        let model = purge_operation_digest::ActiveModel {
            operation_id: Set(operation_id.0),
            digest: Set(digest),
            status: Set(PurgeStatus::DbCommitted.as_str().into()),
            attempts: Set(0),
            last_error: Set(None),
            last_attempt_at: Set(None),
        }
        .insert(conn)
        .await
        .map_err(db_err)?;

        purge_digest_from(model)
    }

    /// Records a digest-specific cleanup retry without changing its identity.
    pub async fn record_digest_attempt_in(
        &self,
        conn: &impl ConnectionTrait,
        operation_id: PurgeOperationId,
        digest: &str,
        status: PurgeStatus,
        error: Option<String>,
    ) -> Result<PurgeDigest, DomainError> {
        let model = purge_operation_digest::Entity::find()
            .filter(purge_operation_digest::Column::OperationId.eq(operation_id.0))
            .filter(purge_operation_digest::Column::Digest.eq(digest))
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "purge_operation_digest",
                id: operation_id.0,
            })?;

        let attempts = model
            .attempts
            .checked_add(1)
            .ok_or_else(|| DomainError::Internal {
                message: "purge digest attempt count overflowed".into(),
            })?;
        let updated = purge_operation_digest::ActiveModel {
            status: Set(status.as_str().into()),
            attempts: Set(attempts),
            last_error: Set(error),
            last_attempt_at: Set(Some(Utc::now())),
            ..model.into_active_model()
        }
        .update(conn)
        .await
        .map_err(db_err)?;

        purge_digest_from(updated)
    }
}

fn purge_operation_from(model: purge_operation::Model) -> Result<PurgeOperation, DomainError> {
    let kind = model
        .target_kind
        .parse::<TrashKind>()
        .map_err(|message| DomainError::Internal {
            message: format!("invalid stored purge target kind: {message}"),
        })?;
    let status = model
        .status
        .parse::<PurgeStatus>()
        .map_err(|message| DomainError::Internal {
            message: format!("invalid stored purge status: {message}"),
        })?;
    let attempts = u32::try_from(model.attempts).map_err(|_| DomainError::Internal {
        message: "stored purge attempts must not be negative".into(),
    })?;

    Ok(PurgeOperation {
        id: PurgeOperationId(model.id),
        workspace_id: WorkspaceId(model.workspace_id),
        target: RestoreTarget {
            kind,
            target_id: model.target_id,
        },
        original_actor_user_id: UserId(model.original_actor_user_id),
        commit_audit_id: SecurityAuditId(model.commit_audit_id),
        status,
        attempts,
        last_action: model.last_action,
        last_executor: model.last_executor_type,
        last_error: model.last_error,
        created_at: model.created_at,
        updated_at: model.updated_at,
    })
}

fn purge_digest_from(model: purge_operation_digest::Model) -> Result<PurgeDigest, DomainError> {
    let status = model
        .status
        .parse::<PurgeStatus>()
        .map_err(|message| DomainError::Internal {
            message: format!("invalid stored purge digest status: {message}"),
        })?;
    let attempts = u32::try_from(model.attempts).map_err(|_| DomainError::Internal {
        message: "stored purge digest attempts must not be negative".into(),
    })?;

    Ok(PurgeDigest {
        operation_id: PurgeOperationId(model.operation_id),
        digest: model.digest,
        status,
        attempts,
        last_error: model.last_error,
        last_attempt_at: model.last_attempt_at,
    })
}

fn db_err(error: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}
