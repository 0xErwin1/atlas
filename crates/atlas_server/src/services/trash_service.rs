use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::lifecycle::PurgeExecutor;
use atlas_acta::entities::lifecycle::PurgeOperation;
use atlas_acta::entities::lifecycle::PurgeStatus;
use atlas_acta::entities::lifecycle::RestoreTarget;
use atlas_acta::entities::lifecycle::SecurityAuditRef;
use atlas_acta::entities::lifecycle::TrashItem;
use atlas_acta::entities::lifecycle::TrashKind;
use atlas_acta::ids::BoardId;
use atlas_acta::ids::DocumentId;
use atlas_acta::ids::FolderId;
use atlas_acta::ids::ProjectId;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::permissions::ResourceRef;
use atlas_acta::permissions::resource_ref_codec;
use atlas_acta::ports::attachment_store::AttachmentStore;
use atlas_core::error::DomainError;
use atlas_core::principal::UserId;
use chrono::{DateTime, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, FromQueryResult, SqlErr, Statement, TransactionTrait,
};
use uuid::Uuid;

use crate::persistence::repos::{
    NewPurgeOperation, PgAttachmentLifecycle, PgGrantHygiene, PgPurgeOperationRepo,
    append_resource_purge_committed_in, append_resource_restored_in,
};
use atlas_postgres::db_err;

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
             SELECT workspace_id, 'project'::text AS kind, id AS target_id, deleted_at FROM acta.projects UNION ALL \
             SELECT workspace_id, 'folder'::text AS kind, id AS target_id, deleted_at FROM acta.folders UNION ALL \
             SELECT workspace_id, 'document'::text AS kind, id AS target_id, deleted_at FROM acta.documents UNION ALL \
             SELECT workspace_id, 'comment'::text AS kind, id AS target_id, deleted_at FROM comments UNION ALL \
             SELECT workspace_id, 'attachment'::text AS kind, id AS target_id, deleted_at FROM acta.attachments\
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
        actor: atlas_core::principal::UserId,
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
        let ctx = WorkspaceCtx::new(
            WorkspaceId(row.workspace_id),
            Actor::User(atlas_acta::actor::UserAttributionId(actor.0)),
        );
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
            txn.execute_raw(Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres, "UPDATE acta.attachments SET deleted_at = NULL, updated_at = now() WHERE workspace_id = $1 AND comment_id = $2 AND deleted_at = $3", [ctx.workspace_id.0.into(), target_id.into(), deleted_at.into()])).await.map_err(db_err)?;
        }
        append_resource_restored_in(&txn, &ctx, kind, target_id).await?;
        txn.commit().await.map_err(db_err)?;
        Ok(true)
    }

    pub async fn purge(
        &self,
        actor: UserId,
        kind: TrashKind,
        target_id: Uuid,
    ) -> Result<PurgeOperation, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let target = RestoreTarget { kind, target_id };
        let table = table_for(kind);
        let operations = PgPurgeOperationRepo;

        if let Some(operation) = operations.find_any_target_in(&txn, &target).await? {
            txn.commit().await.map_err(db_err)?;
            return Ok(operation);
        }

        #[derive(FromQueryResult)]
        struct TargetRow {
            workspace_id: Uuid,
            deleted_at: Option<DateTime<Utc>>,
        }

        let row = TargetRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT workspace_id, deleted_at FROM {table} WHERE id = $1 FOR UPDATE"),
            [target_id.into()],
        ))
        .one(&txn)
        .await
        .map_err(db_err)?;

        let Some(row) = row else {
            return Err(DomainError::NotFound {
                entity: kind.as_str(),
                id: target_id,
            });
        };
        if row.deleted_at.is_none() {
            return Err(DomainError::InvalidInput {
                message: "only a deleted resource can be purged".into(),
            });
        }

        let ctx = WorkspaceCtx::new(
            WorkspaceId(row.workspace_id),
            Actor::User(atlas_acta::actor::UserAttributionId(actor.0)),
        );
        if let Some(operation) = operations
            .find_by_target_in(&txn, ctx.workspace_id, &target)
            .await?
        {
            txn.commit().await.map_err(db_err)?;
            return Ok(operation);
        }

        let digests = self.collect_purge_digests(&txn, kind, target_id).await?;
        let audit_id = append_resource_purge_committed_in(&txn, &ctx, kind, target_id).await?;
        let operation = operations
            .create_in(
                &txn,
                NewPurgeOperation {
                    workspace_id: ctx.workspace_id,
                    target: target.clone(),
                    original_actor_user_id: actor,
                    commit_audit_id: SecurityAuditRef(audit_id.0),
                },
            )
            .await?;

        for digest in digests {
            operations
                .create_digest_in(&txn, operation.id, digest)
                .await?;
        }

        self.delete_purge_closure(&txn, &ctx, kind, target_id)
            .await?;
        let pending = operations
            .record_attempt_in(
                &txn,
                operation.id,
                PurgeStatus::CleanupPending,
                PurgeExecutor::System,
                None,
            )
            .await?;
        txn.commit().await.map_err(db_err)?;
        Ok(pending)
    }

    /// Runs one durable cleanup pass for a committed purge operation.
    ///
    /// Database state is recorded before and after every object-store call. A
    /// process crash after deletion therefore retries an idempotent delete rather
    /// than falsely declaring the operation complete.
    pub async fn cleanup(
        &self,
        operation_id: atlas_acta::ids::PurgeOperationId,
        store: &dyn AttachmentStore,
    ) -> Result<PurgeOperation, DomainError> {
        let operations = PgPurgeOperationRepo;
        let operation = operations
            .find_by_id_in(&self.conn, operation_id)
            .await?
            .ok_or(DomainError::NotFound {
                entity: "purge_operation",
                id: operation_id.0,
            })?;
        if operation.status == PurgeStatus::Complete {
            return Ok(operation);
        }

        self.record_operation_attempt(operation_id, PurgeStatus::CleanupPending, None)
            .await?;
        let digests = operations.list_digests_in(&self.conn, operation_id).await?;
        let mut failed = false;

        for digest in digests
            .iter()
            .filter(|digest| digest.status != PurgeStatus::Complete)
        {
            self.record_digest_attempt(
                operation_id,
                &digest.digest,
                PurgeStatus::CleanupPending,
                None,
            )
            .await?;

            match PgAttachmentLifecycle::cleanup_committed_purge_digest(
                &self.conn,
                store,
                &digest.digest,
            )
            .await
            {
                Ok(_) => {
                    self.record_digest_attempt(
                        operation_id,
                        &digest.digest,
                        PurgeStatus::Complete,
                        None,
                    )
                    .await?;
                }
                Err(_) => {
                    failed = true;
                    self.record_digest_attempt(
                        operation_id,
                        &digest.digest,
                        PurgeStatus::CleanupFailed,
                        Some("attachment cleanup failed".into()),
                    )
                    .await?;
                }
            }
        }

        let all_complete = operations
            .list_digests_in(&self.conn, operation_id)
            .await?
            .iter()
            .all(|digest| digest.status == PurgeStatus::Complete);
        let status = if all_complete {
            PurgeStatus::Complete
        } else if failed {
            PurgeStatus::CleanupFailed
        } else {
            PurgeStatus::CleanupPending
        };
        self.record_operation_attempt(
            operation_id,
            status,
            (status == PurgeStatus::CleanupFailed).then(|| "attachment cleanup failed".into()),
        )
        .await
    }

    pub async fn reconcile(&self, store: &dyn AttachmentStore) -> Result<(), DomainError> {
        let operations = PgPurgeOperationRepo;
        for operation in operations
            .list_cleanup_candidates_in(&self.conn, 100)
            .await?
        {
            if let Err(error) = self.cleanup(operation.id, store).await {
                tracing::warn!(%error, operation_id = %operation.id.0, "purge cleanup reconciliation failed");
            }
        }
        Ok(())
    }

    async fn record_operation_attempt(
        &self,
        operation_id: atlas_acta::ids::PurgeOperationId,
        status: PurgeStatus,
        error: Option<String>,
    ) -> Result<PurgeOperation, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let operation = PgPurgeOperationRepo
            .record_attempt_in(&txn, operation_id, status, PurgeExecutor::System, error)
            .await?;
        txn.commit().await.map_err(db_err)?;
        Ok(operation)
    }

    async fn record_digest_attempt(
        &self,
        operation_id: atlas_acta::ids::PurgeOperationId,
        digest: &str,
        status: PurgeStatus,
        error: Option<String>,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        PgPurgeOperationRepo
            .record_digest_attempt_in(&txn, operation_id, digest, status, error)
            .await?;
        txn.commit().await.map_err(db_err)
    }

    async fn collect_purge_digests(
        &self,
        conn: &impl ConnectionTrait,
        kind: TrashKind,
        id: Uuid,
    ) -> Result<Vec<String>, DomainError> {
        #[derive(FromQueryResult)]
        struct DigestRow {
            sha256: String,
        }
        let scope = attachment_scope(kind);
        DigestRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT DISTINCT sha256 FROM acta.attachments WHERE {scope}"),
            [id.into()],
        ))
        .all(conn)
        .await
        .map_err(db_err)
        .map(|rows| rows.into_iter().map(|row| row.sha256).collect())
    }

    /// Runs the transitive hard-delete closure for `kind`/`id`, then revokes
    /// every grant targeting a row the closure deleted.
    ///
    /// Every `DELETE` on a grant-target table (documents, boards, folders,
    /// projects) returns the ids it removed (`RETURNING id`); the collected
    /// ids are encoded as resource refs and handed to `GrantHygiene` once,
    /// inside this same transaction (T6.4/T6.5). The O1 migration dropped the
    /// target FKs' `ON DELETE CASCADE`, so this call is what keeps a grant
    /// from outliving the resource it targets.
    async fn delete_purge_closure(
        &self,
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        kind: TrashKind,
        id: Uuid,
    ) -> Result<(), DomainError> {
        let mut revoked: Vec<atlas_core::ids::ResourceRef> = Vec::new();

        match kind {
            TrashKind::Project => {
                let document_ids = purge_documents_in(conn, "project_id = $1 OR folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE project_id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)", id).await?;
                extend_resource_refs(&mut revoked, ctx, document_ids, |i| {
                    ResourceRef::Document(DocumentId(i))
                });

                purge_tasks_in(conn, "project_id = $1", id).await?;

                let board_ids = execute_returning_ids(
                    conn,
                    "DELETE FROM acta.boards WHERE project_id = $1",
                    id,
                )
                .await?;
                extend_resource_refs(&mut revoked, ctx, board_ids, |i| {
                    ResourceRef::Board(BoardId(i))
                });

                let folder_ids = delete_folders_leaf_first(conn, "project_id = $1", id).await?;
                extend_resource_refs(&mut revoked, ctx, folder_ids, |i| {
                    ResourceRef::Folder(FolderId(i))
                });

                let project_ids =
                    execute_returning_ids(conn, "DELETE FROM acta.projects WHERE id = $1", id)
                        .await?;
                extend_resource_refs(&mut revoked, ctx, project_ids, |i| {
                    ResourceRef::Project(ProjectId(i))
                });
            }
            TrashKind::Folder => {
                let document_ids = purge_documents_in(conn, "folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)", id).await?;
                extend_resource_refs(&mut revoked, ctx, document_ids, |i| {
                    ResourceRef::Document(DocumentId(i))
                });

                purge_tasks_in(conn, "board_id IN (SELECT id FROM acta.boards WHERE folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure))", id).await?;

                let board_ids = execute_returning_ids(conn, "DELETE FROM acta.boards WHERE folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)", id).await?;
                extend_resource_refs(&mut revoked, ctx, board_ids, |i| {
                    ResourceRef::Board(BoardId(i))
                });

                let folder_ids = delete_folders_leaf_first(conn, "id = $1", id).await?;
                extend_resource_refs(&mut revoked, ctx, folder_ids, |i| {
                    ResourceRef::Folder(FolderId(i))
                });
            }
            TrashKind::Document => {
                let document_ids = purge_documents_in(conn, "id = $1", id).await?;
                extend_resource_refs(&mut revoked, ctx, document_ids, |i| {
                    ResourceRef::Document(DocumentId(i))
                });
            }
            TrashKind::Comment => purge_comments_in(conn, "id = $1", id).await?,
            TrashKind::Attachment => purge_attachments_in(conn, "id = $1", id).await?,
        }

        if !revoked.is_empty() {
            PgGrantHygiene::revoke_grants_for_in(conn, &revoked).await?;
        }

        Ok(())
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
                "SELECT EXISTS (SELECT 1 FROM acta.folders f WHERE f.workspace_id = $1 AND f.id = $2 AND ((f.project_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM acta.projects p WHERE p.id = f.project_id AND p.deleted_at IS NULL)) OR (f.parent_folder_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM acta.folders p WHERE p.id = f.parent_folder_id AND p.deleted_at IS NULL))))"
            }
            TrashKind::Document => {
                "SELECT EXISTS (SELECT 1 FROM acta.documents d WHERE d.workspace_id = $1 AND d.id = $2 AND ((d.project_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM acta.projects p WHERE p.id = d.project_id AND p.deleted_at IS NULL)) OR (d.folder_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM acta.folders f WHERE f.id = d.folder_id AND f.deleted_at IS NULL))))"
            }
            TrashKind::Comment => {
                "SELECT EXISTS (SELECT 1 FROM comments c WHERE c.workspace_id = $1 AND c.id = $2 AND ((c.document_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM acta.documents d WHERE d.id = c.document_id AND d.deleted_at IS NULL)) OR (c.task_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM acta.tasks t WHERE t.id = c.task_id AND t.deleted_at IS NULL))))"
            }
            TrashKind::Attachment => {
                "SELECT EXISTS (SELECT 1 FROM acta.attachments a WHERE a.workspace_id = $1 AND a.id = $2 AND ((a.document_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM acta.documents d WHERE d.id = a.document_id AND d.deleted_at IS NULL)) OR (a.task_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM acta.tasks t WHERE t.id = a.task_id AND t.deleted_at IS NULL)) OR (a.comment_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM comments c WHERE c.id = a.comment_id AND c.deleted_at IS NULL))))"
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
                "SELECT EXISTS (SELECT 1 FROM acta.projects p WHERE p.workspace_id = $1 AND p.id <> $2 AND p.deleted_at IS NULL AND (p.slug = (SELECT slug FROM acta.projects WHERE id = $2) OR p.task_prefix = (SELECT task_prefix FROM acta.projects WHERE id = $2)))"
            }
            TrashKind::Folder => {
                "SELECT EXISTS (SELECT 1 FROM acta.folders f JOIN acta.folders other ON other.workspace_id = f.workspace_id AND other.id <> f.id AND other.deleted_at IS NULL AND other.project_id IS NOT DISTINCT FROM f.project_id AND other.parent_folder_id IS NOT DISTINCT FROM f.parent_folder_id AND other.name = f.name WHERE f.id = $2)"
            }
            TrashKind::Document => {
                "SELECT EXISTS (SELECT 1 FROM acta.documents d JOIN acta.documents other ON other.workspace_id = d.workspace_id AND other.id <> d.id AND other.deleted_at IS NULL AND other.slug IS NOT NULL AND other.slug = d.slug WHERE d.id = $2)"
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

fn attachment_scope(kind: TrashKind) -> &'static str {
    match kind {
        TrashKind::Project => {
            "document_id IN (SELECT id FROM acta.documents WHERE project_id = $1 OR folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE project_id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)) OR task_id IN (SELECT id FROM acta.tasks WHERE project_id = $1) OR comment_id IN (SELECT id FROM comments WHERE document_id IN (SELECT id FROM acta.documents WHERE project_id = $1 OR folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE project_id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)) OR task_id IN (SELECT id FROM acta.tasks WHERE project_id = $1)) OR draft_id IN (SELECT id FROM acta.comment_attachment_drafts WHERE document_id IN (SELECT id FROM acta.documents WHERE project_id = $1 OR folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE project_id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)) OR task_id IN (SELECT id FROM acta.tasks WHERE project_id = $1))"
        }
        TrashKind::Folder => {
            "document_id IN (SELECT id FROM acta.documents WHERE folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)) OR task_id IN (SELECT t.id FROM acta.tasks t JOIN acta.boards b ON b.id = t.board_id WHERE b.folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)) OR comment_id IN (SELECT id FROM comments WHERE document_id IN (SELECT id FROM acta.documents WHERE folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)) OR task_id IN (SELECT t.id FROM acta.tasks t JOIN acta.boards b ON b.id = t.board_id WHERE b.folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure))) OR draft_id IN (SELECT id FROM acta.comment_attachment_drafts WHERE document_id IN (SELECT id FROM acta.documents WHERE folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)) OR task_id IN (SELECT t.id FROM acta.tasks t JOIN acta.boards b ON b.id = t.board_id WHERE b.folder_id IN (WITH RECURSIVE folders_in_closure AS (SELECT id FROM acta.folders WHERE id = $1 UNION ALL SELECT f.id FROM acta.folders f JOIN folders_in_closure c ON f.parent_folder_id = c.id) SELECT id FROM folders_in_closure)))"
        }
        TrashKind::Document => {
            "document_id = $1 OR comment_id IN (SELECT id FROM comments WHERE document_id = $1) OR draft_id IN (SELECT id FROM acta.comment_attachment_drafts WHERE document_id = $1)"
        }
        TrashKind::Comment => {
            "comment_id = $1 OR draft_id IN (SELECT id FROM acta.comment_attachment_drafts WHERE finalized_comment_id = $1)"
        }
        TrashKind::Attachment => "id = $1",
    }
}

