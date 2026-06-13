use crate::ids::{
    ApiKeyId, AttachmentId, DocumentId, FolderId, ProjectId, RevisionId, UserId, WorkspaceId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub workspace_id: WorkspaceId,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
    pub title: String,
    pub content: String,
    pub frontmatter: serde_json::Value,
    pub current_revision_id: RevisionId,
    pub current_revision_seq: i64,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: DocumentId,
    pub workspace_id: WorkspaceId,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
    pub title: String,
    pub frontmatter: serde_json::Value,
    pub current_revision_id: RevisionId,
    pub current_revision_seq: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct DocumentFilter {
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
}

#[derive(Debug, Clone)]
pub struct NewDocument {
    pub title: String,
    pub slug: Option<String>,
    pub content: String,
    pub folder_id: Option<FolderId>,
    pub project_id: Option<ProjectId>,
    pub frontmatter: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRevision {
    pub id: RevisionId,
    pub workspace_id: WorkspaceId,
    pub document_id: DocumentId,
    pub seq: i64,
    pub patch: Option<String>,
    pub snapshot: Option<String>,
    pub is_anchor: bool,
    pub created_by_user_id: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionMeta {
    pub id: RevisionId,
    pub seq: i64,
    pub is_anchor: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentLink {
    pub id: crate::ids::DocumentId,
    pub workspace_id: WorkspaceId,
    pub source_document_id: DocumentId,
    pub target_document_id: Option<DocumentId>,
    pub target_title: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ExtractedLink {
    pub target_title: String,
    pub target_document_id: Option<DocumentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub workspace_id: WorkspaceId,
    pub document_id: Option<DocumentId>,
    pub task_id: Option<crate::ids::TaskId>,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub document_id: Option<DocumentId>,
    pub task_id: Option<crate::ids::TaskId>,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub enum AttachmentOwner {
    Document(DocumentId),
    Task(crate::ids::TaskId),
}
