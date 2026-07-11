#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::{
    entities::boards_tasks::{NewBoard, NewTask, NewTaskReference, PositionBetween, ReferenceKind},
    entities::documents::{ExtractedLink, NewDocument},
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    BoardRepo, DocumentLinkRepo, DocumentRepo, PgBoardRepo, PgDocumentLinkRepo, PgDocumentRepo,
    PgProjectRepo, PgTaskReferenceRepo, PgTaskRepo, ProjectRepo, TaskReferenceRepo, TaskRepo,
};
use chrono::{TimeDelta, Utc};
use sea_orm::{ConnectionTrait, Statement};

async fn seed_project_board_task(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
) -> atlas_domain::entities::boards_tasks::Task {
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        ctx,
        NewProject {
            name: "DocLink Project".into(),
            slug: "dl-proj".into(),
            task_prefix: "DL".into(),
            visibility: Visibility::Workspace(VisibilityRole::Viewer),
        },
    )
    .await
    .expect("seed project");

    let board = PgBoardRepo::new(db.conn().clone())
        .create_board(
            ctx,
            NewBoard {
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

    PgTaskRepo::new(db.conn().clone())
        .create(
            ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: col.id,
                title: "Task with wikilinks".into(),
                description: "[[Target Doc]]".into(),
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
        .expect("seed task")
}

#[tokio::test]
async fn replace_for_task_source_stores_and_replaces_links() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "dlink-task-user").await;
    let ctx = support::ctx(&ws, &user);

    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 10);
    let link_repo = PgDocumentLinkRepo {
        conn: db.conn().clone(),
    };

    let task = seed_project_board_task(&db, &ctx).await;

    let target_doc = doc_repo
        .create(
            &ctx,
            NewDocument {
                title: "Target Doc".into(),
                slug: None,
                content: "".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create target doc");

    // First write: one link.
    link_repo
        .replace_for_task_source(
            &ctx,
            task.id,
            vec![ExtractedLink {
                target_title: "Target Doc".into(),
                target_document_id: Some(target_doc.id),
            }],
        )
        .await
        .expect("replace_for_task_source (first write)");

    // Confirm backlinks from target_doc shows the task-sourced link.
    let backlinks = link_repo
        .backlinks(&ctx, target_doc.id)
        .await
        .expect("backlinks after first write");

    assert_eq!(backlinks.len(), 1, "one backlink after first write");
    let link = backlinks.first().expect("one backlink");
    assert!(
        link.source_task_id.is_some(),
        "backlink must carry source_task_id"
    );
    assert!(
        link.source_document_id.is_none(),
        "backlink must NOT carry source_document_id"
    );

    // Second write: replace with empty list (removing all links).
    link_repo
        .replace_for_task_source(&ctx, task.id, vec![])
        .await
        .expect("replace_for_task_source (clear)");

    let backlinks_after_clear = link_repo
        .backlinks(&ctx, target_doc.id)
        .await
        .expect("backlinks after clear");

    assert!(
        backlinks_after_clear.is_empty(),
        "links must be removed on second write"
    );

    db.teardown().await;
}

#[tokio::test]
async fn outgoing_for_task_returns_description_and_workspace_scoped_links() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "outgoing-task-user").await;
    let ctx = support::ctx(&ws, &user);
    let (other_ws, other_user) = support::seed_workspace(&db, "outgoing-other-user").await;
    let other_ctx = support::ctx(&other_ws, &other_user);
    let link_repo = PgDocumentLinkRepo {
        conn: db.conn().clone(),
    };

    let task = seed_project_board_task(&db, &ctx).await;
    let other_task = seed_project_board_task(&db, &other_ctx).await;
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 10);
    let target = doc_repo
        .create(
            &ctx,
            NewDocument {
                title: "Target Doc".into(),
                slug: None,
                content: "".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create workspace target")
        .id;
    let other_target = doc_repo
        .create(
            &other_ctx,
            NewDocument {
                title: "Target Doc".into(),
                slug: None,
                content: "".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create other workspace target")
        .id;

    link_repo
        .replace_for_task_source(
            &ctx,
            task.id,
            vec![ExtractedLink {
                target_title: "Target Doc".into(),
                target_document_id: Some(target),
            }],
        )
        .await
        .expect("store workspace link");
    link_repo
        .replace_for_task_source(
            &other_ctx,
            other_task.id,
            vec![ExtractedLink {
                target_title: "Target Doc".into(),
                target_document_id: Some(other_target),
            }],
        )
        .await
        .expect("store other workspace link");

    let snapshot = link_repo
        .outgoing_for_task(&ctx, task.id)
        .await
        .expect("read task snapshot")
        .expect("task exists in workspace");

    assert_eq!(snapshot.description, "[[Target Doc]]");
    assert_eq!(
        snapshot
            .links
            .iter()
            .map(|link| link.target_document_id)
            .collect::<Vec<_>>(),
        vec![Some(target)]
    );

    let hidden = link_repo
        .outgoing_for_task(&ctx, other_task.id)
        .await
        .expect("read cross-workspace task");
    assert!(hidden.is_none());

    db.teardown().await;
}

#[tokio::test]
async fn task_references_are_ordered_by_created_at_then_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "reference-order-user").await;
    let ctx = support::ctx(&ws, &user);
    let task = seed_project_board_task(&db, &ctx).await;
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 10);
    let first_target = doc_repo
        .create(&ctx, new_document("First target"))
        .await
        .expect("create first target");
    let second_target = doc_repo
        .create(&ctx, new_document("Second target"))
        .await
        .expect("create second target");
    let reference_repo = PgTaskReferenceRepo::new(db.conn().clone());

    let later = reference_repo
        .create(
            &ctx,
            NewTaskReference {
                source_task_id: task.id,
                kind: ReferenceKind::Docs,
                target_task_id: None,
                target_document_id: Some(second_target.id),
            },
        )
        .await
        .expect("create later reference");
    let earlier = reference_repo
        .create(
            &ctx,
            NewTaskReference {
                source_task_id: task.id,
                kind: ReferenceKind::Docs,
                target_task_id: None,
                target_document_id: Some(first_target.id),
            },
        )
        .await
        .expect("create earlier reference");
    let now = Utc::now();

    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE task_references SET created_at = $1 WHERE id = $2",
            [(now + TimeDelta::seconds(1)).into(), later.id.0.into()],
        ))
        .await
        .expect("set later timestamp");
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE task_references SET created_at = $1 WHERE id = $2",
            [now.into(), earlier.id.0.into()],
        ))
        .await
        .expect("set earlier timestamp");

    let references = reference_repo
        .list_for_task(&ctx, task.id)
        .await
        .expect("list references");

    assert_eq!(
        references
            .iter()
            .map(|reference| reference.id)
            .collect::<Vec<_>>(),
        vec![earlier.id, later.id]
    );

    db.teardown().await;
}

fn new_document(title: &str) -> NewDocument {
    NewDocument {
        title: title.into(),
        slug: None,
        content: "".into(),
        folder_id: None,
        project_id: None,
        frontmatter: None,
    }
}
