use async_trait::async_trait;
use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::comments::CommentOwner;
use atlas_acta::entities::comments::NewCommentAttachmentDraftUpload;
use atlas_acta::entities::documents::Attachment;
use atlas_acta::entities::documents::AttachmentOwner;
use atlas_acta::entities::documents::AttachmentWriteIntent;
use atlas_acta::entities::documents::NewAttachment;
use atlas_acta::entities::lifecycle::TrashKind;
use atlas_acta::ids::AttachmentId;
use atlas_acta::ids::CommentDraftId;
use atlas_acta::ids::DocumentId;
use atlas_acta::ids::TaskId;
use atlas_acta::ports::attachment_store::AttachmentStore;
use atlas_core::error::DomainError;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Statement,
    TransactionTrait,
};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, pool::PoolConnection};
use uuid::Uuid;

use crate::persistence::entities::comments::{
    comment_attachment_draft, comment_attachment_draft_upload,
};
use crate::persistence::entities::documents::{
    attachment, attachment_from, attachment_write_intent, attachment_write_intent_from,
};
use crate::persistence::live_ancestors::{
    live_comment_chain, live_document_chain, live_task_chain,
};
use crate::persistence::repos::comment_attachment_drafts::{
    lock_active_draft_for_upload, record_upload_or_replay_in,
};
use crate::persistence::repos::{PgSearchIndexQueueRepo, append_resource_deleted_in};
use atlas_postgres::db_err;

// R1 scaffolding: `DocumentRepo`/`DocumentLinkRepo` (`PgDocumentRepo`,
// `PgDocumentLinkRepo`, their `*_in` transaction-scoped helpers) now live in
// `atlas_acta_postgres::repos::documents` (S4 PR7). Re-exporting them here
// keeps every existing `crate::persistence::repos::*` call site unaffected.
//
// `PgAttachmentRepo`, `PgAttachmentWriteIntentRepo`, and
// `PgAttachmentLifecycle` stay here: their methods compose a Custos
// security-audit append (`append_resource_deleted_in`) with Acta context,
// the same boundary `security_audit.rs` already documents.
pub use atlas_acta_postgres::repos::documents::{
    DocumentLinkRepo, DocumentRepo, PgDocumentLinkRepo, PgDocumentRepo, create_in, edit_content_in,
    move_to_in, rename_in, soft_delete_in, update_content_in,
};

pub use atlas_acta::ports::documents::AttachmentRepo;
pub use atlas_acta::ports::documents::AttachmentWriteIntentRepo;

const ATTACHMENT_STORE_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

struct DigestSessionLock {
    connection: PoolConnection<Postgres>,
    digest: String,
}

impl DigestSessionLock {
    async fn acquire(conn: &DatabaseConnection, digest: &str) -> Result<Self, DomainError> {
        let mut connection = conn
            .get_postgres_connection_pool()
            .acquire()
            .await
            .map_err(sqlx_err)?;

        sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
            .bind(digest)
            .execute(&mut *connection)
            .await
            .map_err(sqlx_err)?;

        Ok(Self {
            connection,
            digest: digest.into(),
        })
    }

    async fn release(mut self) -> Result<(), DomainError> {
        let unlocked: bool =
            sqlx::query_scalar("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
                .bind(&self.digest)
                .fetch_one(&mut *self.connection)
                .await
                .map_err(sqlx_err)?;

        if unlocked {
            Ok(())
        } else {
            Err(DomainError::Internal {
                message: "attachment digest session lock was not held".into(),
            })
        }
    }
}

pub struct PgAttachmentRepo {
    pub conn: DatabaseConnection,
}

impl PgAttachmentRepo {
    /// Restores one attachment only when it still has the expected lifecycle tombstone.
    pub async fn restore_at_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        id: AttachmentId,
        deleted_at: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let row = attachment::Entity::find_by_id(id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DeletedAt.eq(deleted_at))
            .lock_exclusive()
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "attachment",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(None);
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;
        Ok(())
    }

    /// Renames an attachment only when it belongs to the supplied workspace and owner.
    ///
    /// Owner mismatches are concealed as not-found, matching attachment read/delete
    /// behavior at the API boundary. Only metadata timestamps and the normalized file
    /// name are updated; the content-addressed object key remains unchanged.
    pub async fn rename_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        id: AttachmentId,
        owner: AttachmentOwner,
        file_name: String,
    ) -> Result<Attachment, DomainError> {
        let query = attachment::Entity::find_by_id(id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .filter(live_document_chain("attachments.document_id"))
            .filter(live_task_chain("attachments.task_id"))
            .filter(live_comment_chain("attachments.comment_id"));

        let query = match owner {
            AttachmentOwner::Document(document_id) => {
                query.filter(attachment::Column::DocumentId.eq(document_id.0))
            }
            AttachmentOwner::Task(task_id) => {
                query.filter(attachment::Column::TaskId.eq(task_id.0))
            }
            AttachmentOwner::Comment(comment_id) => {
                query.filter(attachment::Column::CommentId.eq(comment_id.0))
            }
            AttachmentOwner::Draft(draft_id) => {
                query.filter(attachment::Column::DraftId.eq(draft_id.0))
            }
        };

        let row = query
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "attachment",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.file_name = Set(file_name);
        active.updated_at = Set(Utc::now());
        let renamed = active
            .update(&self.conn)
            .await
            .map(attachment_from)
            .map_err(db_err)?;

        enqueue_attachment_owner(&self.conn, ctx, renamed.document_id, renamed.task_id).await?;

        Ok(renamed)
    }
}

