use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, SecurityAuditId,
    entities::security_audit::{
        AuditCursor, AuditFilters, NewSecurityAuditEvent, SecurityAuditEvent,
    },
    entities::task_views::ActorTypeFilter,
    ids::WorkspaceId,
    ports::security_audit::SecurityAuditRepo,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, FromQueryResult,
    Statement,
};

use crate::persistence::entities::security_audit::security_audit_log;

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
}

#[async_trait]
impl SecurityAuditRepo for PgSecurityAuditRepo {
    async fn list_for_workspace(
        &self,
        ws: WorkspaceId,
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
                workspace_id: r.workspace_id.map(WorkspaceId),
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
                workspace_id: r.workspace_id.map(WorkspaceId),
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
    use atlas_domain::ids::{ApiKeyId, UserId};
    match (user_id, api_key_id) {
        (Some(uid), None) => Actor::User(UserId(uid)),
        (None, Some(kid)) => Actor::ApiKey(ApiKeyId(kid)),
        _ => Actor::User(UserId::new()),
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}

pub use atlas_domain::ports::security_audit::SecurityAuditRepo as SecurityAuditRepoTrait;
