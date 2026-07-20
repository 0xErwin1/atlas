#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    entities::boards_tasks::{
        ActivityKind, ActivityPayload, AssigneeRef, NewBoard, NewTask, NewTaskChecklistItem,
        NewTaskReference, PositionBetween, Priority, ReferenceKind, TaskPatch,
    },
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    persistence::repos::{
        BoardRepo, PgBoardRepo, PgProjectRepo, PgTaskActivityRepo, PgTaskChecklistRepo,
        PgTaskReferenceRepo, PgTaskRepo, ProjectRepo, TaskActivityRepo, TaskChecklistRepo,
        TaskReferenceRepo, TaskRepo,
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
    assert!(
        !result.checklist_item.checked,
        "promotion moves the item to a task; it must not mark it completed"
    );
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
            None,
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

    // Move the mover "between" the two colliding neighbors. Anchors are task ids;
    // both Left and Right now resolve to the same collided key, so the first
    // try_between sees equal anchors, the resequence splits Left/Right, and the
    // retry lands the mover strictly between them.
    let moved = svc
        .move_task(
            &ctx,
            mover.id,
            col.id,
            PositionBetween {
                before: Some(left.id.to_string()),
                after: Some(right.id.to_string()),
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

    // before > after with phantom keys that no real neighbour backs: the
    // resequence cannot help and the result is a clean PositionExhausted. Raw
    // fractional keys are the repo-layer contract — the service layer takes task
    // ids — so this exercises PgTaskRepo::move_to_in directly.
    let result = PgTaskRepo::move_to_in(
        db.conn(),
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

/// Two concurrent promote_checklist_item calls on the same item must produce
/// exactly one success and one Forbidden error. Without a FOR UPDATE lock on
/// the guard-read both transactions see promoted_task_id = NULL, pass the
/// is_some() guard, and each create a child task — leaving two orphan children.
/// With the lock the second transaction blocks until the first commits, then
/// sees promoted_task_id = Some and is rejected before creating any child.
///
/// Approach: deterministic two-transaction test. Txn1 opens a raw FOR UPDATE
/// lock on the checklist item row and sets promoted_task_id to a sentinel UUID,
/// simulating a first promote that has acquired the lock but not yet committed.
/// Meanwhile txn2 (the promote_checklist_item service call) tries to acquire the
/// same row lock and blocks at the Postgres level. Txn1 commits; txn2 unblocks,
/// re-reads the row, finds promoted_task_id = Some, and returns Forbidden.
/// This deterministically replays the concurrency window without relying on
/// scheduler timing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn promote_checklist_item_concurrent_double_promote_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-concurrent-promote").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-conc-promo", "CP2").await;

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

    let checklist_repo =
        atlas_server::persistence::repos::PgTaskChecklistRepo::new(db.conn().clone());
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

    // Create a dummy task to use as the promoted_task_id sentinel; the FK on
    // task_checklist_items.promoted_task_id requires a real tasks row.
    let dummy_child = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Dummy child (sentinel)".into(),
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
        .expect("create dummy child task");

    // Deterministic concurrency proof:
    //
    // 1. Txn1 (raw SQL) acquires FOR UPDATE on the checklist item row and
    //    writes promoted_task_id = dummy_child.id, simulating the tail of a
    //    first promote transaction that has locked and updated the row but has
    //    not yet committed.
    // 2. Txn2 (the real service call) begins and tries to read the same row.
    //    - Without lock_exclusive(): reads the stale promoted_task_id = NULL
    //      (txn1 has not committed), passes the is_some() guard, and proceeds
    //      to create a second child — the double-promote bug.
    //    - With lock_exclusive(): blocks on the row lock until txn1 commits,
    //      re-reads promoted_task_id = Some(dummy_child.id), and returns
    //      Forbidden before creating any child.
    // 3. The commit channel controls when txn1 releases its lock, so the
    //    ordering is fully deterministic.
    let (lock_acquired_tx, lock_acquired_rx) = tokio::sync::oneshot::channel::<()>();
    let (commit_tx, commit_rx) = tokio::sync::oneshot::channel::<()>();

    let dummy_child_id = dummy_child.id.0;
    let item_id_raw = item.id.0;
    let ws_id_raw = ctx.workspace_id.0;
    let db_conn_raw = db.conn().clone();

    let locker = tokio::spawn(async move {
        use sea_orm::{ConnectionTrait, Statement, TransactionTrait};

        let txn = db_conn_raw.begin().await.expect("begin txn1");

        txn.execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT id FROM task_checklist_items \
                 WHERE id = '{item_id_raw}' AND workspace_id = '{ws_id_raw}' \
                 FOR UPDATE"
            ),
        ))
        .await
        .expect("txn1 FOR UPDATE");

        txn.execute_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "UPDATE task_checklist_items \
                 SET promoted_task_id = '{dummy_child_id}' \
                 WHERE id = '{item_id_raw}'"
            ),
        ))
        .await
        .expect("txn1 set promoted_task_id");

        lock_acquired_tx.send(()).expect("signal lock acquired");

        commit_rx.await.expect("wait for commit signal");

        txn.commit().await.expect("txn1 commit");
    });

    lock_acquired_rx.await.expect("wait for lock acquired");

    let ctx2 = ctx.clone();
    let db_conn2 = db.conn().clone();
    let (parent_id, item_id, proj_id, board_id, col_id) =
        (parent.id, item.id, proj.id, board.id, col.id);

    let promote_handle = tokio::spawn(async move {
        atlas_server::services::TaskService::new(db_conn2)
            .promote_checklist_item(&ctx2, parent_id, item_id, proj_id, board_id, col_id)
            .await
    });

    // Give txn2 a moment to issue its guard-read (and block on the lock if the
    // fix is present, or proceed with stale data if it is not).
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    commit_tx.send(()).expect("signal commit");
    locker.await.expect("locker task joined");

    let result = promote_handle.await.expect("promote task joined");

    assert!(
        matches!(result, Err(atlas_domain::DomainError::Forbidden { .. })),
        "promote_checklist_item must return Forbidden when promoted_task_id is set by a concurrent transaction"
    );

    // The parent task must have no inbound Parent references beyond what the
    // locker task wrote — the promote was rejected before creating a child.
    let ref_repo = PgTaskReferenceRepo::new(db.conn().clone());
    let inbound = ref_repo
        .list_inbound(&ctx, parent.id)
        .await
        .expect("list inbound refs");

    assert_eq!(
        inbound.len(),
        0,
        "no inbound Parent reference must exist after a rejected promote; got {}",
        inbound.len()
    );

    db.teardown().await;
}

