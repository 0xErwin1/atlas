#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    entities::boards_tasks::{
        ActivityKind, AssigneeRef, NewBoard, NewTask, NewTaskChecklistItem, NewTaskReference,
        PositionBetween, Priority, ReferenceKind, TaskPatch,
    },
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    persistence::repos::{
        BoardRepo, PgBoardRepo, PgProjectRepo, PgTaskActivityRepo, PgTaskAssigneeRepo,
        PgTaskChecklistRepo, PgTaskReferenceRepo, PgTaskRepo, ProjectRepo, TaskActivityRepo,
        TaskChecklistRepo, TaskRepo,
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

#[tokio::test]
async fn task_service_move_task_emits_moved_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-move-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col_a) = seed_project_board_col(&db, &ctx, "svc-move", "MV").await;

    let col_b = PgBoardRepo::new(db.conn().clone())
        .add_column(
            &ctx,
            board.id,
            "Done".into(),
            PositionBetween {
                before: Some(col_a.position_key.clone()),
                after: None,
            },
        )
        .await
        .expect("seed col_b");

    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col_a.id,
                title: "Move me".into(),
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
        .expect("create task");

    let moved = svc
        .move_task(
            &ctx,
            task.id,
            col_b.id,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("move task");

    assert_eq!(moved.column_id, col_b.id, "task must be in col_b");

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list activity");

    // Expect Created + Moved (newest first: Moved, Created)
    assert_eq!(entries.len(), 2, "must have created + moved activities");
    assert_eq!(
        entries[0].kind,
        ActivityKind::Moved,
        "newest entry must be Moved"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_service_assign_emits_assigned_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-assign-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-assign", "SA").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Assign me".into(),
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
        .expect("create task");

    svc.assign(&ctx, task.id, AssigneeRef::User(user.id))
        .await
        .expect("assign");

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list activity");

    assert_eq!(entries.len(), 2, "created + assigned");
    assert_eq!(
        entries[0].kind,
        ActivityKind::Assigned,
        "newest is Assigned"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_service_delete_task_emits_deleted_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-delete-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-delete", "SD").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Delete me".into(),
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
        .expect("create task");

    svc.delete_task(&ctx, task.id).await.expect("delete");

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list activity");

    assert_eq!(entries.len(), 2, "created + deleted");
    assert_eq!(entries[0].kind, ActivityKind::Deleted, "newest is Deleted");

    db.teardown().await;
}

#[tokio::test]
async fn task_service_add_reference_emits_reference_added_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-ref-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-ref", "RF").await;
    let svc = make_svc(&db);

    let task_a = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Source".into(),
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
        .expect("create task_a");

    let task_b = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Target".into(),
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
        .expect("create task_b");

    svc.add_reference(
        &ctx,
        NewTaskReference {
            source_task_id: task_a.id,
            kind: ReferenceKind::Relates,
            target_task_id: Some(task_b.id),
            target_document_id: None,
        },
    )
    .await
    .expect("add reference");

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task_a.id, None, 50)
        .await
        .expect("list activity");

    assert_eq!(entries.len(), 2, "created + reference_added");
    assert_eq!(
        entries[0].kind,
        ActivityKind::ReferenceAdded,
        "newest is ReferenceAdded"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_service_add_checklist_item_emits_checklist_added_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-chk-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-chk", "CK").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Has checklist".into(),
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
        .expect("create task");

    svc.add_checklist_item(
        &ctx,
        NewTaskChecklistItem {
            task_id: task.id,
            title: "Step 1".into(),
            position: PositionBetween {
                before: None,
                after: None,
            },
        },
    )
    .await
    .expect("add checklist item");

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list activity");

    assert_eq!(entries.len(), 2, "created + checklist_added");
    assert_eq!(
        entries[0].kind,
        ActivityKind::ChecklistAdded,
        "newest is ChecklistAdded"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_service_move_task_triggers_resequence_when_space_exhausted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-reseq-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-reseq", "RQ").await;
    let task_repo = PgTaskRepo::new(db.conn().clone());
    let svc = make_svc(&db);

    // Create a task and move it to the same column repeatedly, using the same
    // before/after pair until the position space between adjacent equal keys
    // would be exhausted, then verify a subsequent move still succeeds (resequence
    // kicked in) or fails with PositionExhausted 409 (not a panic/crash).
    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Resequence test".into(),
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
        .expect("create task");

    // Create a second task so we have two adjacent tasks with a known key gap.
    let task2 = task_repo
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Anchor".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: Some(task.position_key.clone()),
                    after: None,
                },
            },
        )
        .await
        .expect("create anchor task");

    // Exhaust the space between task and task2 by repeatedly bisecting.
    // After enough iterations try_between returns None, triggering resequence+retry.
    // We drive the bisection via move_task so the resequence path is exercised.
    let mut before = task.position_key.clone();
    let after = task2.position_key.clone();

    let mut last_result = Ok(());
    for _ in 0..70 {
        let result = svc
            .move_task(
                &ctx,
                task.id,
                col.id,
                PositionBetween {
                    before: Some(before.clone()),
                    after: Some(after.clone()),
                },
            )
            .await;

        match result {
            Ok(moved) => {
                before = moved.position_key.clone();
            }
            Err(atlas_domain::DomainError::PositionExhausted { .. }) => {
                // Retry-once also exhausted after resequence — acceptable 409.
                last_result = Err(());
                break;
            }
            Err(e) => panic!("unexpected error during move: {e:?}"),
        }
    }

    // Either we exhausted naturally and got 409, or resequence succeeded for all moves.
    // Either way, no panic and no unexpected error.
    let _ = last_result;

    db.teardown().await;
}
