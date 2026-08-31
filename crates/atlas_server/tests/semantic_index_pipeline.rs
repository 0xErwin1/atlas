//! End-to-end coverage for ATL-155: a write enqueues indexing work, and the
//! worker turns that work into rows in `search_embeddings`.
//!
//! These tests drive `SearchIndexWorker::drain_once` directly rather than
//! spawning the polling loop, so they assert the pipeline without racing a
//! timer.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use async_trait::async_trait;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::boards_tasks::NewBoard;
use atlas_acta::entities::boards_tasks::NewTask;
use atlas_acta::entities::boards_tasks::PositionBetween;
use atlas_acta::entities::comments::CommentOwner;
use atlas_acta::entities::documents::NewDocument;
use atlas_acta::entities::workspace_core::NewProject;
use atlas_acta::ids::DocumentId;
use atlas_acta::permissions::Visibility;
use atlas_acta::semantic_search::EmbeddingInput;
use atlas_acta::semantic_search::EmbeddingProvider;
use atlas_core::error::DomainError;
use atlas_server::{
    persistence::repos::{
        BoardRepo, PgBoardRepo, PgProjectRepo, PgSemanticIndexWriter, PgSemanticIndexer,
        ProjectRepo,
    },
    search_indexer::SearchIndexWorker,
    services::{CommentService, DocumentService, TaskService},
};
use sea_orm::{DatabaseBackend, FromQueryResult, Statement};
use std::{error::Error, sync::Arc};
use uuid::Uuid;

/// Distinct vectors per text so a reindex is observable, and so a chunk that did
/// not change keeps its hash.
#[derive(Debug)]
struct TextSeededProvider;

#[async_trait]
impl EmbeddingProvider for TextSeededProvider {
    async fn embed(&self, inputs: &[EmbeddingInput]) -> Result<Vec<Vec<f32>>, DomainError> {
        Ok(inputs
            .iter()
            .map(|input| {
                let mut vector = vec![0.0_f32; 1536];
                if let Some(slot) = vector.get_mut(input.text.len() % 1536) {
                    *slot = 1.0;
                }
                vector
            })
            .collect())
    }

    fn model(&self) -> &str {
        "atl155-pipeline-test"
    }

    fn dimensions(&self) -> usize {
        1536
    }
}

fn worker(db: &support::TestDb) -> SearchIndexWorker {
    let writer = Arc::new(PgSemanticIndexWriter::new(
        db.conn().clone(),
        Arc::new(TextSeededProvider),
    ));
    let indexer = Arc::new(PgSemanticIndexer::new(db.conn().clone(), writer));
    SearchIndexWorker::new(
        db.conn().clone(),
        indexer,
        std::time::Duration::from_millis(50),
        16,
    )
}

#[derive(Debug, FromQueryResult)]
struct EmbeddingRow {
    content_hash: String,
    excerpt: String,
}

async fn embedding_rows(db: &support::TestDb, resource_id: Uuid) -> Vec<EmbeddingRow> {
    EmbeddingRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT content_hash, excerpt FROM acta.search_embeddings \
         WHERE resource_id = $1 ORDER BY chunk_ordinal",
        vec![resource_id.into()],
    ))
    .all(db.conn())
    .await
    .expect("read search_embeddings")
}

/// The one chunk a short resource indexes into.
///
/// Asserts the count rather than indexing, so a resource that unexpectedly
/// spans several chunks fails with that fact instead of a panic on `[0]`.
async fn only_embedding(db: &support::TestDb, resource_id: Uuid) -> EmbeddingRow {
    let mut rows = embedding_rows(db, resource_id).await;
    assert_eq!(rows.len(), 1, "expected exactly one indexed chunk");
    rows.pop().expect("length asserted above")
}

async fn queue_depth(db: &support::TestDb) -> i64 {
    #[derive(Debug, FromQueryResult)]
    struct CountRow {
        count: i64,
    }

    CountRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT COUNT(*)::bigint AS count FROM acta.search_index_queue",
        vec![],
    ))
    .one(db.conn())
    .await
    .expect("count queue")
    .map(|row| row.count)
    .unwrap_or_default()
}