async fn execute(conn: &impl ConnectionTrait, sql: &str, id: Uuid) -> Result<u64, DomainError> {
    conn.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        sql,
        [id.into()],
    ))
    .await
    .map(|result| result.rows_affected())
    .map_err(db_err)
}

/// Runs a `DELETE` and returns the ids of the rows it removed.
///
/// `sql` must not already contain a `RETURNING` clause. Used for every
/// grant-target table (documents, boards, folders, projects) so the caller
/// can revoke exactly the grants that targeted a now-deleted row (T6.4/T6.5).
async fn execute_returning_ids(
    conn: &impl ConnectionTrait,
    sql: &str,
    id: Uuid,
) -> Result<Vec<Uuid>, DomainError> {
    #[derive(FromQueryResult)]
    struct IdRow {
        id: Uuid,
    }

    IdRow::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        format!("{sql} RETURNING id"),
        [id.into()],
    ))
    .all(conn)
    .await
    .map(|rows| rows.into_iter().map(|row| row.id).collect())
    .map_err(db_err)
}

/// Encodes each deleted row id as a resource ref via `encode` and appends it
/// to `revoked`, scoped to the closure's own workspace.
fn extend_resource_refs(
    revoked: &mut Vec<atlas_core::ids::ResourceRef>,
    ctx: &WorkspaceCtx,
    ids: Vec<Uuid>,
    encode: impl Fn(Uuid) -> ResourceRef,
) {
    revoked.extend(
        ids.into_iter()
            .map(|id| resource_ref_codec::to_core(&encode(id), ctx.workspace_id)),
    );
}

