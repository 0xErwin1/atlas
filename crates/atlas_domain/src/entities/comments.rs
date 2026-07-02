use crate::actor::Actor;
use crate::ids::{CommentId, DocumentId, TaskId, WorkspaceId};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentOwner {
    Task(TaskId),
    Document(DocumentId),
}
