#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    entities::boards_tasks::{
        ActivityKind, NewBoard, NewTask, NewTaskChecklistItem, PositionBetween, Priority, TaskPatch,
    },
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    persistence::repos::{
        BoardRepo, PgBoardRepo, PgProjectRepo, PgTaskActivityRepo, PgTaskAssigneeRepo,
        PgTaskChecklistRepo, PgTaskReferenceRepo, PgTaskRepo, ProjectRepo, TaskActivityRepo,
        TaskChecklistRepo,
    },
    services::TaskService,
};

async fn seed_project_board_col(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    slug: &str,
    prefix: &str,
) -> (
    atlas_domain::entities::workspace_core::Project,
    atlas_domain::entities::boards_tasks::Board,
    atlas_domain::entities::boards_tasks::BoardColumn,
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
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("seed column");

    (project, board, col)
}

fn make_svc(db: &support::TestDb) -> TaskService {
    TaskService::new(
        db.conn().clone(),
        PgTaskRepo::new(db.conn().clone()),
        PgTaskReferenceRepo::new(db.conn().clone()),
        PgTaskAssigneeRepo::new(db.conn().clone()),
        PgTaskChecklistRepo::new(db.conn().clone()),
        PgTaskActivityRepo::new(db.conn().clone()),
    )
}

#[tokio::test]
async fn task_service_create_emits_created_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-create-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-create", "SC").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "First".into(),
                description: String::new(),
                priority: Some(Priority::High),
                due_date: None,
                estimate: Some(3),
                labels: vec!["backend".into()],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("create task");

    assert_eq!(task.priority, Some(Priority::High));
    assert_eq!(task.estimate, Some(3));

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list activity");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, ActivityKind::Created);

    db.teardown().await;
}

#[tokio::test]
async fn task_service_patch_emits_field_changed_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-patch-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-patch", "SP").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Original".into(),
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
        .expect("create");

    svc.patch(
        &ctx,
        task.id,
        TaskPatch {
            title: Some("Updated".into()),
            ..Default::default()
        },
    )
    .await
    .expect("patch");

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list activity");

    assert_eq!(entries.len(), 2, "create + field_changed");
    assert_eq!(entries[0].kind, ActivityKind::FieldChanged, "newest first");

    db.teardown().await;
}

#[tokio::test]
async fn task_service_promote_checklist_item_is_atomic() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-promote-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-promote", "PR").await;
    let svc = make_svc(&db);

    let parent = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Parent".into(),
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
        .expect("create parent");

    let checklist_repo = PgTaskChecklistRepo::new(db.conn().clone());
    let item = checklist_repo
        .add_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: parent.id,
                title: "Sub-item".into(),
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("add checklist item");

    let result = svc
        .promote_checklist_item(&ctx, item.id, proj.id, board.id, col.id)
        .await
        .expect("promote");

    assert_eq!(result.checklist_item.promoted_task_id, Some(result.task.id));
    assert!(result.parent_reference.is_some(), "parent ref must exist");

    let re_promote = svc
        .promote_checklist_item(&ctx, item.id, proj.id, board.id, col.id)
        .await;
    assert!(re_promote.is_err(), "re-promoting must fail");

    db.teardown().await;
}
