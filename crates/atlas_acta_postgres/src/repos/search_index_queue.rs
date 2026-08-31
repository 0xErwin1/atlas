use atlas_acta::entities::comments::CommentOwner;
use atlas_acta::ids::DocumentId;
use atlas_acta::ids::TaskId;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::semantic_search::ResourceKind;
use atlas_core::error::DomainError;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

use super::semantic_search::ResourceKindSql;
use atlas_postgres::db_err;
use uuid::Uuid;

/// A resource waiting to be re-embedded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedResource {
    pub id: Uuid,
    pub workspace_id: WorkspaceId,
    pub kind: ResourceKind,
    pub resource_id: Uuid,
    pub attempt_count: i32,
    /// When this row last became dirty. Carried through so `complete` can avoid
    /// deleting a re-enqueue that landed mid-embedding.
    pub enqueued_at: chrono::DateTime<chrono::Utc>,
}

/// What a workspace-wide backfill would cost, and how far it already is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceIndexPlan {
    pub documents: i64,
    pub tasks: i64,
    pub characters: i64,
    pub estimated_chunks: i64,
    pub estimated_tokens: i64,
    /// Resources with a live embedding for the current model and dimensions.
    pub indexed_resources: i64,
    /// Resources already waiting in the queue, backfilled or not.
    pub queued_resources: i64,
}

/// Approximates the tokens a corpus of `characters` will bill.
///
/// Four characters per token is the usual English rule of thumb for the
/// BPE vocabularies these providers use. It is deliberately coarse: the point
/// is to tell an operator whether a backfill costs cents or hundreds of
/// dollars before they start it, not to predict an invoice.
fn estimated_tokens(characters: i64) -> i64 {
    characters / 4
}

/// Persistence for the semantic-index work queue.
///
/// The queue exists so the embedding provider's latency never lands inside an
/// HTTP request: writers enqueue inside their own transaction, and the indexer
/// worker drains the queue out of band.
pub struct PgSearchIndexQueueRepo;

