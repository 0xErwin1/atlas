use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::comments::Comment;
use atlas_acta::entities::comments::CommentOwner;
use atlas_acta::entities::comments::NewComment;
use atlas_acta::entities::comments::comment_draft_finalize_digest_input;
use atlas_acta::ids::CommentDraftId;
use atlas_acta::ids::CommentId;
use atlas_acta::ports::attachment_store::AttachmentStore;
use atlas_acta::wikilink::parse_comment_link_candidates;
use atlas_core::error::DomainError;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, QueryFilter, QuerySelect, TransactionTrait,
};
use sha2::{Digest, Sha256};

use crate::persistence::{
    entities::{comments::comment_attachment_draft, documents::attachment},
    repos::{PgCommentLinkRepo, PgCommentRepo, PgSearchIndexQueueRepo, append_resource_deleted_in},
};
use atlas_postgres::db_err;

// Relocated to `atlas_acta_postgres::repos::comment_links` (S4 PR8): it is
// threaded into `PgCommentLinkRepo::replace_for_comment_with_fault_in` as a
// parameter, so it must live in that crate. Re-exported (not just imported)
// here so `services::mod.rs`'s existing `pub use
// comment_service::CommentMutationFault` keeps resolving unchanged.
pub use crate::persistence::repos::CommentMutationFault;

/// Coordinates comment bodies, their derived graph, and comment-owned blob cleanup.
#[derive(Clone)]
pub struct CommentService {
    conn: DatabaseConnection,
    #[cfg(debug_assertions)]
    fault: Option<CommentMutationFault>,
}

pub struct FinalizeCommentResult {
    pub comment: Comment,
    pub replayed: bool,
}

