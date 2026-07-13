#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    Actor, AttachmentStore, DomainError,
    entities::boards_tasks::{NewBoard, NewTask, PositionBetween},
    entities::comments::{CommentOwner, NewComment},
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    AttachmentWriteIntentRepo, BoardRepo, CommentRepo, DiskAttachmentStore, PgAttachmentLifecycle,
    PgAttachmentWriteIntentRepo, PgBoardRepo, PgCommentRepo, PgProjectRepo, PgTaskRepo,
    ProjectRepo, TaskRepo,
};
use chrono::{Duration, Utc};
use sea_orm::{ConnectionTrait, Statement};
use tempfile::TempDir;

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

#[tokio::test]
async fn comment_create_and_get_roundtrips_body_and_author() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-crud-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "comment-proj", "CM").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgCommentRepo::new(db.conn().clone());

    let created = repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "First comment".into(),
            },
        )
        .await
        .expect("create comment");

    assert_eq!(created.body, "First comment");
    assert_eq!(created.task_id, Some(task.id));
    assert!(created.document_id.is_none());
    assert_eq!(created.created_by, Actor::User(user.id));
    assert!(created.deleted_at.is_none());

    let fetched = repo
        .get_for_owner(&ctx, CommentOwner::Task(task.id), created.id)
        .await
        .expect("get comment");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.body, "First comment");

    db.teardown().await;
}

#[tokio::test]
async fn comment_list_is_oldest_first() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-order-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "comment-order-proj", "CO").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgCommentRepo::new(db.conn().clone());

    for body in ["first", "second", "third"] {
        repo.create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: body.into(),
            },
        )
        .await
        .expect("create comment");
    }

    let page = repo
        .list_for_owner(&ctx, CommentOwner::Task(task.id), None, 50)
        .await
        .expect("list comments");

    assert_eq!(page.len(), 3);
    assert_eq!(page[0].body, "first", "oldest first: first");
    assert_eq!(page[1].body, "second");
    assert_eq!(page[2].body, "third", "newest last");

    db.teardown().await;
}

#[tokio::test]
async fn comment_list_empty_state_returns_no_error() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-empty-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "comment-empty-proj", "CE").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgCommentRepo::new(db.conn().clone());

    let page = repo
        .list_for_owner(&ctx, CommentOwner::Task(task.id), None, 50)
        .await
        .expect("list comments");

    assert!(page.is_empty());

    db.teardown().await;
}

#[tokio::test]
async fn comment_cursor_pagination_has_no_gaps_or_duplicates() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-cursor-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) =
        seed_project_board_column(&db, &ctx, "comment-cursor-proj", "CU").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgCommentRepo::new(db.conn().clone());

    for i in 0..5 {
        repo.create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: format!("comment {i}"),
            },
        )
        .await
        .expect("create comment");
    }

    let mut collected = Vec::new();
    let mut cursor = None;

    loop {
        let page = repo
            .list_for_owner(&ctx, CommentOwner::Task(task.id), cursor, 2)
            .await
            .expect("list comments page");

        if page.is_empty() {
            break;
        }

        cursor = Some(page.last().expect("non-empty page").id);
        collected.extend(page);

        if collected.len() >= 5 {
            break;
        }
    }

    assert_eq!(collected.len(), 5, "must walk the full set with no gaps");

    let bodies: Vec<_> = collected.iter().map(|c| c.body.clone()).collect();
    assert_eq!(
        bodies,
        vec![
            "comment 0",
            "comment 1",
            "comment 2",
            "comment 3",
            "comment 4",
        ],
        "cursor walk must preserve oldest-first order with no duplicates"
    );

    db.teardown().await;
}

#[tokio::test]
async fn comment_soft_delete_removes_from_listing_and_get() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-delete-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) =
        seed_project_board_column(&db, &ctx, "comment-delete-proj", "CD").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgCommentRepo::new(db.conn().clone());

    let created = repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "to be deleted".into(),
            },
        )
        .await
        .expect("create comment");

    repo.soft_delete(&ctx, CommentOwner::Task(task.id), created.id)
        .await
        .expect("soft delete comment");

    let page = repo
        .list_for_owner(&ctx, CommentOwner::Task(task.id), None, 50)
        .await
        .expect("list comments");
    assert!(
        page.is_empty(),
        "deleted comment must not appear in listings"
    );

    let err = repo
        .get_for_owner(&ctx, CommentOwner::Task(task.id), created.id)
        .await
        .expect_err("deleted comment must not be fetchable");
    assert!(matches!(err, DomainError::NotFound { .. }));

    db.teardown().await;
}