impl PgSearchIndexQueueRepo {
    /// Marks a resource as needing re-indexing.
    ///
    /// MUST run on the same `ConnectionTrait` as the domain mutation, so a
    /// rollback leaves no queue row claiming work that never happened.
    ///
    /// Repeat enqueues of the same resource coalesce onto the single existing
    /// row: a task edited ten times before the worker wakes up is embedded once.
    /// The conflict branch resets the backoff so a fresh edit is not held back
    /// by a previous failure's `next_attempt_at`.
    pub async fn enqueue_in(
        conn: &impl ConnectionTrait,
        workspace_id: WorkspaceId,
        kind: ResourceKind,
        resource_id: Uuid,
    ) -> Result<(), DomainError> {
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"INSERT INTO search_index_queue (
                   id, workspace_id, resource_kind, resource_id,
                   enqueued_at, attempt_count, next_attempt_at, locked_until, last_error
               ) VALUES (gen_random_uuid(), $1, $2, $3, now(), 0, now(), NULL, NULL)
               ON CONFLICT (workspace_id, resource_kind, resource_id)
               DO UPDATE SET
                   enqueued_at = now(),
                   attempt_count = 0,
                   next_attempt_at = now(),
                   locked_until = NULL,
                   last_error = NULL"#,
            vec![
                workspace_id.0.into(),
                kind.db_str().into(),
                resource_id.into(),
            ],
        ))
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Marks the document behind a write as needing re-indexing.
    pub async fn enqueue_document_in(
        conn: &impl ConnectionTrait,
        workspace_id: WorkspaceId,
        document_id: DocumentId,
    ) -> Result<(), DomainError> {
        Self::enqueue_in(conn, workspace_id, ResourceKind::Document, document_id.0).await
    }

    /// Marks the task behind a write as needing re-indexing.
    ///
    /// Callers editing a subtask should enqueue the parent as well: a subtask's
    /// text is part of the parent's indexed aggregate.
    pub async fn enqueue_task_in(
        conn: &impl ConnectionTrait,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) -> Result<(), DomainError> {
        Self::enqueue_in(conn, workspace_id, ResourceKind::Task, task_id.0).await
    }

    /// Marks whichever resource owns a comment as needing re-indexing.
    pub async fn enqueue_comment_owner_in(
        conn: &impl ConnectionTrait,
        workspace_id: WorkspaceId,
        owner: CommentOwner,
    ) -> Result<(), DomainError> {
        match owner {
            CommentOwner::Task(id) => Self::enqueue_task_in(conn, workspace_id, id).await,
            CommentOwner::Document(id) => Self::enqueue_document_in(conn, workspace_id, id).await,
        }
    }

    /// Enqueues every live document and task of a workspace.
    ///
    /// Existing rows are left alone: a resource already waiting is dirtier than
    /// the backfill knows, and resetting its backoff would let a failing
    /// resource be retried by every backfill run instead of its own schedule.
    ///
    /// Idempotent by construction — re-running enqueues only what is not
    /// already queued, and the indexer skips a resource whose content hash is
    /// unchanged, so a repeated backfill costs queue rows, not embeddings.
    pub async fn enqueue_workspace(
        conn: &impl ConnectionTrait,
        workspace_id: WorkspaceId,
    ) -> Result<u64, DomainError> {
        let mut enqueued: u64 = 0;
        for (kind, table) in [
            (ResourceKind::Document, "acta.documents"),
            (ResourceKind::Task, "tasks"),
        ] {
            let result = conn
                .execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    format!(
                        r#"INSERT INTO search_index_queue (
                               id, workspace_id, resource_kind, resource_id,
                               enqueued_at, attempt_count, next_attempt_at, locked_until, last_error
                           )
                           SELECT gen_random_uuid(), $1, $2, source.id, now(), 0, now(), NULL, NULL
                           FROM {table} source
                           WHERE source.workspace_id = $1
                             AND source.deleted_at IS NULL
                           ON CONFLICT (workspace_id, resource_kind, resource_id) DO NOTHING"#
                    ),
                    vec![workspace_id.0.into(), kind.db_str().into()],
                ))
                .await
                .map_err(db_err)?;
            enqueued = enqueued.saturating_add(result.rows_affected());
        }
        Ok(enqueued)
    }

    /// Counts what a workspace-wide backfill would embed, and what it already has.
    ///
    /// Read-only: this is what an operator sees before paying a provider to
    /// embed a whole corpus.
    pub async fn workspace_plan(
        conn: &impl ConnectionTrait,
        workspace_id: WorkspaceId,
        model: &str,
        dimensions: i32,
        max_chunk_chars: i64,
    ) -> Result<WorkspaceIndexPlan, DomainError> {
        let row = PlanRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"WITH document_text AS (
                   SELECT char_length(coalesce(d.title, '')) + char_length(coalesce(d.content, ''))
                          + coalesce((
                              SELECT sum(char_length(c.body))
                              FROM comments c
                              WHERE c.document_id = d.id AND c.deleted_at IS NULL
                          ), 0) AS characters
                   FROM acta.documents d
                   WHERE d.workspace_id = $1 AND d.deleted_at IS NULL
               ), task_text AS (
                   SELECT char_length(coalesce(t.title, '')) + char_length(coalesce(t.description, ''))
                          + coalesce((
                              SELECT sum(char_length(c.body))
                              FROM comments c
                              WHERE c.task_id = t.id AND c.deleted_at IS NULL
                          ), 0) AS characters
                   FROM tasks t
                   WHERE t.workspace_id = $1 AND t.deleted_at IS NULL
               ), corpus AS (
                   SELECT * FROM document_text UNION ALL SELECT * FROM task_text
               )
               SELECT (SELECT count(*) FROM document_text)::bigint AS documents,
                      (SELECT count(*) FROM task_text)::bigint AS tasks,
                      (SELECT coalesce(sum(characters), 0) FROM corpus)::bigint AS characters,
                      (SELECT coalesce(sum(greatest(ceil(characters::numeric / $4), 1)), 0)
                       FROM corpus WHERE characters > 0)::bigint AS estimated_chunks,
                      (SELECT count(DISTINCT (resource_kind, resource_id))
                       FROM search_embeddings
                       WHERE workspace_id = $1
                         AND model = $2
                         AND dimensions = $3
                         AND stale_at IS NULL)::bigint AS indexed_resources,
                      (SELECT count(*) FROM search_index_queue WHERE workspace_id = $1)::bigint AS queued_resources"#,
            vec![
                workspace_id.0.into(),
                model.to_owned().into(),
                dimensions.into(),
                max_chunk_chars.max(1).into(),
            ],
        ))
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or_else(|| DomainError::Internal {
            message: "semantic backfill plan query returned no row".to_owned(),
        })?;

        Ok(WorkspaceIndexPlan {
            documents: row.documents,
            tasks: row.tasks,
            characters: row.characters,
            estimated_chunks: row.estimated_chunks,
            estimated_tokens: estimated_tokens(row.characters),
            indexed_resources: row.indexed_resources,
            queued_resources: row.queued_resources,
        })
    }

    /// Claims up to `batch_size` due rows, leasing them for `lease_seconds`.
    ///
    /// `FOR UPDATE SKIP LOCKED` keeps concurrent workers from claiming the same
    /// row; an expired lease makes the row claimable again, so a worker that
    /// dies mid-batch loses no work.
    pub async fn claim_batch(
        conn: &impl ConnectionTrait,
        batch_size: i64,
        lease_seconds: i64,
    ) -> Result<Vec<QueuedResource>, DomainError> {
        let rows = QueueRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"UPDATE search_index_queue
               SET locked_until = now() + ($2 * INTERVAL '1 second')
               WHERE id IN (
                   SELECT id FROM search_index_queue
                   WHERE next_attempt_at <= now()
                     AND (locked_until IS NULL OR locked_until <= now())
                   ORDER BY enqueued_at
                   FOR UPDATE SKIP LOCKED
                   LIMIT $1
               )
               RETURNING id, workspace_id, resource_kind, resource_id, attempt_count, enqueued_at"#,
            vec![batch_size.into(), lease_seconds.into()],
        ))
        .all(conn)
        .await
        .map_err(db_err)?;

        rows.into_iter().map(QueueRow::into_domain).collect()
    }

    /// Drops a row once its resource has been embedded.
    ///
    /// Scoped by `enqueued_at` so a re-enqueue that landed while the worker was
    /// embedding is not deleted along with the work it already superseded.
    pub async fn complete(
        conn: &impl ConnectionTrait,
        id: Uuid,
        claimed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), DomainError> {
        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM search_index_queue WHERE id = $1 AND enqueued_at <= $2",
            vec![id.into(), claimed_at.into()],
        ))
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Releases a failed row with exponential backoff, capped at one hour.
    pub async fn fail(
        conn: &impl ConnectionTrait,
        id: Uuid,
        attempt_count: i32,
        error: &str,
    ) -> Result<(), DomainError> {
        let backoff_seconds = backoff_seconds(attempt_count);

        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"UPDATE search_index_queue
               SET attempt_count = attempt_count + 1,
                   next_attempt_at = now() + ($2 * INTERVAL '1 second'),
                   locked_until = NULL,
                   last_error = $3
               WHERE id = $1"#,
            vec![id.into(), backoff_seconds.into(), error.into()],
        ))
        .await
        .map_err(db_err)?;
        Ok(())
    }
}

