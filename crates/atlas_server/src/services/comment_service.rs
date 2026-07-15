use std::{collections::BTreeSet, sync::Arc};

use atlas_domain::{
    AttachmentStore, DomainError, WorkspaceCtx,
    entities::comments::{Comment, CommentOwner, NewComment},
    ids::CommentId,
    wikilink::parse_comment_link_candidates,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter, Statement,
    TransactionTrait,
};

use crate::persistence::{
    entities::documents::attachment,
    repos::{PgAttachmentLifecycle, PgCommentLinkRepo, PgCommentRepo},
};

/// Internal test seam for proving comment mutations commit as one transaction.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentMutationFault {
    AfterBodyWrite,
    AfterGraphReplace,
    AfterEventAppend,
}

/// Coordinates comment bodies, their derived graph, and comment-owned blob cleanup.
#[derive(Clone)]
pub struct CommentService {
    conn: DatabaseConnection,
    attachments: Option<Arc<dyn AttachmentStore>>,
    #[cfg(debug_assertions)]
    fault: Option<CommentMutationFault>,
}

impl CommentService {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self {
            conn,
            attachments: None,
            #[cfg(debug_assertions)]
            fault: None,
        }
    }

    pub fn with_attachment_store(
        conn: DatabaseConnection,
        attachments: Arc<dyn AttachmentStore>,
    ) -> Self {
        Self {
            conn,
            attachments: Some(attachments),
            #[cfg(debug_assertions)]
            fault: None,
        }
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn with_fault_injection(conn: DatabaseConnection, fault: CommentMutationFault) -> Self {
        Self {
            conn,
            attachments: None,
            fault: Some(fault),
        }
    }

    pub async fn create(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        body: String,
    ) -> Result<Comment, DomainError> {
        let candidates = parse_comment_link_candidates(&body);
        let txn = self.conn.begin().await.map_err(db_err)?;

        let comment = PgCommentRepo::create_in(&txn, ctx, NewComment { owner, body }).await?;
        #[cfg(debug_assertions)]
        self.fail_if(CommentMutationFault::AfterBodyWrite)?;
        let targets = PgCommentLinkRepo::classify_candidates_in(&txn, ctx, candidates).await?;
        PgCommentLinkRepo::replace_for_comment_with_fault_in(
            &txn,
            ctx,
            comment.id,
            targets,
            self.fault_for_mutation(),
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(comment)
    }

    pub async fn update(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        comment_id: CommentId,
        body: String,
    ) -> Result<Comment, DomainError> {
        let candidates = parse_comment_link_candidates(&body);
        let txn = self.conn.begin().await.map_err(db_err)?;
        let comment = PgCommentRepo::get_for_owner_in(&txn, ctx, owner, comment_id).await?;

        if comment.created_by != ctx.actor {
            return Err(DomainError::Forbidden {
                message: "only the comment's author may edit it".into(),
            });
        }

        let updated = PgCommentRepo::update_body_from(&txn, ctx, owner, comment, body).await?;
        #[cfg(debug_assertions)]
        self.fail_if(CommentMutationFault::AfterBodyWrite)?;
        let targets = PgCommentLinkRepo::classify_candidates_in(&txn, ctx, candidates).await?;
        PgCommentLinkRepo::replace_for_comment_with_fault_in(
            &txn,
            ctx,
            comment_id,
            targets,
            self.fault_for_mutation(),
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(updated)
    }

    pub async fn remove(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        comment_id: CommentId,
        can_moderate: bool,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let comment = PgCommentRepo::get_for_owner_in(&txn, ctx, owner, comment_id).await?;

        if comment.created_by != ctx.actor && !can_moderate {
            return Err(DomainError::Forbidden {
                message: "only the comment's author or a workspace admin/owner may delete it"
                    .into(),
            });
        }

        let purge = prepare_comment_purge_in(&txn, ctx, comment_id).await?;
        PgCommentLinkRepo::remove_for_comment_in(&txn, ctx, comment_id).await?;
        PgCommentLinkRepo::record_comment_deleted_in(&txn, ctx, comment_id).await?;
        PgCommentRepo::soft_delete_in(&txn, ctx, owner, comment_id).await?;

        txn.commit().await.map_err(db_err)?;

        if let Some(store) = &self.attachments {
            for digest in purge {
                if let Err(error) =
                    PgAttachmentLifecycle::finish_purge_digest(&self.conn, store.as_ref(), &digest)
                        .await
                {
                    tracing::warn!(%error, %digest, "comment attachment cleanup will be retried");
                }
            }
        }

        Ok(())
    }

    #[cfg(debug_assertions)]
    fn fail_if(&self, point: CommentMutationFault) -> Result<(), DomainError> {
        if self.fault == Some(point) {
            return Err(DomainError::Internal {
                message: format!("injected comment mutation fault at {point:?}"),
            });
        }

        Ok(())
    }

    #[cfg(debug_assertions)]
    fn fault_for_mutation(&self) -> Option<CommentMutationFault> {
        self.fault
    }

    #[cfg(not(debug_assertions))]
    fn fault_for_mutation(&self) -> Option<CommentMutationFault> {
        None
    }
}

async fn prepare_comment_purge_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    comment_id: CommentId,
) -> Result<Vec<String>, DomainError> {
    let attachments = attachment::Entity::find()
        .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(attachment::Column::CommentId.eq(comment_id.0))
        .filter(attachment::Column::DeletedAt.is_null())
        .all(conn)
        .await
        .map_err(db_err)?;
    let digests = attachments
        .iter()
        .map(|attachment| attachment.sha256.clone())
        .collect::<BTreeSet<_>>();

    for digest in &digests {
        conn.execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "INSERT INTO attachment_write_intents (id, digest, created_at) VALUES ($1, $2, now()) ON CONFLICT (digest) DO NOTHING",
            [uuid::Uuid::now_v7().into(), digest.clone().into()],
        ))
        .await
        .map_err(db_err)?;
    }

    attachment::Entity::delete_many()
        .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(attachment::Column::CommentId.eq(comment_id.0))
        .filter(attachment::Column::DeletedAt.is_null())
        .exec(conn)
        .await
        .map_err(db_err)?;

    Ok(digests.into_iter().collect())
}

fn db_err(error: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}
