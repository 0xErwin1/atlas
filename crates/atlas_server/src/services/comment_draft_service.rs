use std::sync::Arc;

use atlas_domain::{
    DomainError, WorkspaceCtx,
    entities::comments::{
        CommentAttachmentDraft, CommentOwner, NewCommentAttachmentDraft,
        comment_draft_create_digest_input,
    },
    ids::CommentDraftId,
    ports::comments::CommentAttachmentDraftRepo,
};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct CommentDraftService {
    drafts: Arc<dyn CommentAttachmentDraftRepo>,
}

pub struct CommentDraftCreateResult {
    pub draft: CommentAttachmentDraft,
    pub replayed: bool,
}

impl std::ops::Deref for CommentDraftCreateResult {
    type Target = CommentAttachmentDraft;

    fn deref(&self) -> &Self::Target {
        &self.draft
    }
}

impl CommentDraftService {
    pub fn new(drafts: Arc<dyn CommentAttachmentDraftRepo>) -> Self {
        Self { drafts }
    }

    pub async fn create_or_replay(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        create_token: String,
        expires_at: DateTime<Utc>,
    ) -> Result<CommentDraftCreateResult, DomainError> {
        let id = CommentDraftId::new();
        let create_digest = Sha256::digest(comment_draft_create_digest_input(
            ctx.workspace_id.0,
            id.0,
            &create_token,
        ))
        .to_vec();

        let draft = self
            .drafts
            .create_or_replay(
                ctx,
                NewCommentAttachmentDraft {
                    id,
                    owner,
                    create_token,
                    create_digest,
                    expires_at,
                },
            )
            .await?;

        Ok(CommentDraftCreateResult {
            replayed: draft.id != id,
            draft,
        })
    }
}
