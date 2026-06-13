#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    entities::boards_tasks::{
        ActivityKind, ActivityPayload, AssigneeRef, NewBoard, NewTask, NewTaskActivity,
        NewTaskAssignee, NewTaskChecklistItem, PositionBetween,
    },
    entities::workspace_core::NewProject,
    ids::ColumnId,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    BoardRepo, PgBoardRepo, PgProjectRepo, PgTaskActivityRepo, PgTaskAssigneeRepo,
    PgTaskChecklistRepo, PgTaskRepo, ProjectRepo, TaskActivityRepo, TaskAssigneeRepo,
    TaskChecklistRepo, TaskRepo,
};
use sea_orm::TransactionTrait;

async fn seed_project_board_column(
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

async fn seed_task(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    project_id: atlas_domain::ids::ProjectId,
    board_id: atlas_domain::ids::BoardId,
    col_id: ColumnId,
    title: &str,
) -> atlas_domain::entities::boards_tasks::Task {
    PgTaskRepo::new(db.conn().clone())
        .create(
            ctx,
            NewTask {
                project_id,
                board_id,
                column_id: col_id,
                title: title.into(),
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
        .expect("seed task")
}

// T12: PgTaskAssigneeRepo tests

#[tokio::test]
async fn task_assignee_add_and_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "assignee-add-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "assignee-proj", "AS").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgTaskAssigneeRepo::new(db.conn().clone());

    let assignee = repo
        .add(
            &ctx,
            NewTaskAssignee {
                task_id: task.id,
                assignee: AssigneeRef::User(user.id),
            },
        )
        .await
        .expect("add assignee");

    assert_eq!(assignee.task_id, task.id);
    assert_eq!(assignee.assignee, AssigneeRef::User(user.id));

    let list = repo.list_for_task(&ctx, task.id).await.expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].assignee, AssigneeRef::User(user.id));

    db.teardown().await;
}

#[tokio::test]
async fn task_assignee_remove_works() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "assignee-rm-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "assignee-rm-proj", "AR").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgTaskAssigneeRepo::new(db.conn().clone());

    repo.add(
        &ctx,
        NewTaskAssignee {
            task_id: task.id,
            assignee: AssigneeRef::User(user.id),
        },
    )
    .await
    .expect("add");

    repo.remove(&ctx, task.id, AssigneeRef::User(user.id))
        .await
        .expect("remove");

    let list = repo.list_for_task(&ctx, task.id).await.expect("list");
    assert!(list.is_empty(), "assignee must be removed");

    db.teardown().await;
}

// T12: PgTaskChecklistRepo tests

#[tokio::test]
async fn checklist_add_list_patch_delete() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "checklist-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "checklist-proj", "CL").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgTaskChecklistRepo::new(db.conn().clone());

    let item = repo
        .add_item(
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

    assert_eq!(item.title, "Step 1");
    assert!(!item.checked);
    assert!(item.promoted_task_id.is_none());

    let list = repo.list_for_task(&ctx, task.id).await.expect("list");
    assert_eq!(list.len(), 1);

    let patched = repo
        .patch_item(
            &ctx,
            item.id,
            atlas_domain::entities::boards_tasks::TaskChecklistItemPatch {
                title: Some("Step 1 updated".into()),
                checked: Some(true),
                position: None,
            },
        )
        .await
        .expect("patch");

    assert_eq!(patched.title, "Step 1 updated");
    assert!(patched.checked);

    repo.soft_delete_item(&ctx, item.id)
        .await
        .expect("soft delete");

    let list_after = repo
        .list_for_task(&ctx, task.id)
        .await
        .expect("list after delete");
    assert!(list_after.is_empty(), "deleted item must not appear");

    db.teardown().await;
}

#[tokio::test]
async fn checklist_mark_promoted_idempotency_guard() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "promote-guard-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "promote-guard-proj", "PG").await;
    let task_a = seed_task(&db, &ctx, proj.id, board.id, col.id, "Parent").await;
    let task_b = seed_task(&db, &ctx, proj.id, board.id, col.id, "Child").await;

    let repo = PgTaskChecklistRepo::new(db.conn().clone());

    let item = repo
        .add_item(
            &ctx,
            NewTaskChecklistItem {
                task_id: task_a.id,
                title: "To promote".into(),
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("add item");

    repo.mark_promoted(&ctx, item.id, task_b.id)
        .await
        .expect("first mark_promoted must succeed");

    let result = repo.mark_promoted(&ctx, item.id, task_b.id).await;
    assert!(result.is_err(), "re-promoting must fail");

    db.teardown().await;
}

// T13: PgTaskActivityRepo tests

#[tokio::test]
async fn task_activity_append_in_and_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "activity-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "activity-proj", "AC").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let txn = db.conn().begin().await.expect("begin txn");

    PgTaskActivityRepo::append_in(
        &txn,
        &ctx,
        NewTaskActivity {
            task_id: task.id,
            kind: ActivityKind::Created,
            payload: ActivityPayload::Created,
        },
    )
    .await
    .expect("append_in");

    txn.commit().await.expect("commit");

    let repo = PgTaskActivityRepo::new(db.conn().clone());
    let entries = repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, ActivityKind::Created);

    db.teardown().await;
}

#[tokio::test]
async fn task_activity_list_is_newest_first() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "activity-order-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) =
        seed_project_board_column(&db, &ctx, "activity-order-proj", "AO").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let col_a = atlas_domain::ids::ColumnId::new();
    let col_b = atlas_domain::ids::ColumnId::new();

    let repo = PgTaskActivityRepo::new(db.conn().clone());

    repo.append(
        &ctx,
        NewTaskActivity {
            task_id: task.id,
            kind: ActivityKind::Created,
            payload: ActivityPayload::Created,
        },
    )
    .await
    .expect("append created");

    repo.append(
        &ctx,
        NewTaskActivity {
            task_id: task.id,
            kind: ActivityKind::Moved,
            payload: ActivityPayload::Moved {
                from_column_id: col_a,
                to_column_id: col_b,
            },
        },
    )
    .await
    .expect("append moved");

    let entries = repo
        .list_for_task(&ctx, task.id, None, 50)
        .await
        .expect("list");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].kind, ActivityKind::Moved, "newest first: moved");
    assert_eq!(entries[1].kind, ActivityKind::Created, "second: created");

    db.teardown().await;
}
