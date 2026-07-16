use atlas_domain::{
    DomainError, WorkspaceCtx,
    entities::comments::{Comment, CommentOwner},
    entities::documents::{Document, NewDocument},
    entities::events::{
        DocumentCreatedPayload, DocumentDeletedPayload, DocumentMovedPayload,
        DocumentUpdatedPayload, DomainEvent,
    },
    ids::{CommentDraftId, CommentId, DocumentId, FolderId, ProjectId, RevisionId},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};

use crate::persistence::entities::{comments::comment_attachment_draft, documents::document};
use crate::persistence::repos::{
    CommentRepo, PgCommentRepo, PgOutboxRepo, doc_create_in, doc_move_to_in, doc_rename_in,
    doc_soft_delete_in, doc_update_content_in,
};

/// Coordinates document mutations with transactional outbox emission.
///
/// Each method opens exactly one `DatabaseTransaction`, performs the mutation
/// via the corresponding `*_in` primitive, inserts an outbox row in the same
/// transaction, then commits. A failure at any step rolls back both the
/// mutation and the outbox row atomically.
///
/// `rename` is included for symmetry (so route handlers have one object to
/// call through) but does not emit a domain event — there is no `DocumentRenamed`
/// in the event catalog.
pub struct DocumentService {
    conn: DatabaseConnection,
    anchor_interval: u32,
    comments: crate::services::CommentService,
}

impl DocumentService {
    pub fn new(conn: DatabaseConnection, anchor_interval: u32) -> Self {
        Self {
            comments: crate::services::CommentService::new(conn.clone()),
            conn,
            anchor_interval,
        }
    }

    pub fn with_comment_service(
        conn: DatabaseConnection,
        anchor_interval: u32,
        comments: crate::services::CommentService,
    ) -> Self {
        Self {
            conn,
            anchor_interval,
            comments,
        }
    }

