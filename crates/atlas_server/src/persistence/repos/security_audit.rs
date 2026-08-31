use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::lifecycle::TrashKind;
use atlas_core::error::DomainError;
use atlas_custos::entities::security_audit::NewSecurityAuditEvent;
use atlas_custos::entities::security_audit::SecurityAction;
use atlas_custos::ids::SecurityAuditId;
use atlas_custos_postgres::entities::security_audit::security_audit_log;
use atlas_custos_postgres::repos::security_audit::PgSecurityAuditRepo;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait};

// The core Custos audit-log repo (`PgSecurityAuditRepo`, its
// `SecurityAuditRepo` trait impl, and `append_in`) lives in
// `atlas_custos_postgres`; call sites import it directly. These three
// functions compose a Custos audit append with Acta context (`WorkspaceCtx`,
// `TrashKind`) and stay here: the crate they'd otherwise move into must not
// depend on `atlas_acta`.

/// Appends the safe, normalized audit row for a committed lifecycle tombstone.
pub async fn append_resource_deleted_in<C: ConnectionTrait>(
    conn: &C,
    ctx: &WorkspaceCtx,
    kind: TrashKind,
    target_id: uuid::Uuid,
) -> Result<(), DomainError> {
    PgSecurityAuditRepo::append_in(
        conn,
        NewSecurityAuditEvent {
            workspace_id: Some(atlas_custos::WorkspaceScope(ctx.workspace_id.0)),
            actor: ctx.actor,
            action: SecurityAction::ResourceDeleted,
            target_type: kind.as_str().to_string(),
            target_id: Some(target_id),
            metadata: serde_json::json!({
                "kind": kind.as_str(),
                "outcome": "deleted",
            }),
        },
    )
    .await
}

pub async fn append_resource_restored_in<C: ConnectionTrait>(
    conn: &C,
    ctx: &WorkspaceCtx,
    kind: TrashKind,
    target_id: uuid::Uuid,
) -> Result<(), DomainError> {
    PgSecurityAuditRepo::append_in(
        conn,
        NewSecurityAuditEvent {
            workspace_id: Some(atlas_custos::WorkspaceScope(ctx.workspace_id.0)),
            actor: ctx.actor,
            action: SecurityAction::ResourceRestored,
            target_type: kind.as_str().to_string(),
            target_id: Some(target_id),
            metadata: serde_json::json!({"kind": kind.as_str(), "outcome": "restored"}),
        },
    )
    .await
}

pub async fn append_resource_purge_committed_in<C: ConnectionTrait>(
    conn: &C,
    ctx: &WorkspaceCtx,
    kind: TrashKind,
    target_id: uuid::Uuid,
) -> Result<SecurityAuditId, DomainError> {
    let id = SecurityAuditId::new();
    let model = security_audit_log::ActiveModel {
        id: Set(id.0),
        workspace_id: Set(Some(ctx.workspace_id.0)),
        actor_user_id: Set(match ctx.actor {
            Actor::User(user_id) => Some(user_id.0),
            Actor::ApiKey(_) => None,
        }),
        actor_api_key_id: Set(None),
        action: Set(SecurityAction::ResourcePurgeCommitted.as_str().to_string()),
        target_type: Set(kind.as_str().to_string()),
        target_id: Set(Some(target_id)),
        metadata: Set(serde_json::json!({
            "kind": kind.as_str(),
            "outcome": "db_committed",
        })),
        created_at: Set(Utc::now()),
    };

    model.insert(conn).await.map_err(atlas_postgres::db_err)?;
    Ok(id)
}
