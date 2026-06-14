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
        BoardRepo, PgBoardRepo, PgProjectRepo, PgTaskActivityRepo, PgTaskChecklistRepo, PgTaskRepo,
        ProjectRepo, TaskActivityRepo, TaskChecklistRepo, TaskRepo,
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
    TaskService::new(db.conn().clone())
}

/// Forces a row's `position_key` directly, creating the exhausted (duplicate-key)
/// state that only a resequence can break. `try_between` returns `None` for equal
/// anchors, so this is the one scenario where the rebalance must actually help.
async fn force_position_key(db: &support::TestDb, table: &str, id: uuid::Uuid, key: &str) {
    use sea_orm::{ConnectionTrait, Statement};
    db.conn()
        .execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!("UPDATE {table} SET position_key = '{key}' WHERE id = '{id}'"),
        ))
        .await
        .expect("force position_key");
}

/// Reads a row's current `position_key`.
async fn read_position_key(db: &support::TestDb, table: &str, id: uuid::Uuid) -> String {
    use sea_orm::{ConnectionTrait, Statement};
    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT position_key FROM {table} WHERE id = '{id}'"),
        ))
        .await
        .expect("query position_key")
        .expect("row exists");
    row.try_get("", "position_key").expect("position_key")
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
        .promote_checklist_item(&ctx, parent.id, item.id, proj.id, board.id, col.id)
        .await
        .expect("promote");

    assert_eq!(result.checklist_item.promoted_task_id, Some(result.task.id));
    assert_eq!(
        result.parent_reference.target_task_id,
        Some(parent.id),
        "parent ref must point at the originating task"
    );

    let re_promote = svc
        .promote_checklist_item(&ctx, parent.id, item.id, proj.id, board.id, col.id)
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

/// A move into an exhausted slot (two distinct neighbors sharing one key) must
/// SUCCEED after the resequence re-derives live anchors — not return 409.
#[tokio::test]
async fn task_service_move_task_recovers_after_resequence() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-reseq-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-reseq", "RQ").await;
    let task_repo = PgTaskRepo::new(db.conn().clone());
    let svc = make_svc(&db);

    let mover = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Mover".into(),
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
        .expect("create mover");

    let left = task_repo
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Left".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: Some(mover.position_key.clone()),
                    after: None,
                },
            },
        )
        .await
        .expect("create left");

    let right = task_repo
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Right".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: Some(left.position_key.clone()),
                    after: None,
                },
            },
        )
        .await
        .expect("create right");

    // Collapse Left and Right onto one shared key: the exhausted slot.
    let collision = left.position_key.clone();
    force_position_key(&db, "tasks", right.id.0, &collision).await;

    // Move the mover "between" the two colliding neighbors. The first try_between
    // sees equal anchors (None); the resequence splits Left/Right and re-derives
    // the anchors so the retry lands the mover strictly between them.
    let moved = svc
        .move_task(
            &ctx,
            mover.id,
            col.id,
            PositionBetween {
                before: Some(collision.clone()),
                after: Some(collision.clone()),
            },
        )
        .await
        .expect("move must succeed after resequence");

    let left_key = read_position_key(&db, "tasks", left.id.0).await;
    let right_key = read_position_key(&db, "tasks", right.id.0).await;
    assert!(
        left_key < moved.position_key && moved.position_key < right_key,
        "mover ({}) must land strictly between left ({left_key}) and right ({right_key})",
        moved.position_key
    );

    db.teardown().await;
}

/// Genuinely unplaceable anchors (inverted order) must still surface
/// PositionExhausted after the resequence cannot create room.
#[tokio::test]
async fn task_service_move_task_inverted_anchors_returns_exhausted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-inv-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-inv", "IV").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Task".into(),
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

    // before > after: no real neighbors back these keys, so the resequence cannot
    // help and the result is a clean PositionExhausted.
    let result = svc
        .move_task(
            &ctx,
            task.id,
            col.id,
            PositionBetween {
                before: Some("ZZZZ".into()),
                after: Some("AAAA".into()),
            },
        )
        .await;

    assert!(
        matches!(
            result,
            Err(atlas_domain::DomainError::PositionExhausted { .. })
        ),
        "inverted anchors must return PositionExhausted, got: {result:?}"
    );

    db.teardown().await;
}

