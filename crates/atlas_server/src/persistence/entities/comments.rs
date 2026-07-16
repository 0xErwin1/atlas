use crate::persistence::entities::boards_tasks::actor_from_columns;
use atlas_domain::entities::comments::{
    Comment, CommentAttachmentDraft, CommentAttachmentDraftState, CommentLink, CommentLinkTarget,
    CommentOwner,
};
use atlas_domain::ids::{
    AttachmentId, CommentDraftId, CommentId, CommentLinkId, DocumentId, TaskId, WorkspaceId,
};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod comment {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "comments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub task_id: Option<Uuid>,
        pub document_id: Option<Uuid>,
        pub body: String,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod comment_link {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "comment_links")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub comment_id: Uuid,
        pub target_document_id: Option<Uuid>,
        pub target_task_id: Option<Uuid>,
        pub target_attachment_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod comment_link_event {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "comment_link_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub parent_task_id: Option<Uuid>,
        pub parent_document_id: Option<Uuid>,
        pub comment_id: Uuid,
        pub event_kind: String,
        pub target_document_id: Option<Uuid>,
        pub target_task_id: Option<Uuid>,
        pub target_attachment_id: Option<Uuid>,
        pub actor_type: String,
        pub actor_id: Uuid,
        pub created_at: DateTime<Utc>,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

pub mod comment_attachment_draft {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "comment_attachment_drafts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub task_id: Option<Uuid>,
        pub document_id: Option<Uuid>,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub create_token: String,
        pub create_digest: Vec<u8>,
        pub state: String,
        pub finalized_comment_id: Option<Uuid>,
        pub final_body_digest: Option<Vec<u8>>,
        pub final_request_digest: Option<Vec<u8>>,
        pub expires_at: DateTime<Utc>,
        pub terminal_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod comment_attachment_draft_upload {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "comment_attachment_draft_uploads")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub draft_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub upload_token: String,
        pub original_attachment_id: Uuid,
        pub attachment_id: Option<Uuid>,
        pub request_digest: Vec<u8>,
        pub payload_digest: Vec<u8>,
        pub file_name: String,
        pub content_type: String,
        pub size_bytes: i64,
        pub deleted_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn comment_from(m: comment::Model) -> Comment {
    Comment {
        id: CommentId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        task_id: m.task_id.map(TaskId),
        document_id: m.document_id.map(DocumentId),
        body: m.body,
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}

pub fn comment_attachment_draft_from(
    m: comment_attachment_draft::Model,
) -> Result<CommentAttachmentDraft, String> {
    let owner = match (m.task_id, m.document_id) {
        (Some(task_id), None) => CommentOwner::Task(TaskId(task_id)),
        (None, Some(document_id)) => CommentOwner::Document(DocumentId(document_id)),
        _ => return Err("comment attachment draft violates parent constraint".into()),
    };
    let state = match m.state.as_str() {
        "active" => CommentAttachmentDraftState::Active,
        "finalized" => CommentAttachmentDraftState::Finalized,
        "cancelled" => CommentAttachmentDraftState::Cancelled,
        "expired" => CommentAttachmentDraftState::Expired,
        "deleted_finalized" => CommentAttachmentDraftState::DeletedFinalized,
        _ => return Err("comment attachment draft has invalid state".into()),
    };

    Ok(CommentAttachmentDraft {
        id: CommentDraftId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        owner,
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        create_token: m.create_token,
        create_digest: m.create_digest,
        state,
        finalized_comment_id: m.finalized_comment_id.map(CommentId),
        final_body_digest: m.final_body_digest,
        final_request_digest: m.final_request_digest,
        expires_at: m.expires_at,
        terminal_at: m.terminal_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

pub fn comment_attachment_draft_upload_from(
    m: comment_attachment_draft_upload::Model,
) -> Result<atlas_domain::entities::comments::CommentAttachmentDraftUpload, String> {
    let metadata = atlas_domain::entities::comments::CommentDraftMetadata::normalize(
        &m.file_name,
        &m.content_type,
    )
    .map_err(|error| error.to_string())?;

    Ok(
        atlas_domain::entities::comments::CommentAttachmentDraftUpload {
            draft_id: CommentDraftId(m.draft_id),
            upload_token: m.upload_token,
            original_attachment_id: AttachmentId(m.original_attachment_id),
            attachment_id: m.attachment_id.map(AttachmentId),
            request_digest: m.request_digest,
            payload_digest: m.payload_digest,
            metadata,
            size_bytes: m.size_bytes,
            deleted_at: m.deleted_at,
        },
    )
}

pub fn comment_link_from(m: comment_link::Model) -> CommentLink {
    let target = match (
        m.target_document_id,
        m.target_task_id,
        m.target_attachment_id,
    ) {
        (Some(id), None, None) => CommentLinkTarget::Document(DocumentId(id)),
        (None, Some(id), None) => CommentLinkTarget::Task(TaskId(id)),
        (None, None, Some(id)) => CommentLinkTarget::Attachment(AttachmentId(id)),
        _ => unreachable!("comment_links target constraint must hold"),
    };

    CommentLink {
        id: CommentLinkId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        comment_id: CommentId(m.comment_id),
        target,
        created_at: m.created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn base_model() -> comment::Model {
        comment::Model {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            task_id: Some(Uuid::now_v7()),
            document_id: None,
            body: "hello".into(),
            created_by_user_id: Some(Uuid::now_v7()),
            created_by_api_key_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        }
    }

    #[test]
    fn comment_from_roundtrips_task_owner_and_body() {
        let m = base_model();
        let task_id = m.task_id;
        let body = m.body.clone();

        let comment = comment_from(m);

        assert_eq!(comment.task_id.map(|id| id.0), task_id);
        assert!(comment.document_id.is_none());
        assert_eq!(comment.body, body);
    }

    #[test]
    fn comment_from_resolves_api_key_author() {
        let mut m = base_model();
        let key_id = Uuid::now_v7();
        m.created_by_user_id = None;
        m.created_by_api_key_id = Some(key_id);

        let comment = comment_from(m);

        assert_eq!(
            comment.created_by,
            atlas_domain::Actor::ApiKey(atlas_domain::ApiKeyId(key_id))
        );
    }
}
