use async_trait::async_trait;
use atlas_domain::{
    DomainError,
    ids::WorkspaceId,
    semantic_search::{ResourceKind, SemanticIndexChunk, SemanticIndexer},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};
use std::sync::Arc;
use uuid::Uuid;

use crate::persistence::entities::{
    boards_tasks::{task, task_checklist_item},
    comments::comment,
    documents::{attachment, document},
};
use crate::persistence::repos::PgSemanticIndexWriter;
use crate::semantic_indexer::{
    AttachmentText, ChecklistText, CommentText, DocumentIndexInput, SubtaskText, TaskIndexInput,
    aggregate_document_chunks, aggregate_task_chunks,
};

/// Upper bound on the characters that go into a single embedded chunk.
///
/// Sized well below the token window of the common embedding models so a chunk
/// never gets silently truncated provider-side, where the dropped tail would be
/// indistinguishable from content that was simply never written.
const MAX_CHUNK_CHARS: usize = 1_500;

/// Reads a resource's full indexable text and re-embeds it.
///
/// Deleted resources are not an error: their chunks are removed and the call
/// succeeds, so a delete that races the worker drains the queue instead of
/// retrying forever.
pub struct PgSemanticIndexer {
    conn: DatabaseConnection,
    writer: Arc<PgSemanticIndexWriter>,
}

impl PgSemanticIndexer {
    pub fn new(conn: DatabaseConnection, writer: Arc<PgSemanticIndexWriter>) -> Self {
        Self { conn, writer }
    }

    async fn load_task_chunks(
        &self,
        workspace_id: WorkspaceId,
        task_id: Uuid,
    ) -> Result<Vec<SemanticIndexChunk>, DomainError> {
        let Some(task) = task::Entity::find_by_id(task_id)
            .filter(task::Column::WorkspaceId.eq(workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
        else {
            return Ok(Vec::new());
        };

        let subtasks = task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(workspace_id.0))
            .filter(task::Column::ParentTaskId.eq(task_id))
            .filter(task::Column::DeletedAt.is_null())
            .order_by_asc(task::Column::PositionKey)
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        let mut subtask_texts = Vec::with_capacity(subtasks.len());
        for subtask in subtasks {
            let checklist_items = self.load_checklist(workspace_id, subtask.id).await?;
            subtask_texts.push(SubtaskText {
                readable_id: subtask.readable_id,
                title: subtask.title,
                description: subtask.description,
                checklist_items,
            });
        }

        Ok(aggregate_task_chunks(TaskIndexInput {
            workspace_id,
            task_id,
            readable_id: task.readable_id,
            title: task.title,
            description: task.description,
            labels: task.labels,
            comments: self.load_task_comments(workspace_id, task_id).await?,
            attachments: self
                .load_attachments(workspace_id, ResourceKind::Task, task_id)
                .await?,
            checklist_items: self.load_checklist(workspace_id, task_id).await?,
            subtasks: subtask_texts,
            max_chunk_chars: MAX_CHUNK_CHARS,
        }))
    }

    async fn load_document_chunks(
        &self,
        workspace_id: WorkspaceId,
        document_id: Uuid,
    ) -> Result<Vec<SemanticIndexChunk>, DomainError> {
        let Some(document) = document::Entity::find_by_id(document_id)
            .filter(document::Column::WorkspaceId.eq(workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
        else {
            return Ok(Vec::new());
        };

        Ok(aggregate_document_chunks(DocumentIndexInput {
            workspace_id,
            document_id,
            title: document.title,
            content: document.content,
            comments: self
                .load_document_comments(workspace_id, document_id)
                .await?,
            attachments: self
                .load_attachments(workspace_id, ResourceKind::Document, document_id)
                .await?,
            max_chunk_chars: MAX_CHUNK_CHARS,
        }))
    }

    async fn load_checklist(
        &self,
        workspace_id: WorkspaceId,
        task_id: Uuid,
    ) -> Result<Vec<ChecklistText>, DomainError> {
        Ok(task_checklist_item::Entity::find()
            .filter(task_checklist_item::Column::WorkspaceId.eq(workspace_id.0))
            .filter(task_checklist_item::Column::TaskId.eq(task_id))
            .filter(task_checklist_item::Column::DeletedAt.is_null())
            .order_by_asc(task_checklist_item::Column::PositionKey)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|item| ChecklistText { title: item.title })
            .collect())
    }

    async fn load_task_comments(
        &self,
        workspace_id: WorkspaceId,
        task_id: Uuid,
    ) -> Result<Vec<CommentText>, DomainError> {
        self.load_comments(comment::Column::TaskId.eq(task_id), workspace_id)
            .await
    }

    async fn load_document_comments(
        &self,
        workspace_id: WorkspaceId,
        document_id: Uuid,
    ) -> Result<Vec<CommentText>, DomainError> {
        self.load_comments(comment::Column::DocumentId.eq(document_id), workspace_id)
            .await
    }

    async fn load_comments(
        &self,
        owner: sea_orm::sea_query::SimpleExpr,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<CommentText>, DomainError> {
        Ok(comment::Entity::find()
            .filter(comment::Column::WorkspaceId.eq(workspace_id.0))
            .filter(owner)
            .filter(comment::Column::DeletedAt.is_null())
            .order_by_asc(comment::Column::CreatedAt)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|row| CommentText { body: row.body })
            .collect())
    }

    async fn load_attachments(
        &self,
        workspace_id: WorkspaceId,
        kind: ResourceKind,
        resource_id: Uuid,
    ) -> Result<Vec<AttachmentText>, DomainError> {
        let owner = match kind {
            ResourceKind::Task => attachment::Column::TaskId.eq(resource_id),
            ResourceKind::Document => attachment::Column::DocumentId.eq(resource_id),
        };

        Ok(attachment::Entity::find()
            .filter(attachment::Column::WorkspaceId.eq(workspace_id.0))
            .filter(owner)
            .filter(attachment::Column::DeletedAt.is_null())
            .order_by_asc(attachment::Column::CreatedAt)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(|row| AttachmentText {
                file_name: row.file_name,
            })
            .collect())
    }
}

#[async_trait]
impl SemanticIndexer for PgSemanticIndexer {
    async fn index_resource(
        &self,
        workspace_id: WorkspaceId,
        kind: ResourceKind,
        resource_id: Uuid,
    ) -> Result<(), DomainError> {
        let chunks = match kind {
            ResourceKind::Task => self.load_task_chunks(workspace_id, resource_id).await?,
            ResourceKind::Document => self.load_document_chunks(workspace_id, resource_id).await?,
        };

        self.writer.index_chunks(&chunks).await?;

        // Content that shrank leaves higher ordinals behind. Dropping them after
        // the upsert — never before — keeps the previous embedding searchable
        // until the new one is committed.
        self.writer
            .prune_chunks_beyond(workspace_id, kind, resource_id, chunks.len() as i32)
            .await?;

        Ok(())
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