/// Creating a task into an exhausted slot (two distinct neighbors sharing one
/// key) must SUCCEED after the resequence, landing strictly between them.
#[tokio::test]
async fn task_create_between_recovers_after_resequence() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-create-reseq").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "create-reseq", "CR").await;
    let svc = make_svc(&db);

    let anchor_a = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Anchor A".into(),
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
        .expect("create anchor A");

    let anchor_b = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Anchor B".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: Some(anchor_a.position_key.clone()),
                    after: None,
                },
            },
        )
        .await
        .expect("create anchor B");

    // Collapse A and B onto one shared key: the exhausted slot.
    let collision = anchor_a.position_key.clone();
    force_position_key(&db, "tasks", anchor_b.id.0, &collision).await;

    let created = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Inserted".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: Some(collision.clone()),
                    after: Some(collision.clone()),
                },
            },
        )
        .await
        .expect("create must succeed after resequence");

    let a_key = read_position_key(&db, "tasks", anchor_a.id.0).await;
    let b_key = read_position_key(&db, "tasks", anchor_b.id.0).await;
    assert!(
        a_key < created.position_key && created.position_key < b_key,
        "inserted ({}) must land strictly between A ({a_key}) and B ({b_key})",
        created.position_key
    );

    db.teardown().await;
}

/// Adding a checklist item into an exhausted slot must SUCCEED after resequence.
#[tokio::test]
async fn checklist_add_between_recovers_after_resequence() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "cl-add-reseq").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "cl-add-reseq", "CL").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Host task".into(),
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
        .expect("create host task");

    let item_a = svc
        .add_checklist_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: task.id,
                title: "Anchor A".into(),
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("add item A");

    let item_b = svc
        .add_checklist_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: task.id,
                title: "Anchor B".into(),
                position: PositionBetween {
                    before: Some(item_a.position_key.clone()),
                    after: None,
                },
            },
        )
        .await
        .expect("add item B");

    let collision = item_a.position_key.clone();
    force_position_key(&db, "task_checklist_items", item_b.id.0, &collision).await;

    let inserted = svc
        .add_checklist_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: task.id,
                title: "Inserted".into(),
                position: PositionBetween {
                    before: Some(collision.clone()),
                    after: Some(collision.clone()),
                },
            },
        )
        .await
        .expect("add must succeed after resequence");

    let a_key = read_position_key(&db, "task_checklist_items", item_a.id.0).await;
    let b_key = read_position_key(&db, "task_checklist_items", item_b.id.0).await;
    assert!(
        a_key < inserted.position_key && inserted.position_key < b_key,
        "inserted item ({}) must land strictly between A ({a_key}) and B ({b_key})",
        inserted.position_key
    );

    db.teardown().await;
}

/// Repositioning a checklist item into an exhausted slot must SUCCEED after
/// resequence.
#[tokio::test]
async fn checklist_patch_position_recovers_after_resequence() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "cl-patch-reseq").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "cl-patch-reseq", "CP").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Host task".into(),
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
        .expect("create host task");

    let item_a = svc
        .add_checklist_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: task.id,
                title: "Anchor A".into(),
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("add item A");

    let item_b = svc
        .add_checklist_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: task.id,
                title: "Anchor B".into(),
                position: PositionBetween {
                    before: Some(item_a.position_key.clone()),
                    after: None,
                },
            },
        )
        .await
        .expect("add item B");

    // The item to be repositioned via patch.
    let item_c = svc
        .add_checklist_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: task.id,
                title: "Mover".into(),
                position: PositionBetween {
                    before: Some(item_b.position_key.clone()),
                    after: None,
                },
            },
        )
        .await
        .expect("add item C");

    // Collapse A and B onto one shared key: the exhausted slot.
    let collision = item_a.position_key.clone();
    force_position_key(&db, "task_checklist_items", item_b.id.0, &collision).await;

    svc.patch_checklist_item(
        &ctx,
        task.id,
        item_c.id,
        atlas_domain::entities::boards_tasks::TaskChecklistItemPatch {
            title: None,
            checked: None,
            position: Some(PositionBetween {
                before: Some(collision.clone()),
                after: Some(collision.clone()),
            }),
        },
    )
    .await
    .expect("patch must succeed after resequence");

    let a_key = read_position_key(&db, "task_checklist_items", item_a.id.0).await;
    let b_key = read_position_key(&db, "task_checklist_items", item_b.id.0).await;
    let c_key = read_position_key(&db, "task_checklist_items", item_c.id.0).await;
    assert!(
        a_key < c_key && c_key < b_key,
        "repositioned item ({c_key}) must land strictly between A ({a_key}) and B ({b_key})"
    );

    db.teardown().await;
}