async fn purge_attachments_in(
    conn: &impl ConnectionTrait,
    scope: &str,
    id: Uuid,
) -> Result<(), DomainError> {
    execute(
        conn,
        &format!(
            "DELETE FROM acta.comment_attachment_draft_uploads WHERE original_attachment_id IN (SELECT id FROM acta.attachments WHERE {scope}) OR attachment_id IN (SELECT id FROM acta.attachments WHERE {scope})"
        ),
        id,
    )
    .await?;
    execute(
        conn,
        &format!("DELETE FROM acta.attachments WHERE {scope}"),
        id,
    )
    .await?;
    Ok(())
}

async fn purge_comments_in(
    conn: &impl ConnectionTrait,
    scope: &str,
    id: Uuid,
) -> Result<(), DomainError> {
    purge_drafts_in(
        conn,
        &format!("finalized_comment_id IN (SELECT id FROM comments WHERE {scope})"),
        id,
    )
    .await?;
    purge_attachments_in(
        conn,
        &format!("comment_id IN (SELECT id FROM comments WHERE {scope})"),
        id,
    )
    .await?;
    execute(conn, &format!("DELETE FROM comments WHERE {scope}"), id).await?;
    Ok(())
}

async fn purge_documents_in(
    conn: &impl ConnectionTrait,
    scope: &str,
    id: Uuid,
) -> Result<Vec<Uuid>, DomainError> {
    purge_comments_in(
        conn,
        &format!("document_id IN (SELECT id FROM acta.documents WHERE {scope})"),
        id,
    )
    .await?;
    purge_attachments_in(
        conn,
        &format!("document_id IN (SELECT id FROM acta.documents WHERE {scope})"),
        id,
    )
    .await?;
    purge_drafts_in(
        conn,
        &format!("document_id IN (SELECT id FROM acta.documents WHERE {scope})"),
        id,
    )
    .await?;
    execute_returning_ids(
        conn,
        &format!("DELETE FROM acta.documents WHERE {scope}"),
        id,
    )
    .await
}