#[async_trait]
impl AttachmentRepo for PgAttachmentRepo {
    async fn record(
        &self,
        ctx: &WorkspaceCtx,
        new: NewAttachment,
    ) -> Result<Attachment, DomainError> {
        let (by_user, by_key) = actor_fields(&ctx.actor);
        let model = attachment::ActiveModel {
            id: Set(AttachmentId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            document_id: Set(new.document_id.map(|id| id.0)),
            task_id: Set(new.task_id.map(|id| id.0)),
            comment_id: Set(new.comment_id.map(|id| id.0)),
            draft_id: Set(None),
            file_name: Set(new.file_name),
            content_type: Set(new.content_type),
            size_bytes: Set(new.size_bytes),
            sha256: Set(new.sha256),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        let stored = model
            .insert(&self.conn)
            .await
            .map(attachment_from)
            .map_err(db_err)?;

        enqueue_attachment_owner(&self.conn, ctx, stored.document_id, stored.task_id).await?;

        Ok(stored)
    }

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: AttachmentId,
    ) -> Result<Option<Attachment>, DomainError> {
        attachment::Entity::find_by_id(id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .filter(live_document_chain("attachments.document_id"))
            .filter(live_task_chain("attachments.task_id"))
            .filter(live_comment_chain("attachments.comment_id"))
            .one(&self.conn)
            .await
            .map(|opt| opt.map(attachment_from))
            .map_err(db_err)
    }

    async fn list_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: AttachmentOwner,
    ) -> Result<Vec<Attachment>, DomainError> {
        let q = attachment::Entity::find()
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .filter(live_document_chain("attachments.document_id"))
            .filter(live_task_chain("attachments.task_id"))
            .filter(live_comment_chain("attachments.comment_id"));

        let rows = match owner {
            AttachmentOwner::Document(doc_id) => q
                .filter(attachment::Column::DocumentId.eq(doc_id.0))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
            AttachmentOwner::Task(task_id) => q
                .filter(attachment::Column::TaskId.eq(task_id.0))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
            AttachmentOwner::Comment(comment_id) => q
                .filter(attachment::Column::CommentId.eq(comment_id.0))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
            AttachmentOwner::Draft(draft_id) => q
                .filter(attachment::Column::DraftId.eq(draft_id.0))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
        };

        Ok(rows.into_iter().map(attachment_from).collect())
    }

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: AttachmentId) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = attachment::Entity::find_by_id(id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .lock_exclusive()
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "attachment",
                id: id.0,
            })?;

        let owner_document_id = row.document_id;
        let owner_task_id = row.task_id;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&txn).await.map_err(db_err)?;

        append_resource_deleted_in(&txn, ctx, TrashKind::Attachment, id.0).await?;

        enqueue_attachment_owner(
            &txn,
            ctx,
            owner_document_id.map(DocumentId),
            owner_task_id.map(TaskId),
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }
}

/// Marks the document or task that owns an attachment as needing re-indexing.
///
/// Comment- and draft-owned attachments are skipped on purpose: only attachments
/// hanging directly off a document or task contribute their file name to that
/// resource's indexed text.
async fn enqueue_attachment_owner(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    document_id: Option<DocumentId>,
    task_id: Option<TaskId>,
) -> Result<(), DomainError> {
    if let Some(document_id) = document_id {
        PgSearchIndexQueueRepo::enqueue_document_in(conn, ctx.workspace_id, document_id).await?;
    }
    if let Some(task_id) = task_id {
        PgSearchIndexQueueRepo::enqueue_task_in(conn, ctx.workspace_id, task_id).await?;
    }
    Ok(())
}

pub struct PgAttachmentWriteIntentRepo {
    pub conn: DatabaseConnection,
}

pub struct PgAttachmentLifecycle;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DraftReconciliationReport {
    pub claimed_expiries: u64,
    pub failed_expiries: u64,
    pub pruned: u64,
    pub failed_prunes: u64,
    pub cleanup_failed: u64,
    pub expired_backlog: u64,
    pub terminal_backlog: u64,
}

