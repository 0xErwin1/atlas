use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::lifecycle::{TrashItem, TrashKind},
    ids::WorkspaceId,
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, FromQueryResult, SqlErr, Statement, TransactionTrait,
};
use uuid::Uuid;

use crate::persistence::repos::PgSecurityAuditRepo;

pub struct TrashService {
    conn: DatabaseConnection,
}

impl TrashService {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    pub async fn list(
        &self,
        workspace_id: Option<WorkspaceId>,
        kind: Option<TrashKind>,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: u64,
    ) -> Result<Vec<TrashItem>, DomainError> {
        #[derive(FromQueryResult)]
        struct Row {
            workspace_id: Uuid,
            kind: String,
            target_id: Uuid,
            deleted_at: DateTime<Utc>,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();
        let mut filters = vec!["deleted_at IS NOT NULL".to_string()];
        if let Some(workspace_id) = workspace_id {
            values.push(workspace_id.0.into());
            filters.push(format!("workspace_id = ${}", values.len()));
        }
        if let Some(kind) = kind {
            values.push(kind.as_str().into());
            filters.push(format!("kind = ${}", values.len()));
        }
        if let Some((deleted_at, target_id)) = after {
            values.push(deleted_at.into());
            let timestamp = values.len();
            values.push(target_id.into());
            filters.push(format!(
                "(deleted_at, target_id) < (${}, ${})",
                timestamp,
                values.len()
            ));
        }
        values.push(
            i64::try_from(limit)
                .map_err(|_| DomainError::InvalidInput {
                    message: "trash limit is too large".into(),
                })?
                .into(),
        );

        let sql = format!(
            "SELECT workspace_id, kind, target_id, deleted_at FROM (\
             SELECT workspace_id, 'project'::text AS kind, id AS target_id, deleted_at FROM projects UNION ALL \
             SELECT workspace_id, 'folder'::text AS kind, id AS target_id, deleted_at FROM folders UNION ALL \
             SELECT workspace_id, 'document'::text AS kind, id AS target_id, deleted_at FROM documents UNION ALL \
             SELECT workspace_id, 'comment'::text AS kind, id AS target_id, deleted_at FROM comments UNION ALL \
             SELECT workspace_id, 'attachment'::text AS kind, id AS target_id, deleted_at FROM attachments\
             ) trash WHERE {} ORDER BY deleted_at DESC, target_id DESC LIMIT ${}",
            filters.join(" AND "),
            values.len(),
        );
        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;
        rows.into_iter()
            .map(|row| {
                Ok(TrashItem {
                    workspace_id: WorkspaceId(row.workspace_id),
                    kind: row.kind.parse::<TrashKind>().map_err(|message| {
                        DomainError::Internal {
                            message: message.into(),
                        }
                    })?,
                    target_id: row.target_id,
                    deleted_at: row.deleted_at,
                })
            })
            .collect()
    }

    pub async fn restore(
        &self,
        actor: atlas_domain::ids::UserId,
        kind: TrashKind,
        target_id: Uuid,
    ) -> Result<bool, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let table = table_for(kind);
        #[derive(FromQueryResult)]
        struct Row {
            workspace_id: Uuid,
            deleted_at: Option<DateTime<Utc>>,
        }
        let row = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT workspace_id, deleted_at FROM {table} WHERE id = $1 FOR UPDATE"),
            [target_id.into()],
        ))
        .one(&txn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: kind.as_str(),
            id: target_id,
        })?;
        let Some(deleted_at) = row.deleted_at else {
            txn.commit().await.map_err(db_err)?;
            return Ok(false);
        };
        let ctx = WorkspaceCtx::new(WorkspaceId(row.workspace_id), Actor::User(actor));
        self.ensure_restore_safe(&txn, &ctx, kind, target_id)
            .await?;
        txn.execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            format!("UPDATE {table} SET deleted_at = NULL, updated_at = now() WHERE id = $1"),
            [target_id.into()],
        ))
        .await
        .map_err(restore_db_err)?;
        if kind == TrashKind::Comment {
            txn.execute_raw(Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres, "UPDATE attachments SET deleted_at = NULL, updated_at = now() WHERE workspace_id = $1 AND comment_id = $2 AND deleted_at = $3", [ctx.workspace_id.0.into(), target_id.into(), deleted_at.into()])).await.map_err(db_err)?;
        }
        PgSecurityAuditRepo::append_resource_restored_in(&txn, &ctx, kind, target_id).await?;
        txn.commit().await.map_err(db_err)?;
        Ok(true)
    }

    async fn ensure_restore_safe(
        &self,
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        kind: TrashKind,
        id: Uuid,
    ) -> Result<(), DomainError> {
        let parent_sql = match kind {
            TrashKind::Project => "SELECT false AS exists",
            TrashKind::Folder => {
                "SELECT EXISTS (SELECT 1 FROM folders f WHERE f.workspace_id = $1 AND f.id = $2 AND ((f.project_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM projects p WHERE p.id = f.project_id AND p.deleted_at IS NULL)) OR (f.parent_folder_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM folders p WHERE p.id = f.parent_folder_id AND p.deleted_at IS NULL))))"
            }
            TrashKind::Document => {
                "SELECT EXISTS (SELECT 1 FROM documents d WHERE d.workspace_id = $1 AND d.id = $2 AND ((d.project_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM projects p WHERE p.id = d.project_id AND p.deleted_at IS NULL)) OR (d.folder_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM folders f WHERE f.id = d.folder_id AND f.deleted_at IS NULL))))"
            }
            TrashKind::Comment => {
                "SELECT EXISTS (SELECT 1 FROM comments c WHERE c.workspace_id = $1 AND c.id = $2 AND ((c.document_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM documents d WHERE d.id = c.document_id AND d.deleted_at IS NULL)) OR (c.task_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.id = c.task_id AND t.deleted_at IS NULL))))"
            }
            TrashKind::Attachment => {
                "SELECT EXISTS (SELECT 1 FROM attachments a WHERE a.workspace_id = $1 AND a.id = $2 AND ((a.document_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM documents d WHERE d.id = a.document_id AND d.deleted_at IS NULL)) OR (a.task_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.id = a.task_id AND t.deleted_at IS NULL)) OR (a.comment_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM comments c WHERE c.id = a.comment_id AND c.deleted_at IS NULL))))"
            }
        };
        let parent_blocked = restore_exists(conn, parent_sql, ctx, id).await?;
        if parent_blocked {
            return Err(DomainError::RestoreParentDeleted {
                kind: kind.as_str(),
            });
        }
        let identity_sql = match kind {
            TrashKind::Project => {
                "SELECT EXISTS (SELECT 1 FROM projects p WHERE p.workspace_id = $1 AND p.id <> $2 AND p.deleted_at IS NULL AND (p.slug = (SELECT slug FROM projects WHERE id = $2) OR p.task_prefix = (SELECT task_prefix FROM projects WHERE id = $2)))"
            }
            TrashKind::Folder => {
                "SELECT EXISTS (SELECT 1 FROM folders f JOIN folders other ON other.workspace_id = f.workspace_id AND other.id <> f.id AND other.deleted_at IS NULL AND other.project_id IS NOT DISTINCT FROM f.project_id AND other.parent_folder_id IS NOT DISTINCT FROM f.parent_folder_id AND other.name = f.name WHERE f.id = $2)"
            }
            TrashKind::Document => {
                "SELECT EXISTS (SELECT 1 FROM documents d JOIN documents other ON other.workspace_id = d.workspace_id AND other.id <> d.id AND other.deleted_at IS NULL AND other.slug IS NOT NULL AND other.slug = d.slug WHERE d.id = $2)"
            }
            TrashKind::Comment | TrashKind::Attachment => "SELECT false AS exists",
        };
        if restore_exists(conn, identity_sql, ctx, id).await? {
            return Err(DomainError::RestoreIdentityConflict {
                kind: kind.as_str(),
            });
        }
        Ok(())
    }
}

async fn restore_exists(
    conn: &impl ConnectionTrait,
    sql: &str,
    ctx: &WorkspaceCtx,
    id: Uuid,
) -> Result<bool, DomainError> {
    #[derive(FromQueryResult)]
    struct Exists {
        exists: bool,
    }
    Ok(Exists::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        sql,
        [ctx.workspace_id.0.into(), id.into()],
    ))
    .one(conn)
    .await
    .map_err(db_err)?
    .map(|row| row.exists)
    .unwrap_or(false))
}

fn table_for(kind: TrashKind) -> &'static str {
    match kind {
        TrashKind::Project => "projects",
        TrashKind::Folder => "folders",
        TrashKind::Document => "documents",
        TrashKind::Comment => "comments",
        TrashKind::Attachment => "attachments",
    }
}
fn db_err(error: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}

fn restore_db_err(error: sea_orm::DbErr) -> DomainError {
    if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
        return DomainError::RestoreIdentityConflict { kind: "resource" };
    }
    db_err(error)
}
