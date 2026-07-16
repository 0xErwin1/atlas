use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::comments::{
        CommentAttachmentDraft, CommentAttachmentDraftState, CommentAttachmentDraftUpload,
        CommentOwner, NewCommentAttachmentDraft, NewCommentAttachmentDraftUpload,
        comment_draft_create_digest_input,
    },
    ids::{AttachmentId, CommentDraftId},
    ports::comments::CommentAttachmentDraftRepo,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QuerySelect, TransactionTrait,
};
use sha2::{Digest, Sha256};

use crate::persistence::entities::comments::{
    comment_attachment_draft, comment_attachment_draft_from, comment_attachment_draft_upload,
    comment_attachment_draft_upload_from,
};
use crate::persistence::entities::documents::attachment;

pub struct PgCommentAttachmentDraftRepo {
    conn: DatabaseConnection,
}

impl PgCommentAttachmentDraftRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl CommentAttachmentDraftRepo for PgCommentAttachmentDraftRepo {
    async fn create_or_replay(
        &self,
        ctx: &WorkspaceCtx,
        new: NewCommentAttachmentDraft,
    ) -> Result<CommentAttachmentDraft, DomainError> {
        if let Some(existing) = find_by_create_token(&self.conn, ctx, &new.create_token).await? {
            return replay_or_conflict(existing, new);
        }

        let (task_id, document_id) = owner_columns(new.owner);
        let (created_by_user_id, created_by_api_key_id) = actor_columns(&ctx.actor);
        let now = Utc::now();
        let insert = comment_attachment_draft::ActiveModel {
            id: Set(new.id.0),
            workspace_id: Set(ctx.workspace_id.0),
            task_id: Set(task_id),
            document_id: Set(document_id),
            created_by_user_id: Set(created_by_user_id),
            created_by_api_key_id: Set(created_by_api_key_id),
            create_token: Set(new.create_token.clone()),
            create_digest: Set(new.create_digest.clone()),
            state: Set("active".into()),
            finalized_comment_id: Set(None),
            final_body_digest: Set(None),
            final_request_digest: Set(None),
            expires_at: Set(new.expires_at),
            terminal_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };

        match insert.insert(&self.conn).await {
            Ok(row) => comment_attachment_draft_from(row).map_err(internal_err),
            Err(error) if error.sql_err().is_some() => {
                let existing = find_by_create_token(&self.conn, ctx, &new.create_token)
                    .await?
                    .ok_or_else(|| internal_err("draft create token conflict was not readable"))?;
                replay_or_conflict(existing, new)
            }
            Err(error) => Err(DomainError::Internal {
                message: error.to_string(),
            }),
        }
    }

    async fn get_for_owner_and_creator(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentDraftId,
    ) -> Result<Option<CommentAttachmentDraft>, DomainError> {
        let (task_id, document_id) = owner_columns(owner);
        let (user_id, api_key_id) = actor_columns(&ctx.actor);
        comment_attachment_draft::Entity::find_by_id(id.0)
            .filter(comment_attachment_draft::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(comment_attachment_draft::Column::TaskId.eq(task_id))
            .filter(comment_attachment_draft::Column::DocumentId.eq(document_id))
            .filter(comment_attachment_draft::Column::CreatedByUserId.eq(user_id))
            .filter(comment_attachment_draft::Column::CreatedByApiKeyId.eq(api_key_id))
            .one(&self.conn)
            .await
            .map_err(|error| DomainError::Internal {
                message: error.to_string(),
            })?
            .map(comment_attachment_draft_from)
            .transpose()
            .map_err(internal_err)
    }

    async fn record_upload_or_replay(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        draft_id: CommentDraftId,
        new: NewCommentAttachmentDraftUpload,
    ) -> Result<CommentAttachmentDraftUpload, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let upload = record_upload_or_replay_in(&txn, ctx, owner, draft_id, new).await;
        txn.commit().await.map_err(db_err)?;
        upload
    }

    async fn tombstone_upload(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        draft_id: CommentDraftId,
        upload_token: &str,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let draft = find_draft_for_update(&txn, ctx, owner, draft_id).await?;

        ensure_active(&draft)?;

        let preview =
            find_upload(&txn, draft_id, upload_token)
                .await?
                .ok_or(DomainError::NotFound {
                    entity: "comment attachment draft upload",
                    id: draft_id.0,
                })?;

        if preview.deleted_at.is_some() {
            return Err(DomainError::CommentDraftGone {
                reason: "draft upload was deleted".into(),
            });
        }

        if let Some(attachment_id) = preview.attachment_id {
            lock_attachment_for_draft(&txn, ctx, draft_id, AttachmentId(attachment_id)).await?;
        }

        let upload = find_upload_for_update(&txn, draft_id, upload_token)
            .await?
            .ok_or(DomainError::NotFound {
                entity: "comment attachment draft upload",
                id: draft_id.0,
            })?;

        if upload.deleted_at.is_some() {
            return Err(DomainError::CommentDraftGone {
                reason: "draft upload was deleted".into(),
            });
        }

        let mut active = upload.into_active_model();
        active.attachment_id = Set(None);
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&txn).await.map_err(db_err)?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    async fn get_upload_for_original_attachment_id(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        draft_id: CommentDraftId,
        original_attachment_id: AttachmentId,
    ) -> Result<Option<CommentAttachmentDraftUpload>, DomainError> {
        find_draft_for_update(&self.conn, ctx, owner, draft_id).await?;

        comment_attachment_draft_upload::Entity::find()
            .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
            .filter(
                comment_attachment_draft_upload::Column::OriginalAttachmentId
                    .eq(original_attachment_id.0),
            )
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(comment_attachment_draft_upload_from)
            .transpose()
            .map_err(internal_err)
    }
}

