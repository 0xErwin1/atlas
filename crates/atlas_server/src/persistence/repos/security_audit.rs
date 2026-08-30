use async_trait::async_trait;
use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::lifecycle::TrashKind;
use atlas_acta::entities::task_views::ActorTypeFilter;
use atlas_core::error::DomainError;
use atlas_custos::entities::security_audit::AuditCursor;
use atlas_custos::entities::security_audit::AuditFilters;
use atlas_custos::entities::security_audit::NewSecurityAuditEvent;
use atlas_custos::entities::security_audit::SecurityAction;
use atlas_custos::entities::security_audit::SecurityAuditEvent;
use atlas_custos::ids::SecurityAuditId;
use atlas_custos::ports::security_audit::SecurityAuditRepo;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, FromQueryResult,
    Statement,
};

use crate::persistence::entities::security_audit::security_audit_log;
use atlas_postgres::db_err;

pub struct PgSecurityAuditRepo {
    pub conn: DatabaseConnection,
}

impl PgSecurityAuditRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Inserts one security audit event using the provided connection or transaction.
    ///
    /// `conn` accepts any `ConnectionTrait` implementor — `DatabaseConnection`,
    /// `DatabaseTransaction`, or `&DatabaseTransaction` — so the caller can pass the
    /// same handle that holds the mutation being audited. This guarantees the audit row
    /// is written iff the mutation commits (atomicity invariant).
    pub async fn append_in<C: ConnectionTrait>(
        conn: &C,
        event: NewSecurityAuditEvent,
    ) -> Result<(), DomainError> {
        let (actor_user_id, actor_api_key_id) = actor_columns(&event.actor);

        let model = security_audit_log::ActiveModel {
            id: Set(SecurityAuditId::new().0),
            workspace_id: Set(event.workspace_id.map(|w| w.0)),
            actor_user_id: Set(actor_user_id),
            actor_api_key_id: Set(actor_api_key_id),
            action: Set(event.action.as_str().to_string()),
            target_type: Set(event.target_type),
            target_id: Set(event.target_id),
            metadata: Set(event.metadata),
            created_at: Set(Utc::now()),
        };

        model.insert(conn).await.map_err(db_err)?;

        Ok(())
    }

    /// Appends the safe, normalized audit row for a committed lifecycle tombstone.
    pub async fn append_resource_deleted_in<C: ConnectionTrait>(
        conn: &C,
        ctx: &WorkspaceCtx,
        kind: TrashKind,
        target_id: uuid::Uuid,
    ) -> Result<(), DomainError> {
        Self::append_in(
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
        Self::append_in(
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

        model.insert(conn).await.map_err(db_err)?;
        Ok(id)
    }
}

#[async_trait]
impl SecurityAuditRepo for PgSecurityAuditRepo {
    async fn list_for_workspace(
        &self,
        ws: atlas_custos::WorkspaceScope,
        filters: &AuditFilters,
        cursor: Option<AuditCursor>,
        limit: u64,
    ) -> Result<Vec<SecurityAuditEvent>, DomainError> {
        #[derive(Debug, FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            workspace_id: Option<uuid::Uuid>,
            actor_user_id: Option<uuid::Uuid>,
            actor_api_key_id: Option<uuid::Uuid>,
            action: String,
            target_type: String,
            target_id: Option<uuid::Uuid>,
            metadata: serde_json::Value,
            created_at: chrono::DateTime<Utc>,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();

        values.push(ws.0.into());
        let ws_param = values.len();

        let actor_cond = if let Some(uid) = filters.actor_user_id {
            values.push(uid.0.into());
            format!("AND actor_user_id = ${}", values.len())
        } else {
            String::new()
        };

        let actor_type_cond = match filters.actor_type {
            Some(ActorTypeFilter::User) => "AND actor_user_id IS NOT NULL".to_string(),
            Some(ActorTypeFilter::ApiKey) => "AND actor_api_key_id IS NOT NULL".to_string(),
            None => String::new(),
        };

        let action_cond = if let Some(ref a) = filters.action {
            values.push(a.clone().into());
            format!("AND action = ${}", values.len())
        } else {
            String::new()
        };

        let from_cond = if let Some(from) = filters.from {
            values.push(from.into());
            format!("AND created_at >= ${}", values.len())
        } else {
            String::new()
        };

        let to_cond = if let Some(to) = filters.to {
            values.push(to.into());
            format!("AND created_at <= ${}", values.len())
        } else {
            String::new()
        };

        let cursor_cond = if let Some(c) = cursor {
            values.push(c.created_at.into());
            let ts_param = values.len();
            values.push(c.id.0.into());
            let id_param = values.len();
            format!("AND (created_at, id) < (${ts_param}, ${id_param})")
        } else {
            String::new()
        };

        values.push((limit as i64).into());
        let limit_param = values.len();

        let sql = format!(
            r#"
            SELECT id, workspace_id, actor_user_id, actor_api_key_id,
                   action, target_type, target_id, metadata, created_at
            FROM security_audit_log
            WHERE workspace_id = ${ws_param}
              {actor_cond}
              {actor_type_cond}
              {action_cond}
              {from_cond}
              {to_cond}
              {cursor_cond}
            ORDER BY created_at DESC, id DESC
            LIMIT ${limit_param}
            "#,
        );

        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows
            .into_iter()
            .map(|r| SecurityAuditEvent {
                id: SecurityAuditId(r.id),
                workspace_id: r.workspace_id.map(atlas_custos::WorkspaceScope),
                actor: actor_from_row(r.actor_user_id, r.actor_api_key_id),
                action: r.action,
                target_type: r.target_type,
                target_id: r.target_id,
                metadata: r.metadata,
                created_at: r.created_at,
            })
            .collect())
    }

    async fn list_platform(
        &self,
        filters: &AuditFilters,
        cursor: Option<AuditCursor>,
        limit: u64,
    ) -> Result<Vec<SecurityAuditEvent>, DomainError> {
        #[derive(Debug, FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            workspace_id: Option<uuid::Uuid>,
            actor_user_id: Option<uuid::Uuid>,
            actor_api_key_id: Option<uuid::Uuid>,
            action: String,
            target_type: String,
            target_id: Option<uuid::Uuid>,
            metadata: serde_json::Value,
            created_at: chrono::DateTime<Utc>,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();

        let actor_cond = if let Some(uid) = filters.actor_user_id {
            values.push(uid.0.into());
            format!("AND actor_user_id = ${}", values.len())
        } else {
            String::new()
        };

        let actor_type_cond = match filters.actor_type {
            Some(ActorTypeFilter::User) => "AND actor_user_id IS NOT NULL".to_string(),
            Some(ActorTypeFilter::ApiKey) => "AND actor_api_key_id IS NOT NULL".to_string(),
            None => String::new(),
        };

        let action_cond = if let Some(ref a) = filters.action {
            values.push(a.clone().into());
            format!("AND action = ${}", values.len())
        } else {
            String::new()
        };

        let from_cond = if let Some(from) = filters.from {
            values.push(from.into());
            format!("AND created_at >= ${}", values.len())
        } else {
            String::new()
        };

        let to_cond = if let Some(to) = filters.to {
            values.push(to.into());
            format!("AND created_at <= ${}", values.len())
        } else {
            String::new()
        };

        let cursor_cond = if let Some(c) = cursor {
            values.push(c.created_at.into());
            let ts_param = values.len();
            values.push(c.id.0.into());
            let id_param = values.len();
            format!("AND (created_at, id) < (${ts_param}, ${id_param})")
        } else {
            String::new()
        };

        values.push((limit as i64).into());
        let limit_param = values.len();

        let sql = format!(
            r#"
            SELECT id, workspace_id, actor_user_id, actor_api_key_id,
                   action, target_type, target_id, metadata, created_at
            FROM security_audit_log
            WHERE workspace_id IS NULL
              {actor_cond}
              {actor_type_cond}
              {action_cond}
              {from_cond}
              {to_cond}
              {cursor_cond}
            ORDER BY created_at DESC, id DESC
            LIMIT ${limit_param}
            "#,
        );

        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows
            .into_iter()
            .map(|r| SecurityAuditEvent {
                id: SecurityAuditId(r.id),
                workspace_id: r.workspace_id.map(atlas_custos::WorkspaceScope),
                actor: actor_from_row(r.actor_user_id, r.actor_api_key_id),
                action: r.action,
                target_type: r.target_type,
                target_id: r.target_id,
                metadata: r.metadata,
                created_at: r.created_at,
            })
            .collect())
    }
}