fn mentioned_titles(entries: &[atlas_domain::entities::boards_tasks::TaskActivity]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|e| match &e.payload {
            ActivityPayload::DocumentMentioned { title, .. } => Some(title.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn task_service_create_with_wikilink_emits_document_mentioned_activity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-mention-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-mention", "MN").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "With link".into(),
                description: "See [[Design Doc]] for details".into(),
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

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list activity");

    assert!(
        entries.iter().any(|e| e.kind == ActivityKind::Created),
        "create must still emit Created"
    );
    assert_eq!(
        mentioned_titles(&entries),
        vec!["Design Doc".to_string()],
        "a wikilink in the initial description must emit one DocumentMentioned"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_service_patch_emits_document_mentioned_only_for_new_wikilinks() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "svc-mention2-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_col(&db, &ctx, "svc-mention2", "MO").await;
    let svc = make_svc(&db);

    let task = svc
        .create(
            &ctx,
            NewTask {
                project_id: proj.id,
                board_id: board.id,
                column_id: col.id,
                title: "Evolving".into(),
                description: "Start with [[Doc A]]".into(),
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

    // Adds [[Doc B]] while keeping [[Doc A]]: only Doc B is new.
    svc.patch(
        &ctx,
        task.id,
        TaskPatch {
            description: Some("Start with [[Doc A]] and now [[Doc B]]".into()),
            ..Default::default()
        },
    )
    .await
    .expect("patch adds a wikilink");

    // Edits prose without touching the wikilinks: nothing new.
    svc.patch(
        &ctx,
        task.id,
        TaskPatch {
            description: Some("Reworded, still [[Doc A]] and [[Doc B]]".into()),
            ..Default::default()
        },
    )
    .await
    .expect("patch without new wikilink");

    let activity_repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = activity_repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list activity");

    let mut titles = mentioned_titles(&entries);
    titles.sort();

    assert_eq!(
        titles,
        vec!["Doc A".to_string(), "Doc B".to_string()],
        "each wikilink must be announced exactly once across its lifetime, never re-emitted"
    );

    db.teardown().await;
}
