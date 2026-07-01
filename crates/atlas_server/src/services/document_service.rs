use atlas_domain::{
    DomainError, WorkspaceCtx,
    entities::documents::{Document, NewDocument},
    entities::events::{
        DocumentCreatedPayload, DocumentDeletedPayload, DocumentMovedPayload,
        DocumentUpdatedPayload, DomainEvent,
    },
    ids::{DocumentId, FolderId, ProjectId, RevisionId},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};

use crate::persistence::entities::documents::document;
use crate::persistence::repos::{
    PgOutboxRepo, doc_create_in, doc_move_to_in, doc_rename_in, doc_soft_delete_in,
    doc_update_content_in,
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
}

impl DocumentService {
    pub fn new(conn: DatabaseConnection, anchor_interval: u32) -> Self {
        Self {
            conn,
            anchor_interval,
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

        doc_soft_delete_in(&txn, ctx, id).await?;

        let event = DomainEvent::DocumentDeleted(DocumentDeletedPayload { document_id: id });
        PgOutboxRepo::insert_in(&txn, ctx, pre_project_id, None, event).await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