    /// Creates a new document and emits a `DocumentCreated` event.
    pub async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewDocument,
    ) -> Result<Document, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let doc = doc_create_in(&txn, ctx, new).await?;

        let event = DomainEvent::DocumentCreated(DocumentCreatedPayload {
            document_id: doc.id,
            slug: doc.slug.clone().unwrap_or_default(),
            title: doc.title.clone(),
            project_id: doc.project_id,
            folder_id: doc.folder_id,
        });
        PgOutboxRepo::insert_in(&txn, ctx, doc.project_id, None, event).await?;

        txn.commit().await.map_err(db_err)?;
        Ok(doc)
    }

    /// Updates document content and emits a `DocumentUpdated` event.
    ///
    /// Returns `DomainError::Conflict` when `expected_revision` does not match
    /// the current head (CAS semantics). The transaction is rolled back on
    /// conflict so no outbox row is written.
    pub async fn update_content(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        expected_revision: RevisionId,
        new_content: &str,
    ) -> Result<Document, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let doc = doc_update_content_in(
            &txn,
            ctx,
            id,
            expected_revision,
            new_content,
            self.anchor_interval,
        )
        .await?;

        let event = DomainEvent::DocumentUpdated(DocumentUpdatedPayload {
            document_id: doc.id,
            revision_id: doc.current_revision_id,
            seq: doc.current_revision_seq,
        });
        PgOutboxRepo::insert_in(&txn, ctx, doc.project_id, None, event).await?;

        txn.commit().await.map_err(db_err)?;
        Ok(doc)
    }

    /// Renames a document. No domain event is emitted (no `DocumentRenamed` in catalog).
    pub async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        new_title: String,
    ) -> Result<Document, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let doc = doc_rename_in(&txn, ctx, id, new_title).await?;
        txn.commit().await.map_err(db_err)?;
        Ok(doc)
    }

    /// Moves a document to a different folder or project root and emits a
    /// `DocumentMoved` event carrying the pre-move `from_folder_id`.
    pub async fn move_to(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        folder: Option<FolderId>,
        project: Option<ProjectId>,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let pre = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        let from_folder_id = pre.folder_id.map(FolderId);
        let pre_project_id = pre.project_id.map(ProjectId);

        doc_move_to_in(&txn, ctx, id, folder, project).await?;

        let event = DomainEvent::DocumentMoved(DocumentMovedPayload {
            document_id: id,
            from_folder_id,
            to_folder_id: folder,
            project_id: pre_project_id,
        });
        PgOutboxRepo::insert_in(&txn, ctx, pre_project_id, None, event).await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Soft-deletes a document and emits a `DocumentDeleted` event.
    pub async fn soft_delete(&self, ctx: &WorkspaceCtx, id: DocumentId) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let pre = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        let pre_project_id = pre.project_id.map(ProjectId);

        let retained_draft = comment_attachment_draft::Entity::find()
            .filter(comment_attachment_draft::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(comment_attachment_draft::Column::DocumentId.eq(id.0))
            .one(&txn)
            .await
            .map_err(db_err)?;
        if retained_draft.is_some() {
            return Err(DomainError::CommentDraftConflict {
                reason: "document has retained comment draft state".into(),
            });
        }

        doc_soft_delete_in(&txn, ctx, id).await?;

        let event = DomainEvent::DocumentDeleted(DocumentDeletedPayload { document_id: id });
        PgOutboxRepo::insert_in(&txn, ctx, pre_project_id, None, event).await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Adds a markdown comment to a document.
    ///
    /// Mirrors `TaskService::add_comment` for the document owner: comments are an
    /// append-only conversation surface, so no domain event is emitted.
    pub async fn add_comment(
        &self,
        ctx: &WorkspaceCtx,
        document_id: DocumentId,
        body: String,
    ) -> Result<Comment, DomainError> {
        self.comments
            .create(ctx, CommentOwner::Document(document_id), body)
            .await
    }

    pub async fn finalize_comment_draft(
        &self,
        ctx: &WorkspaceCtx,
        document_id: DocumentId,
        draft_id: CommentDraftId,
        body: String,
    ) -> Result<crate::services::comment_service::FinalizeCommentResult, DomainError> {
        self.comments
            .finalize_draft(ctx, CommentOwner::Document(document_id), draft_id, body)
            .await
    }

    /// Returns paginated comments for a document, oldest-first.
    pub async fn list_comments(
        &self,
        ctx: &WorkspaceCtx,
        document_id: DocumentId,
        after_id: Option<CommentId>,
        limit: u64,
    ) -> Result<Vec<Comment>, DomainError> {
        let repo = PgCommentRepo::new(self.conn.clone());
        repo.list_for_owner(ctx, CommentOwner::Document(document_id), after_id, limit)
            .await
    }

    /// Removes a comment from a document.
    ///
    /// The comment's author may always delete their own comment; `can_moderate`
    /// (workspace admin/owner) allows deleting anyone's. Everyone else gets
    /// `Forbidden`. Mirrors `TaskService::remove_comment`.
    pub async fn remove_comment(
        &self,
        ctx: &WorkspaceCtx,
        document_id: DocumentId,
        comment_id: CommentId,
        can_moderate: bool,
    ) -> Result<(), DomainError> {
        self.comments
            .remove(
                ctx,
                CommentOwner::Document(document_id),
                comment_id,
                can_moderate,
            )
            .await
    }

    /// Edits the body of a document comment. Only the comment's author may edit it
    /// (moderation does not extend to rewriting another person's words). Mirrors
    /// `TaskService::update_comment`.
    pub async fn update_comment(
        &self,
        ctx: &WorkspaceCtx,
        document_id: DocumentId,
        comment_id: CommentId,
        body: String,
    ) -> Result<Comment, DomainError> {
        self.comments
            .update(ctx, CommentOwner::Document(document_id), comment_id, body)
            .await
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
