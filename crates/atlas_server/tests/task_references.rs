#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::{
    DomainError,
    entities::boards_tasks::{NewBoard, NewTask, NewTaskReference, PositionBetween, ReferenceKind},
    entities::workspace_core::NewProject,
    ids::{DocumentId, TaskId},
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    BoardRepo, PgBoardRepo, PgProjectRepo, PgTaskReferenceRepo, PgTaskRepo, ProjectRepo,
    TaskReferenceRepo, TaskRepo,
};

async fn seed_project_board_task(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    slug: &str,
    prefix: &str,
) -> (
    atlas_domain::entities::workspace_core::Project,
    atlas_domain::entities::boards_tasks::Board,
    atlas_domain::entities::boards_tasks::Task,
) {
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        ctx,
        NewProject {
            name: format!("Project {slug}"),
            slug: slug.into(),
            task_prefix: prefix.into(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
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
                name: "Main".into(),
            },
        )
        .await
        .expect("seed board");

    let col = PgBoardRepo::new(db.conn().clone())
        .add_column(
            ctx,
            board.id,
            "Backlog".into(),
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
            ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: col.id,
                title: "Task A".into(),
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
        .await
        .expect("seed task");

    (project, board, task)
}

#[tokio::test]
async fn invalid_task_reference_kind_target_mismatch_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "ref-invalid-user").await;
    let ctx = support::ctx(&ws, &user);

    let (_, _, task_a) = seed_project_board_task(&db, &ctx, "ref-proj", "REF").await;

    let ref_repo = PgTaskReferenceRepo::new(db.conn().clone());

    let result = ref_repo
        .create(
            &ctx,
            NewTaskReference {
                source_task_id: task_a.id,
                kind: ReferenceKind::Blocks,
                target_task_id: None,
                target_document_id: Some(DocumentId(uuid::Uuid::new_v4())),
            },
        )
        .await;

    assert!(
        result.is_err(),
        "Blocks reference with a document_id must be rejected"
    );

    match result.unwrap_err() {
        DomainError::InvalidInput { message } => {
            assert!(
                message.contains("task target"),
                "error must mention 'task target'; got: {message}"
            );
        }
        other => panic!("expected InvalidInput, got: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn valid_blocks_reference_with_task_target_is_accepted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "ref-valid-user").await;
    let ctx = support::ctx(&ws, &user);

    let (_, _, task_a) = seed_project_board_task(&db, &ctx, "ref-valid", "RVL").await;

    let task_b = PgTaskRepo::new(db.conn().clone())
        .create(
            &ctx,
            NewTask {
                project_id: task_a.project_id,
                board_id: task_a.board_id,
                column_id: task_a.column_id,
                title: "Task B".into(),
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
        .await
        .expect("seed task B");

    let ref_repo = PgTaskReferenceRepo::new(db.conn().clone());

    let reference = ref_repo
        .create(
            &ctx,
            NewTaskReference {
                source_task_id: task_a.id,
                kind: ReferenceKind::Blocks,
                target_task_id: Some(task_b.id),
                target_document_id: None,
            },
        )
        .await
        .expect("valid Blocks reference must be created");

    assert_eq!(reference.kind, ReferenceKind::Blocks);
    assert_eq!(reference.target_task_id, Some(task_b.id));
    assert_eq!(reference.target_document_id, None);

    db.teardown().await;
}

#[tokio::test]
async fn reference_to_missing_target_task_returns_not_found() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "ref-missing-target").await;
    let ctx = support::ctx(&ws, &user);

    let (_, _, task_a) = seed_project_board_task(&db, &ctx, "ref-missing", "RMT").await;

    let missing_task_id = TaskId::new();
    let ref_repo = PgTaskReferenceRepo::new(db.conn().clone());

    let result = ref_repo
        .create(
            &ctx,
            NewTaskReference {
                source_task_id: task_a.id,
                kind: ReferenceKind::Blocks,
                target_task_id: Some(missing_task_id),
                target_document_id: None,
            },
        )
        .await;

    match result {
        Err(DomainError::NotFound { entity, id }) => {
            assert_eq!(entity, "reference target");
            assert_eq!(id, missing_task_id.0);
        }
        other => panic!("expected NotFound for a missing target task, got: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn self_reference_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "ref-self-user").await;
    let ctx = support::ctx(&ws, &user);

    let (_, _, task_a) = seed_project_board_task(&db, &ctx, "ref-self", "RSF").await;

    let ref_repo = PgTaskReferenceRepo::new(db.conn().clone());

    let result = ref_repo
        .create(
            &ctx,
            NewTaskReference {
                source_task_id: task_a.id,
                kind: ReferenceKind::Parent,
                target_task_id: Some(task_a.id),
                target_document_id: None,
            },
        )
        .await;

    assert!(result.is_err(), "a task may not reference itself");

    match result.unwrap_err() {
        DomainError::InvalidInput { message } => {
            assert!(
                message.contains("itself"),
                "error must mention 'itself'; got: {message}"
            );
        }
        other => panic!("expected InvalidInput, got: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn spec_reference_with_task_id_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "ref-spec-bad-user").await;
    let ctx = support::ctx(&ws, &user);

    let (_, _, task_a) = seed_project_board_task(&db, &ctx, "ref-spec-bad", "RSB").await;

    let task_b_id = TaskId::new();

    let ref_repo = PgTaskReferenceRepo::new(db.conn().clone());

    let result = ref_repo
        .create(
            &ctx,
            NewTaskReference {
                source_task_id: task_a.id,
                kind: ReferenceKind::Spec,
                target_task_id: Some(task_b_id),
                target_document_id: None,
            },
        )
        .await;

    assert!(
        result.is_err(),
        "Spec reference with task_id must be rejected"
    );

    match result.unwrap_err() {
        DomainError::InvalidInput { .. } => {}
        other => panic!("expected InvalidInput, got: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn valid_docs_reference_with_document_target_is_accepted() {
    use sea_orm::ConnectionTrait;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "ref-docs-user").await;
    let ctx = support::ctx(&ws, &user);

    let (_, _, task_a) = seed_project_board_task(&db, &ctx, "ref-docs", "RDC").await;

    let doc_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO documents
               (id, workspace_id, title, content, current_revision_seq,
                created_by_user_id, created_at, updated_at)
               VALUES ('{doc_id}', '{ws_id}', 'Docs', '', 0, '{user_id}', now(), now())"#,
            ws_id = ws.id.0,
            user_id = user.id.0,
        ))
        .await
        .expect("seed document");

    let ref_repo = PgTaskReferenceRepo::new(db.conn().clone());

    let reference = ref_repo
        .create(
            &ctx,
            NewTaskReference {
                source_task_id: task_a.id,
                kind: ReferenceKind::Docs,
                target_task_id: None,
                target_document_id: Some(DocumentId(doc_id)),
            },
        )
        .await
        .expect("valid Docs reference must be created");

    assert_eq!(reference.kind, ReferenceKind::Docs);
    assert_eq!(reference.target_document_id, Some(DocumentId(doc_id)));
    assert_eq!(reference.target_task_id, None);

    db.teardown().await;
}