async fn seed_project(db: &support::TestDb, ctx: &WorkspaceCtx, slug: &str) -> NewProjectIds {
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        ctx,
        NewProject {
            name: slug.to_owned(),
            slug: slug.to_owned(),
            task_prefix: "IDX".to_owned(),
            visibility: Visibility::Private,
        },
    )
    .await
    .expect("seed project");

    let board = PgBoardRepo::new(db.conn().clone())
        .create_board(
            ctx,
            NewBoard {
                folder_id: None,
                project_id: project.id,
                name: "Board".to_owned(),
            },
        )
        .await
        .expect("seed board");

    let column = PgBoardRepo::new(db.conn().clone())
        .add_column(
            ctx,
            board.id,
            "Backlog".to_owned(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("seed column");

    NewProjectIds {
        project_id: project.id,
        board_id: board.id,
        column_id: column.id,
    }
}

struct NewProjectIds {
    project_id: atlas_acta::ids::ProjectId,
    board_id: atlas_acta::ids::BoardId,
    column_id: atlas_acta::ids::ColumnId,
}

/// The ATL-155 acceptance criterion: creating a document produces a row in
/// `search_embeddings`, and editing it reindexes.
#[tokio::test]
async fn creating_and_editing_a_document_indexes_and_reindexes_it() -> Result<(), Box<dyn Error>> {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, owner) = support::seed_workspace(&db, "atl155-doc").await;
    let ctx = support::ctx(&workspace, &owner);
    let ids = seed_project(&db, &ctx, "atl155-doc-project").await;

    let documents = DocumentService::new(db.conn().clone(), 50);
    let doc = documents
        .create(
            &ctx,
            NewDocument {
                title: "Recovery Runbook".to_owned(),
                slug: None,
                content: "restore the primary from the latest base backup".to_owned(),
                folder_id: None,
                project_id: Some(ids.project_id),
                frontmatter: None,
            },
        )
        .await?;

    assert!(
        embedding_rows(&db, doc.id.0).await.is_empty(),
        "the write itself must not embed inline"
    );
    assert_eq!(queue_depth(&db).await, 1, "the write must enqueue the doc");

    worker(&db).drain_once().await?;

    let after_create = only_embedding(&db, doc.id.0).await;
    assert!(
        after_create.excerpt.contains("base backup"),
        "excerpt must carry the indexed content, got {:?}",
        after_create.excerpt
    );
    assert_eq!(
        queue_depth(&db).await,
        0,
        "drained work must leave the queue"
    );

    let head = doc.current_revision_id;
    documents
        .update_content(
            &ctx,
            doc.id,
            head,
            "failover to the warm standby in eu-west",
        )
        .await?;

    assert_eq!(queue_depth(&db).await, 1, "the edit must re-enqueue");
    worker(&db).drain_once().await?;

    // Exactly one row again: the reindex must replace the chunk, not add one.
    let after_edit = only_embedding(&db, doc.id.0).await;
    assert_ne!(
        after_edit.content_hash, after_create.content_hash,
        "edited content must produce a new embedding"
    );
    assert!(
        after_edit.excerpt.contains("warm standby"),
        "excerpt must reflect the edit, got {:?}",
        after_edit.excerpt
    );

    db.teardown().await;
    Ok(())
}

/// Comments, checklist items and subtasks are part of a task's indexed text, so
/// each of them must be able to trigger a reindex on its own.
#[tokio::test]
async fn task_side_writes_reindex_the_task_aggregate() -> Result<(), Box<dyn Error>> {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, owner) = support::seed_workspace(&db, "atl155-task").await;
    let ctx = support::ctx(&workspace, &owner);
    let ids = seed_project(&db, &ctx, "atl155-task-project").await;

    let tasks = TaskService::new(db.conn().clone());
    let task = tasks
        .create(
            &ctx,
            NewTask {
                project_id: ids.project_id,
                board_id: ids.board_id,
                column_id: ids.column_id,
                title: "Rotate the signing keys".to_owned(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await?;

    worker(&db).drain_once().await?;
    let baseline = only_embedding(&db, task.id.0).await;

    CommentService::new(db.conn().clone())
        .create(
            &ctx,
            CommentOwner::Task(task.id),
            "blocked on the HSM maintenance window".to_owned(),
        )
        .await?;

    assert_eq!(
        queue_depth(&db).await,
        1,
        "a comment must re-enqueue its task"
    );
    worker(&db).drain_once().await?;

    let after_comment = only_embedding(&db, task.id.0).await;
    assert_ne!(
        after_comment.content_hash, baseline.content_hash,
        "the comment must land in the task's indexed text"
    );
    assert!(
        after_comment.excerpt.contains("HSM maintenance"),
        "excerpt must include the comment, got {:?}",
        after_comment.excerpt
    );

    db.teardown().await;
    Ok(())
}

/// A resource whose text shrinks must lose its trailing chunks, otherwise the
/// leftovers keep matching content the resource no longer has.
#[tokio::test]
async fn shrinking_a_document_prunes_its_trailing_chunks() -> Result<(), Box<dyn Error>> {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, owner) = support::seed_workspace(&db, "atl155-prune").await;
    let ctx = support::ctx(&workspace, &owner);
    let ids = seed_project(&db, &ctx, "atl155-prune-project").await;

    // Comfortably past the 1500-char chunk ceiling, so this indexes as several
    // chunks and the shrink below has something to prune.
    let long_content = "incident timeline entry ".repeat(400);

    let documents = DocumentService::new(db.conn().clone(), 50);
    let doc = documents
        .create(
            &ctx,
            NewDocument {
                title: "Postmortem".to_owned(),
                slug: None,
                content: long_content,
                folder_id: None,
                project_id: Some(ids.project_id),
                frontmatter: None,
            },
        )
        .await?;

    worker(&db).drain_once().await?;
    let long_rows = embedding_rows(&db, doc.id.0).await;
    assert!(
        long_rows.len() > 1,
        "fixture must span multiple chunks, got {}",
        long_rows.len()
    );

    let head = reload_head(&db, doc.id).await;
    documents
        .update_content(&ctx, doc.id, head, "resolved")
        .await?;
    worker(&db).drain_once().await?;

    assert_eq!(
        embedding_rows(&db, doc.id.0).await.len(),
        1,
        "the trailing chunks of the previous version must be gone"
    );

    db.teardown().await;
    Ok(())
}

async fn reload_head(db: &support::TestDb, id: DocumentId) -> atlas_acta::ids::RevisionId {
    #[derive(Debug, FromQueryResult)]
    struct HeadRow {
        current_revision_id: Option<Uuid>,
    }

    let row = HeadRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT current_revision_id FROM acta.documents WHERE id = $1",
        vec![id.0.into()],
    ))
    .one(db.conn())
    .await
    .expect("read document head")
    .expect("document exists");

    atlas_acta::ids::RevisionId(row.current_revision_id.expect("document has a head"))
}
