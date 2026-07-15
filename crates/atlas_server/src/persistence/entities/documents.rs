use atlas_domain::entities::documents::{
    Attachment, AttachmentWriteIntent, Document, DocumentLink, DocumentRevision, DocumentSummary,
    RevisionMeta,
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
        pub slug: Option<String>,
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
        pub source_document_id: Option<Uuid>,
        pub source_task_id: Option<Uuid>,
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
        pub comment_id: Option<Uuid>,
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

pub mod attachment_write_intent {
    use super::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "attachment_write_intents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub digest: String,
        pub created_at: DateTime<Utc>,
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
        slug: m.slug,
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
        slug: m.slug,
        frontmatter: m.frontmatter,
        current_revision_id: RevisionId(current_revision_id),
        current_revision_seq: m.current_revision_seq,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_by_api_key_id: m.created_by_api_key_id.map(ApiKeyId),
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
        created_by_api_key_id: m.created_by_api_key_id.map(ApiKeyId),
        created_at: m.created_at,
    }
}

pub fn revision_meta_from(m: document_revision::Model) -> RevisionMeta {
    RevisionMeta {
        id: RevisionId(m.id),
        seq: m.seq,
        is_anchor: m.is_anchor,
        created_by_user_id: m.created_by_user_id.map(UserId),
        created_by_api_key_id: m.created_by_api_key_id.map(ApiKeyId),
        created_at: m.created_at,
    }
}

pub fn document_link_from(m: document_link::Model) -> DocumentLink {
    DocumentLink {
        id: DocumentId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        source_document_id: m.source_document_id.map(DocumentId),
        source_task_id: m.source_task_id.map(TaskId),
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
        comment_id: m.comment_id.map(atlas_domain::ids::CommentId),
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

pub fn attachment_write_intent_from(m: attachment_write_intent::Model) -> AttachmentWriteIntent {
    AttachmentWriteIntent {
        id: m.id,
        digest: m.digest,
        created_at: m.created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn rev_id() -> Uuid {
        Uuid::now_v7()
    }

    fn base_doc_model(rev: Uuid) -> document::Model {
        document::Model {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: None,
            folder_id: None,
            title: "Test".into(),
            slug: Some("test".into()),
            content: "body".into(),
            frontmatter: serde_json::json!({}),
            current_revision_id: Some(rev),
            current_revision_seq: 1,
            created_by_user_id: Some(Uuid::now_v7()),
            created_by_api_key_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        }
    }

    #[test]
    fn document_from_roundtrips_slug() {
        let rev = rev_id();
        let m = base_doc_model(rev);
        let slug = m.slug.clone();

        let doc = document_from(m).expect("document_from must succeed");

        assert_eq!(doc.slug, slug);
    }

    #[test]
    fn document_from_roundtrips_created_by_api_key_id() {
        let rev = rev_id();
        let key_id = Uuid::now_v7();
        let mut m = base_doc_model(rev);
        m.created_by_user_id = None;
        m.created_by_api_key_id = Some(key_id);

        let doc = document_from(m).expect("document_from must succeed");

        assert_eq!(doc.created_by_api_key_id.map(|k| k.0), Some(key_id));
        assert!(doc.created_by_user_id.is_none());
    }

    #[test]
    fn revision_meta_from_carries_actor_ids() {
        let key_uuid = Uuid::now_v7();
        let rev_model = document_revision::Model {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            document_id: Uuid::now_v7(),
            seq: 3,
            patch: Some("patch".into()),
            snapshot: None,
            is_anchor: false,
            created_by_user_id: None,
            created_by_api_key_id: Some(key_uuid),
            created_at: Utc::now(),
        };

        let meta = revision_meta_from(rev_model.clone());

        assert_eq!(meta.id.0, rev_model.id);
        assert_eq!(meta.seq, 3);
        assert_eq!(meta.created_by_api_key_id.map(|k| k.0), Some(key_uuid));
        assert!(meta.created_by_user_id.is_none());
    }
}