pub(crate) async fn lock_active_draft_for_upload(
    conn: &impl sea_orm::ConnectionTrait,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    draft_id: CommentDraftId,
) -> Result<(), DomainError> {
    let draft = find_draft_for_update(conn, ctx, owner, draft_id).await?;

    if draft.state == CommentAttachmentDraftState::Finalized {
        return Err(DomainError::CommentDraftConflict {
            reason: "draft is already finalized".into(),
        });
    }

    ensure_active(&draft)
}

pub(crate) async fn record_upload_or_replay_in(
    conn: &impl sea_orm::ConnectionTrait,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    draft_id: CommentDraftId,
    new: NewCommentAttachmentDraftUpload,
) -> Result<CommentAttachmentDraftUpload, DomainError> {
    lock_active_draft_for_upload(conn, ctx, owner, draft_id).await?;

    if let Some(attachment_id) = new.attachment_id {
        lock_attachment_for_draft(conn, ctx, draft_id, attachment_id).await?;
    }

    if find_upload(conn, draft_id, &new.upload_token)
        .await?
        .is_some()
    {
        let existing = find_upload_for_update(conn, draft_id, &new.upload_token)
            .await?
            .ok_or_else(|| internal_err("draft upload disappeared while replaying"))?;
        let result = replay_upload_or_conflict(existing.clone(), &new);
        remove_provisional_attachment(
            conn,
            new.attachment_id,
            existing.attachment_id.map(AttachmentId),
        )
        .await?;
        return result;
    }

    let attachment_id = new.attachment_id.ok_or(DomainError::InvalidInput {
        message: "new draft upload requires an attachment".into(),
    })?;

    let existing = find_upload_for_update(conn, draft_id, &new.upload_token).await?;
    match existing {
        Some(existing) => {
            let result = replay_upload_or_conflict(existing.clone(), &new);
            remove_provisional_attachment(
                conn,
                Some(attachment_id),
                existing.attachment_id.map(AttachmentId),
            )
            .await?;
            result
        }
        None => insert_upload(conn, draft_id, new).await,
    }
}

async fn find_by_create_token(
    conn: &DatabaseConnection,
    ctx: &WorkspaceCtx,
    create_token: &str,
) -> Result<Option<CommentAttachmentDraft>, DomainError> {
    let (user_id, api_key_id) = actor_columns(&ctx.actor);
    comment_attachment_draft::Entity::find()
        .filter(comment_attachment_draft::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(comment_attachment_draft::Column::CreatedByUserId.eq(user_id))
        .filter(comment_attachment_draft::Column::CreatedByApiKeyId.eq(api_key_id))
        .filter(comment_attachment_draft::Column::CreateToken.eq(create_token))
        .one(conn)
        .await
        .map_err(|error| DomainError::Internal {
            message: error.to_string(),
        })?
        .map(comment_attachment_draft_from)
        .transpose()
        .map_err(internal_err)
}

fn replay_or_conflict(
    existing: CommentAttachmentDraft,
    new: NewCommentAttachmentDraft,
) -> Result<CommentAttachmentDraft, DomainError> {
    let expected_digest = Sha256::digest(comment_draft_create_digest_input(
        existing.workspace_id.0,
        existing.id.0,
        &existing.create_token,
    ));

    if existing.owner != new.owner || existing.create_digest != expected_digest.as_slice() {
        return Err(DomainError::CommentDraftConflict {
            reason: "create token was reused with a different request".into(),
        });
    }

    if !matches!(
        existing.state,
        CommentAttachmentDraftState::Active | CommentAttachmentDraftState::Finalized
    ) {
        return Err(DomainError::CommentDraftGone {
            reason: "draft is no longer active".into(),
        });
    }

    Ok(existing)
}

fn owner_columns(owner: CommentOwner) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match owner {
        CommentOwner::Task(task_id) => (Some(task_id.0), None),
        CommentOwner::Document(document_id) => (None, Some(document_id.0)),
    }
}

fn actor_columns(actor: &Actor) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match actor {
        Actor::User(user_id) => (Some(user_id.0), None),
        Actor::ApiKey(api_key_id) => (None, Some(api_key_id.0)),
    }
}