fn backoff_seconds(attempt_count: i32) -> i64 {
    const MAX_BACKOFF_SECONDS: i64 = 3_600;
    const BASE_SECONDS: i64 = 5;

    let exponent = attempt_count.clamp(0, 16) as u32;
    BASE_SECONDS
        .saturating_mul(2_i64.saturating_pow(exponent))
        .min(MAX_BACKOFF_SECONDS)
}

#[derive(Debug, FromQueryResult)]
struct PlanRow {
    documents: i64,
    tasks: i64,
    characters: i64,
    estimated_chunks: i64,
    indexed_resources: i64,
    queued_resources: i64,
}

#[derive(Debug, FromQueryResult)]
struct QueueRow {
    id: Uuid,
    workspace_id: Uuid,
    resource_kind: String,
    resource_id: Uuid,
    attempt_count: i32,
    enqueued_at: chrono::DateTime<chrono::Utc>,
}

impl QueueRow {
    fn into_domain(self) -> Result<QueuedResource, DomainError> {
        let kind = match self.resource_kind.as_str() {
            "document" => ResourceKind::Document,
            "task" => ResourceKind::Task,
            other => {
                return Err(DomainError::Internal {
                    message: format!("unknown queued resource kind: {other}"),
                });
            }
        };

        Ok(QueuedResource {
            id: self.id,
            workspace_id: WorkspaceId(self.workspace_id),
            kind,
            resource_id: self.resource_id,
            attempt_count: self.attempt_count,
            enqueued_at: self.enqueued_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_saturates_at_one_hour() {
        assert_eq!(backoff_seconds(0), 5);
        assert_eq!(backoff_seconds(1), 10);
        assert_eq!(backoff_seconds(4), 80);
        assert_eq!(backoff_seconds(20), 3_600);
    }

    #[test]
    fn queue_row_rejects_unknown_kinds() {
        let row = QueueRow {
            id: Uuid::nil(),
            workspace_id: Uuid::nil(),
            resource_kind: "board".to_owned(),
            resource_id: Uuid::nil(),
            attempt_count: 0,
            enqueued_at: chrono::Utc::now(),
        };

        assert!(row.into_domain().is_err());
    }

    #[test]
    fn queue_row_maps_known_kinds() {
        let row = QueueRow {
            id: Uuid::nil(),
            workspace_id: Uuid::nil(),
            resource_kind: "task".to_owned(),
            resource_id: Uuid::nil(),
            attempt_count: 3,
            enqueued_at: chrono::Utc::now(),
        };

        let queued = row.into_domain().expect("task is a known kind");
        assert_eq!(queued.kind, ResourceKind::Task);
        assert_eq!(queued.attempt_count, 3);
    }
}
