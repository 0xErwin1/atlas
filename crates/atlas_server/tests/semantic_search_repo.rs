#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use async_trait::async_trait;
use atlas_domain::{
    DomainError, WorkspaceCtx,
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        documents::NewDocument,
        identity::MemberRole,
        workspace_core::NewProject,
    },
    ids::WorkspaceId,
    permissions::{Principal, Visibility, VisibilityRole},
    semantic_search::{
        EmbeddingInput, EmbeddingProvider, ResourceKind, SemanticIndexChunk, SemanticSearchQuery,
        SemanticSearchRepo, SemanticSearchSource, SemanticSearchTypeFilter,
    },
};
use atlas_server::{
    persistence::repos::{
        BoardRepo, DocumentRepo, MembershipRepo, PgBoardRepo, PgDocumentRepo, PgMembershipRepo,
        PgProjectRepo, PgSemanticIndexWriter, PgSemanticSearchRepo, PgTaskRepo, ProjectRepo,
        TaskRepo, UserRepo,
    },
    semantic_indexer::{
        AttachmentText, ChecklistText, CommentText, SubtaskText, TaskIndexInput,
        aggregate_task_chunks,
    },
};
use std::{error::Error, io, sync::Arc};
use uuid::Uuid;

#[derive(Debug)]
struct SeededProvider;

#[async_trait]
impl EmbeddingProvider for SeededProvider {
    async fn embed(&self, inputs: &[EmbeddingInput]) -> Result<Vec<Vec<f32>>, DomainError> {
        Ok(inputs.iter().map(|input| vector_for(&input.text)).collect())
    }

    fn model(&self) -> &str {
        "seeded-semantic-test"
    }

    fn dimensions(&self) -> usize {
        1536
    }
}

fn vector_for(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0; 1536];
    let axis = if text.contains("alpha") {
        0
    } else if text.contains("beta") {
        1
    } else {
        2
    };
    if let Some(value) = vector.get_mut(axis) {
        *value = 1.0;
    }
    vector
}