impl PgAttachmentLifecycle {
    pub async fn list_active_draft_attachments(
        conn: &DatabaseConnection,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        draft_id: CommentDraftId,
    ) -> Result<Vec<Attachment>, DomainError> {
        let txn = conn.begin().await.map_err(db_err)?;
        lock_active_draft_for_upload(&txn, ctx, owner, draft_id).await?;

        let rows = attachment::Entity::find()
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DraftId.eq(draft_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .all(&txn)
            .await
            .map_err(db_err)?;

        txn.commit().await.map_err(db_err)?;
        Ok(rows.into_iter().map(attachment_from).collect())
    }

    pub async fn find_active_draft_attachment(
        conn: &DatabaseConnection,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        draft_id: CommentDraftId,
        attachment_id: AttachmentId,
    ) -> Result<Attachment, DomainError> {
        let txn = conn.begin().await.map_err(db_err)?;
        lock_active_draft_for_upload(&txn, ctx, owner, draft_id).await?;

        let attachment = attachment::Entity::find_by_id(attachment_id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DraftId.eq(draft_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?;

        let Some(attachment) = attachment else {
            let tombstoned = comment_attachment_draft_upload::Entity::find()
                .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
                .filter(
                    comment_attachment_draft_upload::Column::OriginalAttachmentId
                        .eq(attachment_id.0),
                )
                .one(&txn)
                .await
                .map_err(db_err)?
                .is_some();

            return Err(if tombstoned {
                DomainError::CommentDraftGone {
                    reason: "draft attachment was deleted".into(),
                }
            } else {
                DomainError::NotFound {
                    entity: "draft attachment",
                    id: attachment_id.0,
                }
            });
        };

        txn.commit().await.map_err(db_err)?;
        Ok(attachment_from(attachment))
    }

    pub async fn is_tombstoned_draft_attachment(
        conn: &DatabaseConnection,
        draft_id: CommentDraftId,
        attachment_id: AttachmentId,
    ) -> Result<bool, DomainError> {
        let upload = comment_attachment_draft_upload::Entity::find()
            .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
            .filter(
                comment_attachment_draft_upload::Column::OriginalAttachmentId.eq(attachment_id.0),
            )
            .one(conn)
            .await
            .map_err(db_err)?;

        let Some(upload) = upload else {
            return Ok(false);
        };

        attachment::Entity::find_by_id(attachment_id.0)
            .filter(attachment::Column::DeletedAt.is_not_null())
            .one(conn)
            .await
            .map(|attachment| upload.deleted_at.is_some() || attachment.is_some())
            .map_err(db_err)
    }

    pub async fn cancel_draft(
        conn: &DatabaseConnection,
        ctx: &WorkspaceCtx,
        draft_id: CommentDraftId,
        store: &dyn AttachmentStore,
    ) -> Result<(), DomainError> {
        let txn = conn.begin().await.map_err(db_err)?;
        let draft = crate::persistence::entities::comments::comment_attachment_draft::Entity::find_by_id(draft_id.0)
            .filter(crate::persistence::entities::comments::comment_attachment_draft::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .lock_exclusive()
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound { entity: "comment attachment draft", id: draft_id.0 })?;

        match draft.state.as_str() {
            "active" => {}
            "finalized" => {
                return Err(DomainError::CommentDraftConflict {
                    reason: "draft was finalized".into(),
                });
            }
            _ => {
                return Err(DomainError::CommentDraftGone {
                    reason: "draft is no longer active".into(),
                });
            }
        }

        let attachments = attachment::Entity::find()
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DraftId.eq(draft_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .all(&txn)
            .await
            .map_err(db_err)?;
        let digests = attachments
            .iter()
            .map(|attachment| attachment.sha256.clone())
            .collect::<std::collections::BTreeSet<_>>();

        for digest in &digests {
            txn.execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "INSERT INTO acta.attachment_write_intents (id, digest, created_at) VALUES ($1, $2, now()) ON CONFLICT (digest) DO NOTHING",
                [Uuid::now_v7().into(), digest.clone().into()],
            )).await.map_err(db_err)?;
        }

        comment_attachment_draft_upload::Entity::update_many()
            .col_expr(
                comment_attachment_draft_upload::Column::AttachmentId,
                sea_orm::sea_query::Expr::value(None::<Uuid>),
            )
            .col_expr(
                comment_attachment_draft_upload::Column::DeletedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .col_expr(
                comment_attachment_draft_upload::Column::UpdatedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
            .filter(comment_attachment_draft_upload::Column::DeletedAt.is_null())
            .exec(&txn)
            .await
            .map_err(db_err)?;

        attachment::Entity::update_many()
            .col_expr(
                attachment::Column::DeletedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .col_expr(
                attachment::Column::UpdatedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DraftId.eq(draft_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .exec(&txn)
            .await
            .map_err(db_err)?;

        let mut draft = draft.into_active_model();
        draft.state = Set("cancelled".into());
        draft.terminal_at = Set(Some(Utc::now()));
        draft.updated_at = Set(Utc::now());
        draft.update(&txn).await.map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;

        for digest in digests {
            if let Err(error) = Self::finish_purge_digest(conn, store, &digest).await {
                tracing::warn!(%error, %digest, "cancelled draft attachment cleanup will be retried");
            }
        }
        Ok(())
    }

    pub async fn delete_draft_attachment(
        conn: &DatabaseConnection,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        draft_id: CommentDraftId,
        attachment_id: AttachmentId,
        store: &dyn AttachmentStore,
    ) -> Result<(), DomainError> {
        let txn = conn.begin().await.map_err(db_err)?;
        lock_active_draft_for_upload(&txn, ctx, owner, draft_id).await?;
        let attachment = attachment::Entity::find_by_id(attachment_id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DraftId.eq(draft_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?;

        let Some(attachment) = attachment else {
            let tombstoned = comment_attachment_draft_upload::Entity::find()
                .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
                .filter(
                    comment_attachment_draft_upload::Column::OriginalAttachmentId
                        .eq(attachment_id.0),
                )
                .one(&txn)
                .await
                .map_err(db_err)?
                .is_some();

            return Err(if tombstoned {
                DomainError::CommentDraftGone {
                    reason: "draft attachment was deleted".into(),
                }
            } else {
                DomainError::NotFound {
                    entity: "draft attachment",
                    id: attachment_id.0,
                }
            });
        };

        txn.execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO acta.attachment_write_intents (id, digest, created_at) VALUES ($1, $2, now()) ON CONFLICT (digest) DO NOTHING",
            [Uuid::now_v7().into(), attachment.sha256.clone().into()],
        ))
        .await
        .map_err(db_err)?;

        let upload = comment_attachment_draft_upload::Entity::find()
            .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
            .filter(
                comment_attachment_draft_upload::Column::OriginalAttachmentId.eq(attachment_id.0),
            )
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "draft attachment upload",
                id: attachment_id.0,
            })?;
        let mut upload = upload.into_active_model();
        upload.attachment_id = Set(None);
        upload.deleted_at = Set(Some(Utc::now()));
        upload.updated_at = Set(Utc::now());
        upload.update(&txn).await.map_err(db_err)?;

        let digest = attachment.sha256.clone();
        let mut attachment = attachment.into_active_model();
        attachment.deleted_at = Set(Some(Utc::now()));
        attachment.updated_at = Set(Utc::now());
        attachment.update(&txn).await.map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;

        if let Err(error) = Self::finish_purge_digest(conn, store, &digest).await {
            tracing::warn!(%error, digest = %digest, "draft attachment cleanup will be retried");
        }

        Ok(())
    }

    pub async fn delete_comment_attachment(
        conn: &DatabaseConnection,
        ctx: &WorkspaceCtx,
        comment_id: atlas_acta::ids::CommentId,
        attachment_id: AttachmentId,
    ) -> Result<(), DomainError> {
        let txn = conn.begin().await.map_err(db_err)?;
        let attachment = attachment::Entity::find_by_id(attachment_id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::CommentId.eq(comment_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .lock_exclusive()
            .one(&txn)
            .await
            .map_err(db_err)?;

        let Some(attachment) = attachment else {
            return Ok(());
        };

        let deleted_at = Utc::now();
        let mut attachment = attachment.into_active_model();
        attachment.deleted_at = Set(Some(deleted_at));
        attachment.updated_at = Set(deleted_at);
        attachment.update(&txn).await.map_err(db_err)?;

        append_resource_deleted_in(&txn, ctx, TrashKind::Attachment, attachment_id.0).await?;

        txn.commit().await.map_err(db_err)?;

        Ok(())
    }

    /// Finishes a committed comment-attachment purge while holding the same
    /// digest lock used by writes and stale-intent reconciliation.
    pub async fn finish_purge_digest(
        conn: &DatabaseConnection,
        store: &dyn AttachmentStore,
        digest: &str,
    ) -> Result<(), DomainError> {
        let lock = DigestSessionLock::acquire(conn, digest).await?;
        let result = async {
            let intent = attachment_write_intent::Entity::find()
                .filter(attachment_write_intent::Column::Digest.eq(digest))
                .one(conn)
                .await
                .map_err(db_err)?;

            let Some(intent) = intent else {
                return Ok(());
            };

            let has_live_reference = attachment::Entity::find()
                .filter(attachment::Column::Sha256.eq(digest))
                .filter(attachment::Column::DeletedAt.is_null())
                .one(conn)
                .await
                .map_err(db_err)?
                .is_some();

            if !has_live_reference {
                bounded_store_delete(store, digest).await?;
            }

            attachment_write_intent::Entity::delete_by_id(intent.id)
                .exec(conn)
                .await
                .map(|_| ())
                .map_err(db_err)
        }
        .await;
        let unlock = lock.release().await;

        result?;
        unlock
    }

    /// Deletes a blob only after the purge closure removed every recoverable
    /// reference to its digest. The advisory lock keeps concurrent writes and
    /// cleanup attempts from racing this check with a content-addressed write.
    pub async fn cleanup_committed_purge_digest(
        conn: &DatabaseConnection,
        store: &dyn AttachmentStore,
        digest: &str,
    ) -> Result<bool, DomainError> {
        let lock = DigestSessionLock::acquire(conn, digest).await?;
        let result = async {
            let protected = digest_has_recoverable_reference(conn, digest).await?;
            if protected {
                return Ok(false);
            }

            bounded_store_delete(store, digest).await?;
            Ok(true)
        }
        .await;
        let unlock = lock.release().await;

        let deleted = result?;
        unlock?;
        Ok(deleted)
    }

    pub async fn run_reconciler(
        conn: DatabaseConnection,
        store: std::sync::Arc<dyn AttachmentStore>,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        Self::run_reconciler_with_timing(
            conn,
            store,
            shutdown,
            std::time::Duration::from_secs(300),
            chrono::Duration::minutes(10),
        )
        .await;
    }

    pub async fn run_reconciler_with_timing(
        conn: DatabaseConnection,
        store: std::sync::Arc<dyn AttachmentStore>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
        interval_period: std::time::Duration,
        stale_after: chrono::Duration,
    ) {
        let mut interval = tokio::time::interval(interval_period);

        if *shutdown.borrow() {
            return;
        }

        loop {
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return; }
                }
                _ = interval.tick() => {
                    let started_at = std::time::Instant::now();
                    let draft_report = match Self::reconcile_drafts(&conn, store.as_ref()).await {
                        Ok(report) => report,
                        Err(error) => {
                            tracing::warn!(%error, "comment draft retention reconciliation failed");
                            DraftReconciliationReport::default()
                        }
                    };
                    if let Err(error) = crate::services::TrashService::new(conn.clone())
                        .reconcile(store.as_ref())
                        .await
                    {
                        tracing::warn!(%error, "purge cleanup reconciliation failed");
                    }
                    let older_than = Utc::now() - stale_after;
                    tokio::select! {
                        biased;
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() { return; }
                        }
                        result = Self::reconcile_stale(&conn, store.as_ref(), older_than) => {
                            if let Err(error) = result {
                                tracing::warn!(%error, "attachment intent reconciliation failed");
                            }
                        }
                    }
                    tracing::info!(
                        claimed = draft_report.claimed_expiries,
                        failed = draft_report.failed_expiries,
                        pruned = draft_report.pruned,
                        prune_failed = draft_report.failed_prunes,
                        cleanup_failed = draft_report.cleanup_failed,
                        expired_backlog = draft_report.expired_backlog,
                        terminal_backlog = draft_report.terminal_backlog,
                        duration_ms = started_at.elapsed().as_millis(),
                        "attachment lifecycle reconciliation completed"
                    );
                }
            }
        }
    }

    pub async fn reconcile_drafts(
        conn: &DatabaseConnection,
        store: &dyn AttachmentStore,
    ) -> Result<DraftReconciliationReport, DomainError> {
        const BATCH_SIZE: u64 = 100;
        let started_at = std::time::Instant::now();
        let expiry_ids = due_draft_ids(conn, BATCH_SIZE).await?;
        let prune_ids = prunable_draft_ids(conn, BATCH_SIZE).await?;
        let mut report = DraftReconciliationReport {
            expired_backlog: count_due_drafts(conn).await?,
            terminal_backlog: count_prunable_drafts(conn).await?,
            ..DraftReconciliationReport::default()
        };

        for draft_id in expiry_ids {
            match Self::expire_draft(conn, CommentDraftId(draft_id), store).await {
                Ok(Some(cleanup_failed)) => {
                    report.claimed_expiries += 1;
                    report.cleanup_failed += u64::from(cleanup_failed);
                }
                Ok(None) => {}
                Err(error) => {
                    report.failed_expiries += 1;
                    tracing::warn!(%error, %draft_id, "comment draft expiry failed");
                }
            }
        }

        for draft_id in prune_ids {
            match prune_draft(conn, CommentDraftId(draft_id)).await {
                Ok(true) => report.pruned += 1,
                Ok(false) => {}
                Err(error) => {
                    report.failed_prunes += 1;
                    tracing::warn!(%error, %draft_id, "comment draft terminal prune failed");
                }
            }
        }

        tracing::info!(
            claimed = report.claimed_expiries,
            failed = report.failed_expiries,
            pruned = report.pruned,
            prune_failed = report.failed_prunes,
            cleanup_failed = report.cleanup_failed,
            expired_backlog = report.expired_backlog,
            terminal_backlog = report.terminal_backlog,
            duration_ms = started_at.elapsed().as_millis(),
            "comment draft retention reconciliation completed"
        );

        Ok(report)
    }

    async fn expire_draft(
        conn: &DatabaseConnection,
        draft_id: CommentDraftId,
        store: &dyn AttachmentStore,
    ) -> Result<Option<bool>, DomainError> {
        let txn = conn.begin().await.map_err(db_err)?;
        let claimed = comment_attachment_draft::Entity::find_by_id(draft_id.0)
            .filter(comment_attachment_draft::Column::State.eq("active"))
            .filter(comment_attachment_draft::Column::ExpiresAt.lte(Utc::now()))
            .lock_with_behavior(
                sea_orm::sea_query::LockType::Update,
                sea_orm::sea_query::LockBehavior::SkipLocked,
            )
            .one(&txn)
            .await
            .map_err(db_err)?
            .is_some();

        if !claimed {
            txn.commit().await.map_err(db_err)?;
            return Ok(None);
        }

        let attachments = attachment::Entity::find()
            .filter(attachment::Column::DraftId.eq(draft_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .all(&txn)
            .await
            .map_err(db_err)?;
        let digests = attachments
            .iter()
            .map(|attachment| attachment.sha256.clone())
            .collect::<std::collections::BTreeSet<_>>();

        for digest in &digests {
            txn.execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "INSERT INTO acta.attachment_write_intents (id, digest, created_at) \
                 VALUES ($1, $2, now()) ON CONFLICT (digest) DO NOTHING",
                [Uuid::now_v7().into(), digest.clone().into()],
            ))
            .await
            .map_err(db_err)?;
        }

        comment_attachment_draft_upload::Entity::update_many()
            .col_expr(
                comment_attachment_draft_upload::Column::AttachmentId,
                sea_orm::sea_query::Expr::value(None::<Uuid>),
            )
            .col_expr(
                comment_attachment_draft_upload::Column::DeletedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .col_expr(
                comment_attachment_draft_upload::Column::UpdatedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
            .filter(comment_attachment_draft_upload::Column::DeletedAt.is_null())
            .exec(&txn)
            .await
            .map_err(db_err)?;
        attachment::Entity::update_many()
            .col_expr(
                attachment::Column::DeletedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .col_expr(
                attachment::Column::UpdatedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .filter(attachment::Column::DraftId.eq(draft_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .exec(&txn)
            .await
            .map_err(db_err)?;
        comment_attachment_draft::Entity::update_many()
            .col_expr(
                comment_attachment_draft::Column::State,
                sea_orm::sea_query::Expr::value("expired"),
            )
            .col_expr(
                comment_attachment_draft::Column::TerminalAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .col_expr(
                comment_attachment_draft::Column::UpdatedAt,
                sea_orm::sea_query::Expr::current_timestamp(),
            )
            .filter(comment_attachment_draft::Column::Id.eq(draft_id.0))
            .exec(&txn)
            .await
            .map_err(db_err)?;
        txn.commit().await.map_err(db_err)?;

        let mut cleanup_failed = false;
        for digest in digests {
            if let Err(error) = Self::finish_purge_digest(conn, store, &digest).await {
                cleanup_failed = true;
                tracing::warn!(%error, %digest, "expired draft attachment cleanup will be retried");
            }
        }

        Ok(Some(cleanup_failed))
    }

    pub async fn store_and_record(
        conn: &DatabaseConnection,
        ctx: &WorkspaceCtx,
        new: NewAttachment,
        data: &[u8],
        store: &dyn AttachmentStore,
    ) -> Result<Attachment, DomainError> {
        let digest = hex_digest(data);
        PgAttachmentWriteIntentRepo { conn: conn.clone() }
            .create_if_absent(digest.clone())
            .await?;

        let lock = DigestSessionLock::acquire(conn, &digest).await?;
        let result = async {
            PgAttachmentWriteIntentRepo { conn: conn.clone() }
                .create_if_absent(digest.clone())
                .await?;

            let stored = bounded_store_put(store, data).await?;
            if stored != digest {
                return Err(DomainError::Internal {
                    message: "attachment store returned an unexpected digest".into(),
                });
            }

            let txn = conn.begin().await.map_err(db_err)?;
            let attachment = PgAttachmentRepo::record_in(&txn, ctx, new, digest).await?;
            attachment_write_intent::Entity::delete_many()
                .filter(attachment_write_intent::Column::Digest.eq(&attachment.sha256))
                .exec(&txn)
                .await
                .map_err(db_err)?;
            txn.commit().await.map_err(db_err)?;
            Ok(attachment)
        }
        .await;
        let unlock = lock.release().await;

        let attachment = result?;
        unlock?;

        Ok(attachment)
    }

    pub async fn store_and_record_draft(
        conn: &DatabaseConnection,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        draft_id: atlas_acta::ids::CommentDraftId,
        upload: NewCommentAttachmentDraftUpload,
        data: &[u8],
        store: &dyn AttachmentStore,
    ) -> Result<(Attachment, bool), DomainError> {
        let digest = hex_digest(data);
        let lock = DigestSessionLock::acquire(conn, &digest).await?;
        let result = async {
            PgAttachmentWriteIntentRepo { conn: conn.clone() }
                .create_if_absent(digest.clone())
                .await?;

            let txn = conn.begin().await.map_err(db_err)?;
            lock_active_draft_for_upload(&txn, ctx, owner, draft_id).await?;

            let stored = bounded_store_put(store, data).await?;
            if stored != digest {
                return Err(DomainError::Internal {
                    message: "attachment store returned an unexpected digest".into(),
                });
            }

            let (by_user, by_key) = actor_fields(&ctx.actor);
            let row = attachment::ActiveModel {
                id: Set(AttachmentId::new().0),
                workspace_id: Set(ctx.workspace_id.0),
                document_id: Set(None),
                task_id: Set(None),
                comment_id: Set(None),
                draft_id: Set(Some(draft_id.0)),
                file_name: Set(upload.metadata.file_name.clone()),
                content_type: Set(upload.metadata.content_type.clone()),
                size_bytes: Set(data.len() as i64),
                sha256: Set(digest.clone()),
                created_by_user_id: Set(by_user),
                created_by_api_key_id: Set(by_key),
                created_at: Set(Utc::now()),
                updated_at: Set(Utc::now()),
                deleted_at: Set(None),
            }
            .insert(&txn)
            .await
            .map(attachment_from)
            .map_err(db_err)?;

            let recorded = record_upload_or_replay_in(
                &txn,
                ctx,
                owner,
                draft_id,
                NewCommentAttachmentDraftUpload {
                    attachment_id: Some(row.id),
                    upload_token: upload.upload_token,
                    request_digest: upload.request_digest,
                    payload_digest: upload.payload_digest,
                    metadata: upload.metadata,
                    size_bytes: upload.size_bytes,
                },
            )
            .await?;

            let attachment_id = recorded
                .attachment_id
                .ok_or_else(|| DomainError::Internal {
                    message: "active draft upload has no attachment identity".into(),
                })?;
            let replayed = attachment_id != row.id;
            let attachment = attachment::Entity::find_by_id(attachment_id.0)
                .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
                .one(&txn)
                .await
                .map_err(db_err)?
                .map(attachment_from)
                .ok_or(DomainError::NotFound {
                    entity: "draft attachment",
                    id: attachment_id.0,
                })?;

            attachment_write_intent::Entity::delete_many()
                .filter(attachment_write_intent::Column::Digest.eq(&attachment.sha256))
                .exec(&txn)
                .await
                .map_err(db_err)?;
            txn.commit().await.map_err(db_err)?;
            Ok((attachment, replayed))
        }
        .await;
        let unlock = lock.release().await;
        let attachment = result?;
        unlock?;

        Ok(attachment)
    }

    pub async fn reconcile_stale(
        conn: &DatabaseConnection,
        store: &dyn AttachmentStore,
        older_than: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let intents = attachment_write_intent::Entity::find()
            .filter(attachment_write_intent::Column::CreatedAt.lt(older_than))
            .order_by_asc(attachment_write_intent::Column::CreatedAt)
            .order_by_asc(attachment_write_intent::Column::Id)
            .all(conn)
            .await
            .map_err(db_err)?;

        for intent in intents {
            if let Err(error) = Self::reconcile_intent(conn, store, older_than, intent).await {
                tracing::warn!(%error, "attachment intent cleanup failed");
            }
        }

        Ok(())
    }

    async fn reconcile_intent(
        conn: &DatabaseConnection,
        store: &dyn AttachmentStore,
        older_than: DateTime<Utc>,
        intent: attachment_write_intent::Model,
    ) -> Result<(), DomainError> {
        let current = attachment_write_intent::Entity::find_by_id(intent.id)
            .filter(attachment_write_intent::Column::CreatedAt.lt(older_than))
            .one(conn)
            .await
            .map_err(db_err)?;

        let Some(current) = current else {
            return Ok(());
        };

        Self::finish_purge_digest(conn, store, &current.digest).await
    }
}

async fn due_draft_ids(conn: &DatabaseConnection, limit: u64) -> Result<Vec<Uuid>, DomainError> {
    comment_attachment_draft::Entity::find()
        .filter(comment_attachment_draft::Column::State.eq("active"))
        .filter(comment_attachment_draft::Column::ExpiresAt.lte(Utc::now()))
        .order_by_asc(comment_attachment_draft::Column::ExpiresAt)
        .order_by_asc(comment_attachment_draft::Column::Id)
        .limit(limit)
        .all(conn)
        .await
        .map(|drafts| drafts.into_iter().map(|draft| draft.id).collect())
        .map_err(db_err)
}

async fn prunable_draft_ids(
    conn: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<Uuid>, DomainError> {
    terminal_draft_query()
        .order_by_asc(comment_attachment_draft::Column::TerminalAt)
        .order_by_asc(comment_attachment_draft::Column::Id)
        .limit(limit)
        .all(conn)
        .await
        .map(|drafts| drafts.into_iter().map(|draft| draft.id).collect())
        .map_err(db_err)
}

async fn count_due_drafts(conn: &DatabaseConnection) -> Result<u64, DomainError> {
    comment_attachment_draft::Entity::find()
        .filter(comment_attachment_draft::Column::State.eq("active"))
        .filter(comment_attachment_draft::Column::ExpiresAt.lte(Utc::now()))
        .count(conn)
        .await
        .map_err(db_err)
}

async fn count_prunable_drafts(conn: &DatabaseConnection) -> Result<u64, DomainError> {
    terminal_draft_query().count(conn).await.map_err(db_err)
}

async fn prune_draft(
    conn: &DatabaseConnection,
    draft_id: CommentDraftId,
) -> Result<bool, DomainError> {
    let txn = conn.begin().await.map_err(db_err)?;
    let claimed = terminal_draft_query()
        .filter(comment_attachment_draft::Column::Id.eq(draft_id.0))
        .lock_with_behavior(
            sea_orm::sea_query::LockType::Update,
            sea_orm::sea_query::LockBehavior::SkipLocked,
        )
        .one(&txn)
        .await
        .map_err(db_err)?
        .is_some();

    if !claimed {
        txn.commit().await.map_err(db_err)?;
        return Ok(false);
    }

    let attachments = attachment::Entity::find()
        .filter(attachment::Column::DraftId.eq(draft_id.0))
        .all(&txn)
        .await
        .map_err(db_err)?;
    if attachments
        .iter()
        .any(|attachment| attachment.deleted_at.is_none())
    {
        txn.commit().await.map_err(db_err)?;
        return Ok(false);
    }
    for attachment in &attachments {
        if attachment_write_intent::Entity::find()
            .filter(attachment_write_intent::Column::Digest.eq(&attachment.sha256))
            .one(&txn)
            .await
            .map_err(db_err)?
            .is_some()
        {
            txn.commit().await.map_err(db_err)?;
            return Ok(false);
        }
    }
    let uploads = comment_attachment_draft_upload::Entity::find()
        .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
        .all(&txn)
        .await
        .map_err(db_err)?;
    let original_attachment_ids = uploads
        .iter()
        .map(|upload| upload.original_attachment_id)
        .collect::<Vec<_>>();
    for upload in uploads {
        let attachment = attachment::Entity::find_by_id(upload.original_attachment_id)
            .one(&txn)
            .await
            .map_err(db_err)?;
        let Some(attachment) = attachment else {
            continue;
        };
        if attachment.deleted_at.is_none()
            || attachment_write_intent::Entity::find()
                .filter(attachment_write_intent::Column::Digest.eq(&attachment.sha256))
                .one(&txn)
                .await
                .map_err(db_err)?
                .is_some()
        {
            txn.commit().await.map_err(db_err)?;
            return Ok(false);
        }
    }

    comment_attachment_draft_upload::Entity::delete_many()
        .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
        .exec(&txn)
        .await
        .map_err(db_err)?;
    attachment::Entity::delete_many()
        .filter(attachment::Column::DraftId.eq(draft_id.0))
        .filter(attachment::Column::DeletedAt.is_not_null())
        .exec(&txn)
        .await
        .map_err(db_err)?;
    if !original_attachment_ids.is_empty() {
        attachment::Entity::delete_many()
            .filter(attachment::Column::Id.is_in(original_attachment_ids))
            .filter(attachment::Column::DeletedAt.is_not_null())
            .exec(&txn)
            .await
            .map_err(db_err)?;
    }
    comment_attachment_draft::Entity::delete_by_id(draft_id.0)
        .exec(&txn)
        .await
        .map_err(db_err)?;
    txn.commit().await.map_err(db_err)?;

    Ok(true)
}

fn terminal_draft_query() -> sea_orm::Select<comment_attachment_draft::Entity> {
    comment_attachment_draft::Entity::find()
        .filter(comment_attachment_draft::Column::State.is_in([
            "cancelled",
            "expired",
            "deleted_finalized",
        ]))
        .filter(
            comment_attachment_draft::Column::TerminalAt
                .lte(Utc::now() - chrono::Duration::days(7)),
        )
}

async fn bounded_store_put(
    store: &dyn AttachmentStore,
    data: &[u8],
) -> Result<String, DomainError> {
    tokio::time::timeout(ATTACHMENT_STORE_IO_TIMEOUT, store.put(data))
        .await
        .map_err(|_| attachment_store_timeout("put"))?
}

async fn digest_has_recoverable_reference(
    conn: &DatabaseConnection,
    digest: &str,
) -> Result<bool, DomainError> {
    use sea_orm::FromQueryResult;

    #[derive(sea_orm::FromQueryResult)]
    struct Exists {
        exists: bool,
    }

    let row = Exists::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT EXISTS (\
            SELECT 1 FROM acta.attachments WHERE sha256 = $1 \
            UNION ALL \
            SELECT 1 FROM acta.comment_attachment_draft_uploads u \
            JOIN acta.attachments a ON a.id = u.original_attachment_id \
            JOIN acta.comment_attachment_drafts d ON d.id = u.draft_id \
            WHERE a.sha256 = $1 AND d.state IN ('active', 'finalized')\
        ) AS exists",
        [digest.into()],
    ))
    .one(conn)
    .await
    .map_err(db_err)?;

    Ok(row.is_some_and(|row| row.exists))
}

async fn bounded_store_delete(
    store: &dyn AttachmentStore,
    digest: &str,
) -> Result<(), DomainError> {
    tokio::time::timeout(ATTACHMENT_STORE_IO_TIMEOUT, store.delete(digest))
        .await
        .map_err(|_| attachment_store_timeout("delete"))?
}

fn attachment_store_timeout(operation: &str) -> DomainError {
    DomainError::Internal {
        message: format!("attachment store {operation} timed out"),
    }
}

fn sqlx_err(error: sqlx::Error) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}

#[async_trait]
impl AttachmentWriteIntentRepo for PgAttachmentWriteIntentRepo {
    async fn create(&self, digest: String) -> Result<AttachmentWriteIntent, DomainError> {
        attachment_write_intent::ActiveModel {
            id: Set(Uuid::now_v7()),
            digest: Set(digest),
            created_at: Set(Utc::now()),
        }
        .insert(&self.conn)
        .await
        .map(attachment_write_intent_from)
        .map_err(db_err)
    }

    async fn remove(&self, digest: &str) -> Result<(), DomainError> {
        attachment_write_intent::Entity::delete_many()
            .filter(attachment_write_intent::Column::Digest.eq(digest))
            .exec(&self.conn)
            .await
            .map(|_| ())
            .map_err(db_err)
    }

    async fn list_stale(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<AttachmentWriteIntent>, DomainError> {
        attachment_write_intent::Entity::find()
            .filter(attachment_write_intent::Column::CreatedAt.lt(older_than))
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(attachment_write_intent_from).collect())
            .map_err(db_err)
    }
}

impl PgAttachmentWriteIntentRepo {
    async fn create_if_absent(&self, digest: String) -> Result<(), DomainError> {
        self.conn
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "INSERT INTO acta.attachment_write_intents (id, digest, created_at) \
                 VALUES ($1, $2, now()) ON CONFLICT (digest) DO NOTHING",
                [Uuid::now_v7().into(), digest.into()],
            ))
            .await
            .map(|_| ())
            .map_err(db_err)
    }
}

impl PgAttachmentRepo {
    async fn record_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewAttachment,
        sha256: String,
    ) -> Result<Attachment, DomainError> {
        let (by_user, by_key) = actor_fields(&ctx.actor);
        attachment::ActiveModel {
            id: Set(AttachmentId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            document_id: Set(new.document_id.map(|id| id.0)),
            task_id: Set(new.task_id.map(|id| id.0)),
            comment_id: Set(new.comment_id.map(|id| id.0)),
            draft_id: Set(None),
            file_name: Set(new.file_name),
            content_type: Set(new.content_type),
            size_bytes: Set(new.size_bytes),
            sha256: Set(sha256),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        }
        .insert(conn)
        .await
        .map(attachment_from)
        .map_err(db_err)
    }
}

fn hex_digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn actor_fields(actor: &Actor) -> (Option<Uuid>, Option<Uuid>) {
    match actor {
        Actor::User(uid) => (Some(uid.0), None),
        Actor::ApiKey(kid) => (None, Some(kid.0)),
    }
}