#[tokio::test]
async fn comment_delete_missing_comment_returns_not_found() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-missing-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) =
        seed_project_board_column(&db, &ctx, "comment-missing-proj", "CX").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;

    let repo = PgCommentRepo::new(db.conn().clone());
    let bogus_id = atlas_domain::ids::CommentId::new();

    let err = repo
        .soft_delete(&ctx, CommentOwner::Task(task.id), bogus_id)
        .await
        .expect_err("deleting a nonexistent comment must fail");
    assert!(matches!(err, DomainError::NotFound { .. }));

    db.teardown().await;
}

#[tokio::test]
async fn comment_cross_task_id_is_not_found() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-idor-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) = seed_project_board_column(&db, &ctx, "comment-idor-proj", "CI").await;
    let task_a = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task A").await;
    let task_b = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task B").await;

    let repo = PgCommentRepo::new(db.conn().clone());

    let created = repo
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task_a.id),
                body: "belongs to task A".into(),
            },
        )
        .await
        .expect("create comment");

    let err = repo
        .get_for_owner(&ctx, CommentOwner::Task(task_b.id), created.id)
        .await
        .expect_err("comment scoped to task A must not resolve under task B");
    assert!(matches!(err, DomainError::NotFound { .. }));

    let err = repo
        .soft_delete(&ctx, CommentOwner::Task(task_b.id), created.id)
        .await
        .expect_err("deleting under the wrong owner must not succeed");
    assert!(matches!(err, DomainError::NotFound { .. }));

    db.teardown().await;
}

#[tokio::test]
async fn comment_freedom_schema_enforces_owner_and_link_constraints() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-freedom-schema-user").await;
    let ctx = support::ctx(&ws, &user);

    let (proj, board, col) =
        seed_project_board_column(&db, &ctx, "comment-freedom-schema-proj", "CF").await;
    let task = seed_task(&db, &ctx, proj.id, board.id, col.id, "Task").await;
    let comment = PgCommentRepo::new(db.conn().clone())
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "comment".into(),
            },
        )
        .await
        .expect("create comment");

    let invalid_attachment = db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachments (id, workspace_id, task_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id, created_at, updated_at) \
             VALUES ('{}', '{}', '{}', '{}', 'invalid.txt', 'text/plain', 1, '{}', '{}', now(), now())",
            uuid::Uuid::now_v7(),
            ws.id.0,
            task.id.0,
            comment.id.0,
            "a".repeat(64),
            user.id.0,
        ))
        .await;
    assert!(
        invalid_attachment.is_err(),
        "attachment owner XOR must reject task plus comment"
    );

    let link_id = uuid::Uuid::now_v7();
    let insert_link = Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO comment_links (id, workspace_id, comment_id, target_task_id, created_at) VALUES ($1, $2, $3, $4, now())",
        [
            link_id.into(),
            ws.id.0.into(),
            comment.id.0.into(),
            task.id.0.into(),
        ],
    );
    db.conn()
        .execute_raw(insert_link)
        .await
        .expect("insert link");

    let duplicate_statement = Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "INSERT INTO comment_links (id, workspace_id, comment_id, target_task_id, created_at) VALUES ($1, $2, $3, $4, now())",
        [
            uuid::Uuid::now_v7().into(),
            ws.id.0.into(),
            comment.id.0.into(),
            task.id.0.into(),
        ],
    );
    let duplicate = db.conn().execute_raw(duplicate_statement).await;
    assert!(
        duplicate.is_err(),
        "per-target partial uniqueness must reject duplicates"
    );

    db.teardown().await;
}

#[tokio::test]
async fn stale_intent_without_live_attachment_is_reconciled() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let tempdir = TempDir::new().expect("tempdir");
    let store = DiskAttachmentStore::new(tempdir.path())
        .await
        .expect("attachment store");
    let data = b"orphaned after a crash";
    let digest = store.put(data).await.expect("put object");
    let intents = PgAttachmentWriteIntentRepo {
        conn: db.conn().clone(),
    };
    intents
        .create(digest.clone())
        .await
        .expect("commit intent before put");

    PgAttachmentLifecycle::reconcile_stale(
        &db.conn().clone(),
        &store,
        Utc::now() + Duration::seconds(1),
    )
    .await
    .expect("reconcile stale intent");

    assert!(
        !store.exists(&digest).await.expect("object existence"),
        "an unreferenced stale object must be removed"
    );
    assert!(
        intents
            .list_stale(Utc::now() + Duration::seconds(1))
            .await
            .expect("list intents")
            .is_empty(),
        "the completed reconciliation must remove its intent"
    );

    db.teardown().await;
}
