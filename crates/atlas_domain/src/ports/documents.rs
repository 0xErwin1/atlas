use crate::{
    DomainError, WorkspaceCtx,
    entities::documents::{
        Attachment, AttachmentOwner, Document, DocumentFilter, DocumentLink, DocumentSummary,
        ExtractedLink, NewAttachment, NewDocument, RevisionMeta,
    },
    ids::{AttachmentId, DocumentId, FolderId, ProjectId, RevisionId},
};
use async_trait::async_trait;

#[async_trait]
pub trait DocumentRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewDocument) -> Result<Document, DomainError>;

    async fn get(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
    ) -> Result<Option<Document>, DomainError>;

    async fn list(
        &self,
        ctx: &WorkspaceCtx,
        filter: DocumentFilter,
    ) -> Result<Vec<DocumentSummary>, DomainError>;

    async fn update_content(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        expected_revision: RevisionId,
        new_content: &str,
    ) -> Result<Document, DomainError>;

    async fn update_frontmatter(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        fm: serde_json::Value,
    ) -> Result<Document, DomainError>;

    async fn move_to(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        folder: Option<FolderId>,
        project: Option<ProjectId>,
    ) -> Result<(), DomainError>;

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: DocumentId) -> Result<(), DomainError>;

    async fn history(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
    ) -> Result<Vec<RevisionMeta>, DomainError>;

    async fn content_at(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        seq: i64,
    ) -> Result<String, DomainError>;
}

#[async_trait]
pub trait DocumentLinkRepo: Send + Sync {
    async fn replace_for_source(
        &self,
        ctx: &WorkspaceCtx,
        source: DocumentId,
        links: Vec<ExtractedLink>,
    ) -> Result<(), DomainError>;

    async fn backlinks(
        &self,
        ctx: &WorkspaceCtx,
        target: DocumentId,
    ) -> Result<Vec<DocumentLink>, DomainError>;
}

#[async_trait]
pub trait AttachmentRepo: Send + Sync {
    async fn record(
        &self,
        ctx: &WorkspaceCtx,
        new: NewAttachment,
    ) -> Result<Attachment, DomainError>;

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: AttachmentId,
    ) -> Result<Option<Attachment>, DomainError>;

    async fn list_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: AttachmentOwner,
    ) -> Result<Vec<Attachment>, DomainError>;

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: AttachmentId) -> Result<(), DomainError>;
}