fn actor_columns(actor: &Actor) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match actor {
        Actor::User(uid) => (Some(uid.0), None),
        Actor::ApiKey(kid) => (None, Some(kid.0)),
    }
}

fn actor_from_row(user_id: Option<uuid::Uuid>, api_key_id: Option<uuid::Uuid>) -> Actor {
    use atlas_core::principal::UserId;
    match (user_id, api_key_id) {
        (Some(uid), None) => Actor::User(atlas_acta::actor::UserAttributionId(uid)),
        (None, Some(kid)) => Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(kid)),
        _ => Actor::User(atlas_acta::actor::UserAttributionId(UserId::new().0)),
    }
}

pub use atlas_custos::ports::security_audit::SecurityAuditRepo as SecurityAuditRepoTrait;

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::principal::ApiKeyId;
    use atlas_core::principal::UserId;

    #[test]
    fn user_actor_round_trips_through_xor_columns() {
        let actor = Actor::User(atlas_acta::actor::UserAttributionId(UserId::new().0));
        let (user_col, key_col) = actor_columns(&actor);

        assert_eq!(
            user_col,
            Some(match actor {
                Actor::User(uid) => uid.0,
                _ => unreachable!(),
            })
        );
        assert_eq!(key_col, None);
        assert_eq!(actor_from_row(user_col, key_col), actor);
    }

    #[test]
    fn api_key_actor_round_trips_through_xor_columns() {
        let actor = Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(ApiKeyId::new().0));
        let (user_col, key_col) = actor_columns(&actor);

        assert_eq!(user_col, None);
        assert_eq!(
            key_col,
            Some(match actor {
                Actor::ApiKey(kid) => kid.0,
                _ => unreachable!(),
            })
        );
        assert_eq!(actor_from_row(user_col, key_col), actor);
    }

    /// Known latent bug, out of scope for this slice, defect candidate post-E2:
    /// a `(None, None)` row silently becomes an attributed-to-a-random-user
    /// record. Pin the control flow only (returns `Actor::User(_)` without
    /// erroring); never assert the specific random uuid.
    #[test]
    fn none_none_row_falls_back_to_random_user_actor() {
        let actor = actor_from_row(None, None);

        assert!(matches!(actor, Actor::User(_)));
    }
}