async fn seed_private_project_doc_and_task(
    db: &support::TestDb,
    owner_ctx: &WorkspaceCtx,
) -> (Uuid, Uuid) {
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        owner_ctx,
        NewProject {
            name: "Private Project".to_owned(),
            slug: "private-project".to_owned(),
            task_prefix: "SEM".to_owned(),
            visibility: Visibility::Private,
        },
    )
    .await
    .expect("seed private project");

    let doc = PgDocumentRepo::new(db.conn().clone(), 50)
        .create(
            owner_ctx,
            NewDocument {
                title: "Semantic Alpha Runbook".to_owned(),
                slug: None,
                content: "alpha recovery procedure".to_owned(),
                folder_id: None,
                project_id: Some(project.id),
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");

    let board = PgBoardRepo::new(db.conn().clone())
        .create_board(
            owner_ctx,
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
            owner_ctx,
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
    let task = PgTaskRepo::new(db.conn().clone())
        .create(
            owner_ctx,
            NewTask {
                column_id: column.id,
                board_id: board.id,
                project_id: project.id,
                title: "Beta follow-up".to_owned(),
                description: "beta incident review".to_owned(),
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
        .await
        .expect("seed task");

    (doc.id.0, task.id.0)
}

#[tokio::test]
async fn semantic_search_repo_dedups_stale_rows_and_filters_permissions()
-> Result<(), Box<dyn Error>> {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, owner) = support::seed_workspace(&db, "semantic-owner").await;
    let owner_ctx = support::ctx(&workspace, &owner);
    let denied_user = db
        .user_repo()
        .create(atlas_server::persistence::repos::NewUser {
            username: "semantic-denied".to_owned(),
            display_name: "semantic-denied".to_owned(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await?;
    PgMembershipRepo {
        conn: db.conn().clone(),
    }
    .add(&owner_ctx, denied_user.id, MemberRole::Member)
    .await?;

    let (doc_id, task_id) = seed_private_project_doc_and_task(&db, &owner_ctx).await;
    let unindexed_doc = PgDocumentRepo::new(db.conn().clone(), 50)
        .create(
            &owner_ctx,
            NewDocument {
                title: "Unindexed Alpha Note".to_owned(),
                slug: None,
                content: "alpha content without an embedding row".to_owned(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await?;
    let provider = Arc::new(SeededProvider);
    let writer = PgSemanticIndexWriter::new(db.conn().clone(), provider.clone());
    writer
        .index_chunks(&[
            chunk(
                workspace.id,
                ResourceKind::Document,
                doc_id,
                0,
                "alpha exact match",
            ),
            chunk(
                workspace.id,
                ResourceKind::Document,
                doc_id,
                1,
                "alpha duplicate chunk",
            ),
            chunk(
                workspace.id,
                ResourceKind::Task,
                task_id,
                0,
                "beta task chunk",
            ),
        ])
        .await?;
    writer
        .mark_resource_stale(workspace.id, ResourceKind::Task, task_id)
        .await?;

    let repo = PgSemanticSearchRepo::new(db.conn().clone(), provider);
    let owner_hits = repo
        .search(&SemanticSearchQuery::new(
            workspace.id,
            Principal::User(owner.id),
            "alpha".to_owned(),
            SemanticSearchTypeFilter::all(),
            10,
            None,
            false,
            true,
            true,
        ))
        .await?;
    assert_eq!(owner_hits.len(), 1);
    let owner_hit = owner_hits
        .first()
        .ok_or_else(|| io::Error::other("missing owner semantic hit"))?;
    assert_eq!(owner_hit.kind, ResourceKind::Document);
    assert_eq!(owner_hit.id, doc_id);
    assert_eq!(owner_hit.source, SemanticSearchSource::Aggregate);
    assert!(owner_hit.similarity > 0.99);
    assert!(owner_hits.iter().all(|hit| hit.id != unindexed_doc.id.0));

    let denied_hits = repo
        .search(&SemanticSearchQuery::new(
            workspace.id,
            Principal::User(denied_user.id),
            "alpha".to_owned(),
            SemanticSearchTypeFilter::all(),
            10,
            None,
            false,
            true,
            true,
        ))
        .await?;
    assert!(denied_hits.is_empty());

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn semantic_search_returns_task_from_inherited_visible_task_text()
-> Result<(), Box<dyn Error>> {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, owner) = support::seed_workspace(&db, "semantic-derived-owner").await;
    let owner_ctx = support::ctx(&workspace, &owner);

    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        &owner_ctx,
        NewProject {
            name: "Derived Source Project".to_owned(),
            slug: "derived-source-project".to_owned(),
            task_prefix: "DRV".to_owned(),
            visibility: Visibility::Workspace(VisibilityRole::Viewer),
        },
    )
    .await?;
    let board = PgBoardRepo::new(db.conn().clone())
        .create_board(
            &owner_ctx,
            NewBoard {
                folder_id: None,
                project_id: project.id,
                name: "Board".to_owned(),
            },
        )
        .await?;
    let column = PgBoardRepo::new(db.conn().clone())
        .add_column(
            &owner_ctx,
            board.id,
            "Backlog".to_owned(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await?;
    let task = PgTaskRepo::new(db.conn().clone())
        .create(
            &owner_ctx,
            NewTask {
                column_id: column.id,
                board_id: board.id,
                project_id: project.id,
                title: "Routine planning".to_owned(),
                description: "Prepare ordinary coordination notes".to_owned(),
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

    let chunks = aggregate_task_chunks(TaskIndexInput {
        workspace_id: workspace.id,
        task_id: task.id.0,
        readable_id: task.readable_id.clone(),
        title: task.title.clone(),
        description: task.description.clone(),
        labels: vec![],
        comments: vec![CommentText {
            body: "alpha customer escalation context".to_owned(),
        }],
        attachments: vec![AttachmentText {
            file_name: "alpha-evidence-log.txt".to_owned(),
        }],
        checklist_items: vec![ChecklistText {
            title: "alpha checklist verification".to_owned(),
        }],
        subtasks: vec![SubtaskText {
            readable_id: "DRV-99".to_owned(),
            title: "alpha subtask investigation".to_owned(),
            description: "collect inherited-visible details".to_owned(),
            checklist_items: vec![ChecklistText {
                title: "alpha subtask checklist".to_owned(),
            }],
        }],
        max_chunk_chars: 1000,
    });
    let aggregate_chunk = chunks
        .first()
        .ok_or_else(|| io::Error::other("missing aggregate chunk"))?;
    assert!(
        aggregate_chunk
            .text
            .contains("alpha customer escalation context")
    );
    assert!(aggregate_chunk.text.contains("alpha-evidence-log.txt"));
    assert!(
        aggregate_chunk
            .text
            .contains("alpha checklist verification")
    );
    assert!(aggregate_chunk.text.contains("alpha subtask investigation"));

    let provider = Arc::new(SeededProvider);
    PgSemanticIndexWriter::new(db.conn().clone(), provider.clone())
        .index_chunks(&chunks)
        .await?;
    let hits = PgSemanticSearchRepo::new(db.conn().clone(), provider)
        .search(&SemanticSearchQuery::new(
            workspace.id,
            Principal::User(owner.id),
            "alpha".to_owned(),
            SemanticSearchTypeFilter::tasks(),
            10,
            None,
            false,
            true,
            true,
        ))
        .await?;

    assert_eq!(hits.len(), 1);
    let hit = hits
        .first()
        .ok_or_else(|| io::Error::other("missing derived semantic hit"))?;
    assert_eq!(hit.kind, ResourceKind::Task);
    assert_eq!(hit.id, task.id.0);
    assert_eq!(hit.source, SemanticSearchSource::Aggregate);
    assert_eq!(hit.readable_id.as_deref(), Some(task.readable_id.as_str()));
    assert!(hit.excerpt.contains("alpha customer escalation context"));

    db.teardown().await;
    Ok(())
}

fn chunk(
    workspace_id: WorkspaceId,
    kind: ResourceKind,
    resource_id: Uuid,
    chunk_ordinal: i32,
    text: &str,
) -> SemanticIndexChunk {
    SemanticIndexChunk {
        workspace_id,
        kind,
        resource_id,
        source: SemanticSearchSource::Aggregate,
        chunk_ordinal,
        content_hash: text.to_owned(),
        text: text.to_owned(),
        excerpt: text.to_owned(),
    }
}