impl CommentService {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self {
            conn,
            #[cfg(debug_assertions)]
            fault: None,
        }
    }

    pub fn with_attachment_store(
        conn: DatabaseConnection,
        _attachments: std::sync::Arc<dyn AttachmentStore>,
    ) -> Self {
        Self {
            conn,
            #[cfg(debug_assertions)]
            fault: None,
        }
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn with_fault_injection(conn: DatabaseConnection, fault: CommentMutationFault) -> Self {
        Self {
            conn,
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
        PgSearchIndexQueueRepo::enqueue_comment_owner_in(&txn, ctx.workspace_id, owner).await?;

        txn.commit().await.map_err(db_err)?;
        Ok(comment)
    }

    pub async fn finalize_draft(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        draft_id: CommentDraftId,
        body: String,
    ) -> Result<FinalizeCommentResult, DomainError> {
        let body_digest = Sha256::digest(body.as_bytes()).to_vec();
        let request_digest = Sha256::digest(comment_draft_finalize_digest_input(
            draft_id.0,
            &body,
            &body_digest,
        ))
        .to_vec();
        let candidates = parse_comment_link_candidates(&body);
        let txn = self.conn.begin().await.map_err(db_err)?;
        let draft = find_draft_for_finalize(&txn, ctx, owner, draft_id).await?;

        if draft.state == "finalized" {
            if draft.final_body_digest.as_deref() != Some(&body_digest)
                || draft.final_request_digest.as_deref() != Some(&request_digest)
            {
                return Err(DomainError::CommentDraftConflict {
                    reason: "draft finalization request differs from the original".into(),
                });
            }

            let comment_id = draft
                .finalized_comment_id
                .ok_or_else(|| DomainError::Internal {
                    message: "finalized draft has no comment identity".into(),
                })?;
            let comment =
                PgCommentRepo::get_for_owner_in(&txn, ctx, owner, CommentId(comment_id)).await?;
            txn.commit().await.map_err(db_err)?;
            return Ok(FinalizeCommentResult {
                comment,
                replayed: true,
            });
        }

        if draft.state != "active" {
            return Err(DomainError::CommentDraftGone {
                reason: "draft is no longer active".into(),
            });
        }

        let comment = PgCommentRepo::create_with_id_in(
            &txn,
            ctx,
            NewComment { owner, body },
            CommentId(draft_id.0),
        )
        .await?;
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

        attachment::Entity::update_many()
            .col_expr(
                attachment::Column::CommentId,
                sea_orm::sea_query::Expr::value(comment.id.0),
            )
            .col_expr(
                attachment::Column::DraftId,
                sea_orm::sea_query::Expr::value(None::<uuid::Uuid>),
            )
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DraftId.eq(draft_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .exec(&txn)
            .await
            .map_err(db_err)?;

        let mut active = draft.into_active_model();
        active.state = Set("finalized".into());
        active.finalized_comment_id = Set(Some(comment.id.0));
        active.final_body_digest = Set(Some(body_digest));
        active.final_request_digest = Set(Some(request_digest));
        active.updated_at = Set(chrono::Utc::now());
        active.update(&txn).await.map_err(db_err)?;

        txn.commit().await.map_err(db_err)?;
        Ok(FinalizeCommentResult {
            comment,
            replayed: false,
        })
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
        PgSearchIndexQueueRepo::enqueue_comment_owner_in(&txn, ctx.workspace_id, owner).await?;

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
        let comment =
            PgCommentRepo::get_for_owner_for_update_in(&txn, ctx, owner, comment_id).await?;

        if comment.created_by != ctx.actor && !can_moderate {
            return Err(DomainError::Forbidden {
                message: "only the comment's author or a workspace admin/owner may delete it"
                    .into(),
            });
        }

        let deleted_at = chrono::Utc::now();
        let attachment_ids = live_comment_attachment_ids_in(&txn, ctx, comment_id).await?;

        PgCommentLinkRepo::remove_for_comment_in(&txn, ctx, comment_id).await?;
        PgCommentLinkRepo::record_comment_deleted_in(&txn, ctx, comment_id).await?;
        PgCommentRepo::soft_delete_at_in(&txn, ctx, owner, comment_id, deleted_at).await?;

        tombstone_comment_attachments_in(&txn, ctx, comment_id, deleted_at).await?;
        append_resource_deleted_in(
            &txn,
            ctx,
            atlas_acta::entities::lifecycle::TrashKind::Comment,
            comment_id.0,
        )
        .await?;

        for attachment_id in attachment_ids {
            append_resource_deleted_in(
                &txn,
                ctx,
                atlas_acta::entities::lifecycle::TrashKind::Attachment,
                attachment_id,
            )
            .await?;
        }

        PgSearchIndexQueueRepo::enqueue_comment_owner_in(&txn, ctx.workspace_id, owner).await?;

        txn.commit().await.map_err(db_err)?;

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

async fn find_draft_for_finalize(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    draft_id: CommentDraftId,
) -> Result<crate::persistence::entities::comments::comment_attachment_draft::Model, DomainError> {
    let (task_id, document_id) = match owner {
        CommentOwner::Task(id) => (Some(id.0), None),
        CommentOwner::Document(id) => (None, Some(id.0)),
    };
    let (user_id, api_key_id) = match &ctx.actor {
        atlas_acta::actor::Actor::User(id) => (Some(id.0), None),
        atlas_acta::actor::Actor::ApiKey(id) => (None, Some(id.0)),
    };

    comment_attachment_draft::Entity::find_by_id(draft_id.0)
        .filter(comment_attachment_draft::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(comment_attachment_draft::Column::TaskId.eq(task_id))
        .filter(comment_attachment_draft::Column::DocumentId.eq(document_id))
        .filter(comment_attachment_draft::Column::CreatedByUserId.eq(user_id))
        .filter(comment_attachment_draft::Column::CreatedByApiKeyId.eq(api_key_id))
        .lock_exclusive()
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "comment attachment draft",
            id: draft_id.0,
        })
}

async fn live_comment_attachment_ids_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    comment_id: CommentId,
) -> Result<Vec<uuid::Uuid>, DomainError> {
    attachment::Entity::find()
        .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(attachment::Column::CommentId.eq(comment_id.0))
        .filter(attachment::Column::DeletedAt.is_null())
        .lock_exclusive()
        .all(conn)
        .await
        .map(|attachments| {
            attachments
                .into_iter()
                .map(|attachment| attachment.id)
                .collect()
        })
        .map_err(db_err)
}

async fn tombstone_comment_attachments_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    comment_id: CommentId,
    deleted_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), DomainError> {
    attachment::Entity::update_many()
        .col_expr(
            attachment::Column::DeletedAt,
            sea_orm::sea_query::Expr::value(deleted_at),
        )
        .col_expr(
            attachment::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(deleted_at),
        )
        .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(attachment::Column::CommentId.eq(comment_id.0))
        .filter(attachment::Column::DeletedAt.is_null())
        .exec(conn)
        .await
        .map_err(db_err)?;

    Ok(())
}