async fn purge_tasks_in(
    conn: &impl ConnectionTrait,
    scope: &str,
    id: Uuid,
) -> Result<(), DomainError> {
    purge_comments_in(
        conn,
        &format!("task_id IN (SELECT id FROM acta.tasks WHERE {scope})"),
        id,
    )
    .await?;
    purge_attachments_in(
        conn,
        &format!("task_id IN (SELECT id FROM acta.tasks WHERE {scope})"),
        id,
    )
    .await?;
    purge_drafts_in(
        conn,
        &format!("task_id IN (SELECT id FROM acta.tasks WHERE {scope})"),
        id,
    )
    .await?;
    execute(
        conn,
        &format!(
            "DELETE FROM acta.task_references WHERE source_task_id IN (SELECT id FROM acta.tasks WHERE {scope}) OR target_task_id IN (SELECT id FROM acta.tasks WHERE {scope})"
        ),
        id,
    )
    .await?;
    execute(conn, &format!("DELETE FROM acta.tasks WHERE {scope}"), id).await?;
    Ok(())
}

async fn purge_drafts_in(
    conn: &impl ConnectionTrait,
    scope: &str,
    id: Uuid,
) -> Result<(), DomainError> {
    let draft_scope =
        format!("draft_id IN (SELECT id FROM acta.comment_attachment_drafts WHERE {scope})");
    purge_attachments_in(conn, &draft_scope, id).await?;
    execute(
        conn,
        &format!(
            "DELETE FROM acta.comment_attachment_draft_uploads WHERE draft_id IN (SELECT id FROM acta.comment_attachment_drafts WHERE {scope})"
        ),
        id,
    )
    .await?;
    execute(
        conn,
        &format!("DELETE FROM acta.comment_attachment_drafts WHERE {scope}"),
        id,
    )
    .await?;
    Ok(())
}

