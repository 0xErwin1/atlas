use crate::actor::Actor;
use crate::ids::{
    AttachmentId, CommentId, CommentLinkEventId, CommentLinkId, DocumentId, TaskId, WorkspaceId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentDraftMetadata {
    pub file_name: String,
    pub content_type: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentAttachmentDraftState {
    Active,
    Finalized,
    Cancelled,
    Expired,
    DeletedFinalized,
}

#[derive(Debug, Clone)]
pub struct CommentAttachmentDraft {
    pub id: crate::ids::CommentDraftId,
    pub workspace_id: crate::ids::WorkspaceId,
    pub owner: CommentOwner,
    pub created_by: Actor,
    pub create_token: String,
    pub create_digest: Vec<u8>,
    pub state: CommentAttachmentDraftState,
    pub finalized_comment_id: Option<CommentId>,
    pub final_body_digest: Option<Vec<u8>>,
    pub final_request_digest: Option<Vec<u8>>,
    pub expires_at: DateTime<Utc>,
    pub terminal_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewCommentAttachmentDraft {
    pub id: crate::ids::CommentDraftId,
    pub owner: CommentOwner,
    pub create_token: String,
    pub create_digest: Vec<u8>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CommentAttachmentDraftUpload {
    pub draft_id: crate::ids::CommentDraftId,
    pub upload_token: String,
    pub original_attachment_id: crate::ids::AttachmentId,
    pub attachment_id: Option<crate::ids::AttachmentId>,
    pub request_digest: Vec<u8>,
    pub payload_digest: Vec<u8>,
    pub metadata: CommentDraftMetadata,
    pub size_bytes: i64,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewCommentAttachmentDraftUpload {
    pub attachment_id: Option<crate::ids::AttachmentId>,
    pub upload_token: String,
    pub request_digest: Vec<u8>,
    pub payload_digest: Vec<u8>,
    pub metadata: CommentDraftMetadata,
    pub size_bytes: i64,
}

impl CommentDraftMetadata {
    pub fn normalize(file_name: &str, content_type: &str) -> Result<Self, crate::DomainError> {
        let file_name = file_name.trim_matches(|c: char| c.is_ascii_whitespace());

        if file_name.is_empty()
            || file_name
                .chars()
                .any(|c| matches!(c, '/' | '\\' | '\0' | '\r' | '\n'))
        {
            return Err(crate::DomainError::InvalidInput {
                message: "draft attachment file name is invalid".into(),
            });
        }

        let content_type = if content_type.is_empty() {
            "application/octet-stream"
        } else {
            content_type
        };

        let Some((type_name, subtype)) = content_type.split_once('/') else {
            return Err(crate::DomainError::InvalidInput {
                message: "draft attachment content type is invalid".into(),
            });
        };

        if type_name.is_empty()
            || subtype.is_empty()
            || subtype.contains('/')
            || content_type.contains(';')
            || !content_type.is_ascii()
            || !type_name.bytes().all(is_mime_token)
            || !subtype.bytes().all(is_mime_token)
        {
            return Err(crate::DomainError::InvalidInput {
                message: "draft attachment content type is invalid".into(),
            });
        }

        Ok(Self {
            file_name: file_name.to_string(),
            content_type: content_type.to_ascii_lowercase(),
        })
    }
}

fn is_mime_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

pub fn comment_draft_digest_input(operation: &[u8], components: &[&[u8]]) -> Vec<u8> {
    let mut input = Vec::from(b"comment-draft/v1".as_slice());
    input.extend_from_slice(operation);

    for component in components {
        input.extend_from_slice(&(component.len() as u64).to_be_bytes());
        input.extend_from_slice(component);
    }

    input
}

pub fn comment_draft_create_digest_input(
    workspace_id: uuid::Uuid,
    draft_id: uuid::Uuid,
    create_token: &str,
) -> Vec<u8> {
    comment_draft_digest_input(
        b"create",
        &[
            workspace_id.as_bytes(),
            draft_id.as_bytes(),
            create_token.as_bytes(),
        ],
    )
}

pub fn comment_draft_upload_digest_input(
    draft_id: uuid::Uuid,
    upload_token: &str,
    file_name: &str,
    content_type: &str,
    size_bytes: i64,
    payload_digest: &[u8],
) -> Vec<u8> {
    comment_draft_digest_input(
        b"upload",
        &[
            draft_id.as_bytes(),
            upload_token.as_bytes(),
            file_name.as_bytes(),
            content_type.as_bytes(),
            &size_bytes.to_be_bytes(),
            payload_digest,
        ],
    )
}

pub fn comment_draft_finalize_digest_input(
    draft_id: uuid::Uuid,
    body: &str,
    request_digest: &[u8],
) -> Vec<u8> {
    comment_draft_digest_input(
        b"finalize",
        &[draft_id.as_bytes(), body.as_bytes(), request_digest],
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: CommentId,
    pub workspace_id: WorkspaceId,
    pub task_id: Option<TaskId>,
    pub document_id: Option<DocumentId>,
    pub body: String,
    pub created_by: Actor,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewComment {
    pub owner: CommentOwner,
    pub body: String,
}

/// The owning parent of a comment (polymorphic: task or document).
///
/// Mirrors `AttachmentOwner` (entities/documents.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentOwner {
    Task(TaskId),
    Document(DocumentId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommentLinkTarget {
    Document(DocumentId),
    Task(TaskId),
    Attachment(AttachmentId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentLink {
    pub id: CommentLinkId,
    pub workspace_id: WorkspaceId,
    pub comment_id: CommentId,
    pub target: CommentLinkTarget,
    pub created_at: DateTime<Utc>,
}

/// A live derived comment link together with its live owning parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentBacklink {
    pub id: CommentLinkId,
    pub workspace_id: WorkspaceId,
    pub comment_id: CommentId,
    pub parent: CommentOwner,
    pub parent_readable_id: Option<String>,
    pub parent_slug: Option<String>,
    pub parent_title: String,
    pub target: CommentLinkTarget,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommentLinkEventKind {
    LinkAdded,
    LinkRemoved,
    CommentDeleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentLinkEvent {
    pub id: CommentLinkEventId,
    pub workspace_id: WorkspaceId,
    pub parent: CommentOwner,
    pub comment_id: CommentId,
    pub kind: CommentLinkEventKind,
    pub target: Option<CommentLinkTarget>,
    pub actor: Actor,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum CommentFeedEntry {
    Comment(Comment),
    Event(CommentLinkEvent),
}

impl CommentFeedEntry {
    pub fn cursor(&self) -> CommentFeedCursor {
        match self {
            Self::Comment(comment) => CommentFeedCursor {
                created_at: comment.created_at,
                id: comment.id.0,
            },
            Self::Event(event) => CommentFeedCursor {
                created_at: event.created_at,
                id: event.id.0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommentFeedCursor {
    pub created_at: DateTime<Utc>,
    pub id: uuid::Uuid,
}

/// One oldest-first page from a comment parent feed.
#[derive(Debug, Clone)]
pub struct CommentFeedPage {
    pub entries: Vec<CommentFeedEntry>,
    pub has_more: bool,
}
