use crate::actor::Actor;
use crate::ids::{
    AttachmentId, CommentId, CommentLinkEventId, CommentLinkId, DocumentId, TaskId, WorkspaceId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