fn internal_err(message: impl Into<String>) -> DomainError {
    DomainError::Internal {
        message: message.into(),
    }
}

fn db_err(error: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}

async fn find_draft_for_update(
    conn: &impl sea_orm::ConnectionTrait,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    id: CommentDraftId,
) -> Result<CommentAttachmentDraft, DomainError> {
    let (task_id, document_id) = owner_columns(owner);
    let (user_id, api_key_id) = actor_columns(&ctx.actor);
    let row = comment_attachment_draft::Entity::find_by_id(id.0)
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
            id: id.0,
        })?;

    comment_attachment_draft_from(row).map_err(internal_err)
}

fn ensure_active(draft: &CommentAttachmentDraft) -> Result<(), DomainError> {
    if draft.state == CommentAttachmentDraftState::Active {
        return Ok(());
    }

    Err(DomainError::CommentDraftGone {
        reason: "draft is no longer active".into(),
    })
}

async fn lock_attachment_for_draft(
    conn: &impl sea_orm::ConnectionTrait,
    ctx: &WorkspaceCtx,
    draft_id: CommentDraftId,
    attachment_id: AttachmentId,
) -> Result<(), DomainError> {
    attachment::Entity::find_by_id(attachment_id.0)
        .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(attachment::Column::DraftId.eq(draft_id.0))
        .filter(attachment::Column::DeletedAt.is_null())
        .lock_exclusive()
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "draft attachment",
            id: attachment_id.0,
        })?;
    Ok(())
}

async fn remove_provisional_attachment(
    conn: &impl sea_orm::ConnectionTrait,
    provisional_attachment_id: Option<AttachmentId>,
    persisted_attachment_id: Option<AttachmentId>,
) -> Result<(), DomainError> {
    let Some(provisional_attachment_id) = provisional_attachment_id else {
        return Ok(());
    };

    if Some(provisional_attachment_id) == persisted_attachment_id {
        return Ok(());
    }

    attachment::Entity::delete_by_id(provisional_attachment_id.0)
        .exec(conn)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn find_upload_for_update(
    conn: &impl sea_orm::ConnectionTrait,
    draft_id: CommentDraftId,
    upload_token: &str,
) -> Result<Option<comment_attachment_draft_upload::Model>, DomainError> {
    comment_attachment_draft_upload::Entity::find()
        .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
        .filter(comment_attachment_draft_upload::Column::UploadToken.eq(upload_token))
        .lock_exclusive()
        .one(conn)
        .await
        .map_err(db_err)
}

async fn find_upload(
    conn: &impl sea_orm::ConnectionTrait,
    draft_id: CommentDraftId,
    upload_token: &str,
) -> Result<Option<comment_attachment_draft_upload::Model>, DomainError> {
    comment_attachment_draft_upload::Entity::find()
        .filter(comment_attachment_draft_upload::Column::DraftId.eq(draft_id.0))
        .filter(comment_attachment_draft_upload::Column::UploadToken.eq(upload_token))
        .one(conn)
        .await
        .map_err(db_err)
}

fn replay_upload_or_conflict(
    existing: comment_attachment_draft_upload::Model,
    new: &NewCommentAttachmentDraftUpload,
) -> Result<CommentAttachmentDraftUpload, DomainError> {
    if existing.deleted_at.is_some() {
        return Err(DomainError::CommentDraftGone {
            reason: "draft upload was deleted".into(),
        });
    }

    if existing.request_digest != new.request_digest
        || existing.payload_digest != new.payload_digest
        || existing.file_name != new.metadata.file_name
        || existing.content_type != new.metadata.content_type
        || existing.size_bytes != new.size_bytes
    {
        return Err(DomainError::CommentDraftConflict {
            reason: "upload token was reused with a different request".into(),
        });
    }

    comment_attachment_draft_upload_from(existing).map_err(internal_err)
}

async fn insert_upload(
    conn: &impl sea_orm::ConnectionTrait,
    draft_id: CommentDraftId,
    new: NewCommentAttachmentDraftUpload,
) -> Result<CommentAttachmentDraftUpload, DomainError> {
    let now = Utc::now();
    let row = comment_attachment_draft_upload::ActiveModel {
        draft_id: Set(draft_id.0),
        upload_token: Set(new.upload_token),
        original_attachment_id: Set(new
            .attachment_id
            .ok_or(DomainError::InvalidInput {
                message: "new draft upload requires an attachment".into(),
            })?
            .0),
        attachment_id: Set(Some(
            new.attachment_id
                .ok_or(DomainError::InvalidInput {
                    message: "new draft upload requires an attachment".into(),
                })?
                .0,
        )),
        request_digest: Set(new.request_digest),
        payload_digest: Set(new.payload_digest),
        file_name: Set(new.metadata.file_name),
        content_type: Set(new.metadata.content_type),
        size_bytes: Set(new.size_bytes),
        deleted_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(conn)
    .await
    .map_err(db_err)?;

    comment_attachment_draft_upload_from(row).map_err(internal_err)
}
