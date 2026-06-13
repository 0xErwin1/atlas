use atlas_domain::entities::documents::{
    Attachment, Document, DocumentLink, DocumentRevision, DocumentSummary, RevisionMeta,
};
use atlas_domain::ids::{
    ApiKeyId, AttachmentId, DocumentId, FolderId, ProjectId, RevisionId, TaskId, UserId,
    WorkspaceId,
};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod document {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "documents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub project_id: Option<Uuid>,
        pub folder_id: Option<Uuid>,
        pub title: String,
        pub content: String,
        pub frontmatter: Json,
        pub current_revision_id: Option<Uuid>,
        pub current_revision_seq: i64,
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

pub mod document_revision {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "document_revisions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub document_id: Uuid,
        pub seq: i64,
        pub patch: Option<String>,
        pub snapshot: Option<String>,
        pub is_anchor: bool,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod document_link {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "document_links")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub source_document_id: Uuid,
        pub target_document_id: Option<Uuid>,
        pub target_title: String,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod attachment {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "attachments")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub document_id: Option<Uuid>,
        pub task_id: Option<Uuid>,
        pub file_name: String,
        pub content_type: String,
        pub size_bytes: i64,
        pub sha256: String,
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

pub fn document_from(m: document::Model) -> Result<Document, String> {
    let current_revision_id = m
        .current_revision_id
        .ok_or_else(|| "document missing current_revision_id".to_string())?;

    Ok(Document {
        id: DocumentId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        project_id: m.project_id.map(ProjectId),
        folder_id: m.folder_id.map(FolderId),
        title: m.title,
        content: m.content,
        frontmatter: m.frontmatter,
        current_revision_id: RevisionId(current_revision_id),
        current_revision_seq: m.current_revision_seq,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_by_api_key_id: m.created_by_api_key_id.map(ApiKeyId),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    })
}

pub fn document_summary_from(m: document::Model) -> Result<DocumentSummary, String> {
    let current_revision_id = m
        .current_revision_id
        .ok_or_else(|| "document missing current_revision_id".to_string())?;

    Ok(DocumentSummary {
        id: DocumentId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        project_id: m.project_id.map(ProjectId),
        folder_id: m.folder_id.map(FolderId),
        title: m.title,
        frontmatter: m.frontmatter,
        current_revision_id: RevisionId(current_revision_id),
        current_revision_seq: m.current_revision_seq,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

pub fn revision_from(m: document_revision::Model) -> DocumentRevision {
    DocumentRevision {
        id: RevisionId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        document_id: DocumentId(m.document_id),
        seq: m.seq,
        patch: m.patch,
        snapshot: m.snapshot,
        is_anchor: m.is_anchor,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_at: m.created_at,
    }
}

pub fn revision_meta_from(m: document_revision::Model) -> RevisionMeta {
    RevisionMeta {
        id: RevisionId(m.id),
        seq: m.seq,
        is_anchor: m.is_anchor,
        created_at: m.created_at,
    }
}

pub fn document_link_from(m: document_link::Model) -> DocumentLink {
    DocumentLink {
        id: DocumentId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        source_document_id: DocumentId(m.source_document_id),
        target_document_id: m.target_document_id.map(DocumentId),
        target_title: m.target_title,
        created_at: m.created_at,
    }
}

pub fn attachment_from(m: attachment::Model) -> Attachment {
    Attachment {
        id: AttachmentId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        document_id: m.document_id.map(DocumentId),
        task_id: m.task_id.map(TaskId),
        file_name: m.file_name,
        content_type: m.content_type,
        size_bytes: m.size_bytes,
        sha256: m.sha256,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_by_api_key_id: m.created_by_api_key_id.map(ApiKeyId),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}
