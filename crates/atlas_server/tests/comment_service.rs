#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    Actor, DomainError,
    entities::boards_tasks::{NewBoard, NewTask, PositionBetween},
    entities::identity::MemberRole,
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    persistence::repos::{
        BoardRepo, MembershipRepo, NewUser, PgBoardRepo, PgMembershipRepo, PgProjectRepo,
        PgTaskRepo, ProjectRepo, TaskRepo, UserRepo,
    },
    services::TaskService,
};

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

async fn seed_task(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    project_id: atlas_domain::ids::ProjectId,
    board_id: atlas_domain::ids::BoardId,
    col_id: atlas_domain::ids::ColumnId,
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

/// Adds `username` as a member of `ws` with `role`, returning a `WorkspaceCtx`
/// acting as that member.
async fn seed_member(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    username: &str,
    role: MemberRole,
) -> atlas_domain::WorkspaceCtx {
    let user_repo = db.user_repo();
    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };

    let user = user_repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed member user");

    support::activate_user_in_db(db, user.id.0).await;

    let ctx = atlas_domain::WorkspaceCtx::new(ws.id, Actor::User(user.id));
    membership_repo
        .add(&ctx, user.id, role)
        .await
        .expect("seed membership");

    ctx
}

#[tokio::test]
async fn add_comment_then_list_returns_it_oldest_first() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "cs-add-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "cs-add-proj", "CA").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let service = TaskService::new(db.conn().clone());

    let first = service
        .add_comment(&ctx, task.id, "first".into())
        .await
        .expect("add first comment");
    let second = service
        .add_comment(&ctx, task.id, "second".into())
        .await
        .expect("add second comment");

    assert_eq!(first.body, "first");
    assert_eq!(first.task_id, Some(task.id));
    assert_eq!(first.created_by, Actor::User(user.id));

    let page = service
        .list_comments(&ctx, task.id, None, 50)
        .await
        .expect("list comments");

    assert_eq!(page.len(), 2);
    assert_eq!(page[0].id, first.id, "oldest first");
    assert_eq!(page[1].id, second.id);

    db.teardown().await;
}

#[tokio::test]
async fn list_comments_on_task_with_no_comments_is_empty() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "cs-empty-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "cs-empty-proj", "CE").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let service = TaskService::new(db.conn().clone());

    let page = service
        .list_comments(&ctx, task.id, None, 50)
        .await
        .expect("list comments");

    assert!(page.is_empty());

    db.teardown().await;
}

#[tokio::test]
async fn author_can_remove_own_comment_without_moderation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "cs-author-owner").await;
    let owner_ctx = support::ctx(&ws, &owner);

    let (proj, board, col) =
        seed_project_board_column(&db, &owner_ctx, "cs-author-proj", "CR").await;
    let task = seed_task(&db, &owner_ctx, proj.id, board.id, col.id, "Task").await;

    let author_ctx = seed_member(&db, &ws, "cs-author-member", MemberRole::Member).await;

    let service = TaskService::new(db.conn().clone());

    let comment = service
        .add_comment(&author_ctx, task.id, "mine".into())
        .await
        .expect("add comment as member");

    service
        .remove_comment(&author_ctx, task.id, comment.id, false)
        .await
        .expect("author deletes own comment");

    let page = service
        .list_comments(&owner_ctx, task.id, None, 50)
        .await
        .expect("list comments");
    assert!(page.is_empty(), "deleted comment must not be listed");

    db.teardown().await;
}

#[tokio::test]
async fn admin_can_remove_another_members_comment_via_can_moderate() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "cs-admin-owner").await;
    let owner_ctx = support::ctx(&ws, &owner);

    let (proj, board, col) = seed_project_board_column(&db, &owner_ctx, "cs-admin-proj", "CM").await;
    let task = seed_task(&db, &owner_ctx, proj.id, board.id, col.id, "Task").await;

    let author_ctx = seed_member(&db, &ws, "cs-admin-author", MemberRole::Member).await;
    let admin_ctx = seed_member(&db, &ws, "cs-admin-admin", MemberRole::Admin).await;

    let service = TaskService::new(db.conn().clone());

    let comment = service
        .add_comment(&author_ctx, task.id, "someone else's comment".into())
        .await
        .expect("add comment as author");

    service
        .remove_comment(&admin_ctx, task.id, comment.id, true)
        .await
        .expect("admin deletes another member's comment");

    let page = service
        .list_comments(&owner_ctx, task.id, None, 50)
        .await
        .expect("list comments");
    assert!(page.is_empty(), "deleted comment must not be listed");

    db.teardown().await;
}

#[tokio::test]
async fn non_author_non_moderator_cannot_remove_comment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "cs-forbid-owner").await;
    let owner_ctx = support::ctx(&ws, &owner);

    let (proj, board, col) =
        seed_project_board_column(&db, &owner_ctx, "cs-forbid-proj", "CF").await;
    let task = seed_task(&db, &owner_ctx, proj.id, board.id, col.id, "Task").await;

    let author_ctx = seed_member(&db, &ws, "cs-forbid-author", MemberRole::Member).await;
    let other_ctx = seed_member(&db, &ws, "cs-forbid-other", MemberRole::Member).await;

    let service = TaskService::new(db.conn().clone());

    let comment = service
        .add_comment(&author_ctx, task.id, "not yours".into())
        .await
        .expect("add comment as author");

    let err = service
        .remove_comment(&other_ctx, task.id, comment.id, false)
        .await
        .expect_err("non-author non-moderator must be forbidden");
    assert!(matches!(err, DomainError::Forbidden { .. }));

    let page = service
        .list_comments(&owner_ctx, task.id, None, 50)
        .await
        .expect("list comments");
    assert_eq!(page.len(), 1, "comment must remain after forbidden delete");

    db.teardown().await;
}

#[tokio::test]
async fn remove_missing_comment_returns_not_found() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "cs-missing-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "cs-missing-proj", "CX").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let service = TaskService::new(db.conn().clone());
    let bogus_id = atlas_domain::ids::CommentId::new();

    let err = service
        .remove_comment(&ctx, task.id, bogus_id, true)
        .await
        .expect_err("removing a nonexistent comment must fail");
    assert!(matches!(err, DomainError::NotFound { .. }));

    db.teardown().await;
}