async fn delete_folders_leaf_first(
    conn: &impl ConnectionTrait,
    roots: &str,
    id: Uuid,
) -> Result<Vec<Uuid>, DomainError> {
    let mut deleted_ids = Vec::new();
    loop {
        let batch = execute_returning_ids(
            conn,
            &format!(
                "WITH RECURSIVE closure AS (SELECT id FROM acta.folders WHERE {roots} UNION ALL SELECT child.id FROM acta.folders child JOIN closure parent ON child.parent_folder_id = parent.id) DELETE FROM acta.folders f WHERE f.id IN (SELECT id FROM closure) AND NOT EXISTS (SELECT 1 FROM acta.folders child WHERE child.parent_folder_id = f.id AND child.id IN (SELECT id FROM closure))"
            ),
            id,
        )
        .await?;
        if batch.is_empty() {
            break;
        }
        deleted_ids.extend(batch);
    }
    Ok(deleted_ids)
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
        TrashKind::Project => "acta.projects",
        TrashKind::Folder => "acta.folders",
        TrashKind::Document => "acta.documents",
        TrashKind::Comment => "comments",
        TrashKind::Attachment => "acta.attachments",
    }
}

fn restore_db_err(error: sea_orm::DbErr) -> DomainError {
    if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
        return DomainError::RestoreIdentityConflict { kind: "resource" };
    }
    db_err(error)
}
