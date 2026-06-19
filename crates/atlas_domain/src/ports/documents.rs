use crate::{
    DomainError, WorkspaceCtx,
    entities::documents::{
        Attachment, AttachmentOwner, Document, DocumentLink, DocumentSummary, ExtractedLink,
        NewAttachment, NewDocument, RevisionMeta,
    },
    ids::{AttachmentId, DocumentId, FolderId, ProjectId, RevisionId, TaskId},
    permissions::Principal,
};
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait DocumentRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewDocument) -> Result<Document, DomainError>;

    async fn get(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
    ) -> Result<Option<Document>, DomainError>;

    /// Lists documents visible to `principal` within the workspace.
    ///
    /// Visibility rules mirror those of `ProjectRepo::list_visible`: membership
    /// and explicit grants determine access. When `project_filter` is `Some`, the
    /// listing is additionally scoped to documents belonging to that project;
    /// `None` lists across the whole workspace. `after_id` is an exclusive cursor
    /// (UUID of the last seen document).
    async fn list_visible(
        &self,
        ctx: &WorkspaceCtx,
        principal: &Principal,
        project_filter: Option<ProjectId>,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<DocumentSummary>, DomainError>;

    /// Returns the document whose slug matches within this workspace, if any.
    async fn find_by_slug(
        &self,
        ctx: &WorkspaceCtx,
        slug: &str,
    ) -> Result<Option<Document>, DomainError>;

    /// Lists every live document whose `folder_id` equals `folder`, without any
    /// visibility filtering. Used by the recursive folder copy, which has already
    /// authorized the caller against the source subtree and needs the raw set of
    /// documents to duplicate.
    async fn list_in_folder(
        &self,
        ctx: &WorkspaceCtx,
        folder: FolderId,
    ) -> Result<Vec<Document>, DomainError>;

    /// Renames a document: updates `title`, re-derives `slug` (with collision
    /// resolution), and propagates the display title to any inbound
    /// `document_links` rows that resolved to this document.
    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        new_title: String,
    ) -> Result<Document, DomainError>;

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

    /// Replaces all wikilinks emitted from a task description.
    ///
    /// Analogous to `replace_for_source` but writes rows with `source_task_id`
    /// instead of `source_document_id`. Called inside the task create/patch txn.
    async fn replace_for_task_source(
        &self,
        ctx: &WorkspaceCtx,
        source: TaskId,
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
