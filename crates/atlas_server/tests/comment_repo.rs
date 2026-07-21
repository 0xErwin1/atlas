#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use async_trait::async_trait;
use atlas_domain::ids::CommentDraftId;
use atlas_domain::ports::comments::CommentLinkRepo;
use atlas_domain::{
    Actor, AttachmentStore, DomainError,
    entities::boards_tasks::{NewBoard, NewTask, PositionBetween},
    entities::comments::{CommentFeedEntry, CommentLinkTarget, CommentOwner, NewComment},
    entities::documents::NewAttachment,
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::entities::comments::comment_attachment_draft;
use atlas_server::persistence::repos::{
    AttachmentWriteIntentRepo, BoardRepo, CommentRepo, DiskAttachmentStore, PgAttachmentLifecycle,
    PgAttachmentRepo, PgAttachmentWriteIntentRepo, PgBoardRepo, PgCommentLinkRepo, PgCommentRepo,
    PgProjectRepo, PgTaskRepo, ProjectRepo, TaskRepo,
};
use atlas_server::services::CommentService;
use chrono::{Duration, Utc};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Statement};
use sha2::Digest;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio::time::timeout;

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

async fn seed_draft(
    db: &support::TestDb,
    workspace_id: uuid::Uuid,
    task_id: uuid::Uuid,
    user_id: uuid::Uuid,
    state: &str,
    expires_at: &str,
    terminal_at: Option<&str>,
) -> CommentDraftId {
    let id = uuid::Uuid::now_v7();
    let terminal_at = terminal_at
        .map(|value| format!("'{value}'::timestamptz"))
        .unwrap_or_else(|| "NULL".into());

    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_drafts \
             (id, workspace_id, task_id, created_by_user_id, create_token, create_digest, state, expires_at, terminal_at) \
             VALUES ('{id}', '{workspace_id}', '{task_id}', '{user_id}', '{id}', '\\x{}', '{state}', '{expires_at}'::timestamptz, {terminal_at})",
            "01".repeat(32),
        ))
        .await
        .expect("seed comment attachment draft");

    CommentDraftId(id)
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
async fn comment_freedom_migration_defines_required_constraints_and_indexes() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let constraints = db
        .conn()
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT pg_get_constraintdef(c.oid) AS definition \
             FROM pg_constraint c \
             JOIN pg_class t ON t.oid = c.conrelid \
             WHERE t.relname IN ('comment_links', 'attachments')",
        ))
        .await
        .expect("read constraint definitions")
        .into_iter()
        .map(|row| {
            row.try_get::<String>("", "definition")
                .expect("constraint definition")
        })
        .collect::<Vec<_>>();

    assert!(
        constraints.iter().any(|definition| {
            definition.contains("CHECK ((num_nonnulls(target_document_id, target_task_id, target_attachment_id) = 1))")
        }),
        "comment links must require exactly one target"
    );
    assert!(
        constraints.iter().any(|definition| {
            definition
                .contains("CHECK ((num_nonnulls(document_id, task_id, comment_id, draft_id) = 1))")
        }),
        "attachments must require exactly one document, task, comment, or draft owner"
    );

    let cascade_references = db
        .conn()
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT source.relname AS source_table, target.relname AS target_table \
             FROM pg_constraint c \
             JOIN pg_class source ON source.oid = c.conrelid \
             JOIN pg_class target ON target.oid = c.confrelid \
             WHERE c.contype = 'f' AND c.confdeltype = 'c' \
               AND source.relname IN ('comment_links', 'attachments')",
        ))
        .await
        .expect("read cascade references")
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "source_table")
                    .expect("source table"),
                row.try_get::<String>("", "target_table")
                    .expect("target table"),
            )
        })
        .collect::<Vec<_>>();

    for target in ["comments", "documents", "tasks", "attachments"] {
        assert!(
            cascade_references
                .iter()
                .any(|(source, reference)| source == "comment_links" && reference == target),
            "comment links must cascade when {target} is deleted"
        );
    }
    assert!(
        cascade_references
            .iter()
            .any(|(source, target)| source == "attachments" && target == "comments"),
        "comment-owned attachments must cascade when the comment is deleted"
    );

    let indexes = db
        .conn()
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT indexname, indexdef FROM pg_indexes \
             WHERE tablename IN ('comment_links', 'attachment_write_intents')",
        ))
        .await
        .expect("read index definitions")
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "indexname").expect("index name"),
                row.try_get::<String>("", "indexdef")
                    .expect("index definition"),
            )
        })
        .collect::<Vec<_>>();

    for index in [
        "comment_links_document_unique",
        "comment_links_task_unique",
        "comment_links_attachment_unique",
    ] {
        assert!(
            indexes.iter().any(|(name, definition)| {
                name == index && definition.contains("UNIQUE") && definition.contains("WHERE")
            }),
            "{index} must be a partial unique index"
        );
    }
    assert!(
        indexes.iter().any(|(name, definition)| {
            name == "attachment_write_intents_digest_idx"
                && definition.contains("UNIQUE")
                && definition.contains("(digest)")
        }),
        "write intents must have a unique digest index"
    );

    db.teardown().await;
}

#[tokio::test]
async fn comment_link_events_retain_parent_owned_history_with_reverse_indexes() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let constraints = db
        .conn()
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT pg_get_constraintdef(c.oid) AS definition \
             FROM pg_constraint c \
             JOIN pg_class t ON t.oid = c.conrelid \
             WHERE t.relname = 'comment_link_events'",
        ))
        .await
        .expect("read event constraint definitions")
        .into_iter()
        .map(|row| {
            row.try_get::<String>("", "definition")
                .expect("event constraint definition")
        })
        .collect::<Vec<_>>();

    assert!(
        constraints.iter().any(|definition| {
            definition.contains("CHECK ((num_nonnulls(parent_task_id, parent_document_id) = 1))")
        }),
        "events must belong to exactly one comment parent"
    );
    assert!(
        constraints.iter().any(|definition| {
            definition.contains("event_kind")
                && definition.contains(
                    "num_nonnulls(target_document_id, target_task_id, target_attachment_id) = 1",
                )
                && definition.contains(
                    "num_nonnulls(target_document_id, target_task_id, target_attachment_id) = 0",
                )
        }),
        "link events must retain one target for link events and none for deletion events"
    );

    let indexes = db
        .conn()
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT indexname FROM pg_indexes \
             WHERE tablename IN ('comment_links', 'comment_link_events', 'attachments')",
        ))
        .await
        .expect("read comment freedom indexes")
        .into_iter()
        .map(|row| row.try_get::<String>("", "indexname").expect("index name"))
        .collect::<Vec<_>>();

    for index in [
        "comment_link_events_task_feed_idx",
        "comment_link_events_document_feed_idx",
        "comment_link_events_document_reverse_idx",
        "comment_link_events_task_reverse_idx",
        "comment_link_events_attachment_reverse_idx",
        "comment_links_document_reverse_idx",
        "comment_links_task_reverse_idx",
        "comment_links_attachment_reverse_idx",
        "attachments_comment_owner_idx",
    ] {
        assert!(indexes.iter().any(|name| name == index), "missing {index}");
    }

    db.teardown().await;
}

#[tokio::test]
async fn comment_link_repo_replaces_live_edges_and_retains_diff_events() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-link-diff-user").await;
    let ctx = support::ctx(&ws, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "comment-link-diff-proj", "CL").await;
    let parent = seed_task(&db, &ctx, project.id, board.id, column.id, "Parent").await;
    let first_target = seed_task(&db, &ctx, project.id, board.id, column.id, "First").await;
    let second_target = seed_task(&db, &ctx, project.id, board.id, column.id, "Second").await;
    let comment = PgCommentRepo::new(db.conn().clone())
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(parent.id),
                body: "derived links".into(),
            },
        )
        .await
        .expect("create comment");
    let links = PgCommentLinkRepo::new(db.conn().clone());

    links
        .replace_for_comment(
            &ctx,
            comment.id,
            vec![CommentLinkTarget::Task(first_target.id)],
        )
        .await
        .expect("add first target");
    links
        .replace_for_comment(
            &ctx,
            comment.id,
            vec![CommentLinkTarget::Task(second_target.id)],
        )
        .await
        .expect("replace target");

    assert!(
        links
            .backlinks_for_target(&ctx, CommentLinkTarget::Task(first_target.id))
            .await
            .expect("first backlink")
            .is_empty()
    );
    assert_eq!(
        links
            .backlinks_for_target(&ctx, CommentLinkTarget::Task(second_target.id))
            .await
            .expect("second backlink")
            .into_iter()
            .map(|link| link.comment_id)
            .collect::<Vec<_>>(),
        vec![comment.id]
    );

    let events = links
        .feed_for_owner(&ctx, CommentOwner::Task(parent.id), None, 20)
        .await
        .expect("parent feed")
        .entries;
    assert_eq!(
        events
            .into_iter()
            .filter_map(|entry| match entry {
                CommentFeedEntry::Event(event) => Some(event.kind),
                CommentFeedEntry::Comment(_) => None,
            })
            .collect::<Vec<_>>(),
        vec![
            atlas_domain::entities::comments::CommentLinkEventKind::LinkAdded,
            atlas_domain::entities::comments::CommentLinkEventKind::LinkRemoved,
            atlas_domain::entities::comments::CommentLinkEventKind::LinkAdded,
        ]
    );

    db.teardown().await;
}

#[tokio::test]
async fn comment_feed_paginates_merged_comments_and_retained_events_without_duplicates() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-feed-page-user").await;
    let ctx = support::ctx(&ws, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "comment-feed-page-proj", "FP").await;
    let parent = seed_task(&db, &ctx, project.id, board.id, column.id, "Parent").await;
    let target = seed_task(&db, &ctx, project.id, board.id, column.id, "Target").await;
    let comments = PgCommentRepo::new(db.conn().clone());
    let links = PgCommentLinkRepo::new(db.conn().clone());

    let first = comments
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(parent.id),
                body: "first".into(),
            },
        )
        .await
        .expect("create first comment");
    links
        .replace_for_comment(&ctx, first.id, vec![CommentLinkTarget::Task(target.id)])
        .await
        .expect("create link-added event");
    let second = comments
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(parent.id),
                body: "second".into(),
            },
        )
        .await
        .expect("create second comment");
    links
        .remove_for_comment(&ctx, first.id)
        .await
        .expect("create link-removed event");
    CommentService::new(db.conn().clone())
        .remove(&ctx, CommentOwner::Task(parent.id), first.id, false)
        .await
        .expect("retain deletion event while soft-deleting first comment");

    let first_page = links
        .feed_for_owner(&ctx, CommentOwner::Task(parent.id), None, 2)
        .await
        .expect("load first merged page");
    assert_eq!(first_page.entries.len(), 2);
    assert!(first_page.has_more, "the n+1 row must set has_more");

    let cursor = first_page
        .entries
        .last()
        .expect("non-empty first page")
        .cursor();
    let second_page = links
        .feed_for_owner(&ctx, CommentOwner::Task(parent.id), Some(cursor), 2)
        .await
        .expect("load second merged page");
    assert_eq!(second_page.entries.len(), 2);
    assert!(!second_page.has_more);
    assert!(matches!(
        second_page.entries.last(),
        Some(CommentFeedEntry::Event(event))
            if event.kind == atlas_domain::entities::comments::CommentLinkEventKind::CommentDeleted
    ));
    assert_eq!(
        second.id,
        first_page
            .entries
            .iter()
            .chain(second_page.entries.iter())
            .find_map(|entry| match entry {
                CommentFeedEntry::Comment(comment) if comment.id == second.id => Some(comment.id),
                _ => None,
            })
            .expect("live second comment appears exactly once"),
    );

    let mut cursors = first_page
        .entries
        .iter()
        .chain(second_page.entries.iter())
        .map(CommentFeedEntry::cursor)
        .collect::<Vec<_>>();
    let original_len = cursors.len();
    cursors.sort();
    cursors.dedup();
    assert_eq!(
        cursors.len(),
        original_len,
        "pages must not duplicate entries"
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

#[tokio::test]
async fn failed_object_delete_keeps_intent_for_a_later_reconciliation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let store = FailOnceDeleteStore::new();
    let data = b"retry object cleanup";
    let digest = format!("{:x}", sha2::Sha256::digest(data));
    store.put(data).await.expect("seed object");

    let intents = PgAttachmentWriteIntentRepo {
        conn: db.conn().clone(),
    };
    intents.create(digest.clone()).await.expect("seed intent");

    PgAttachmentLifecycle::reconcile_stale(
        &db.conn().clone(),
        &store,
        Utc::now() + Duration::seconds(1),
    )
    .await
    .expect("a failed individual cleanup must not abort the sweep");
    assert_eq!(
        intents
            .list_stale(Utc::now() + Duration::seconds(1))
            .await
            .expect("list retained intent")
            .len(),
        1,
        "failed cleanup must keep durable retry intent"
    );

    PgAttachmentLifecycle::reconcile_stale(
        &db.conn().clone(),
        &store,
        Utc::now() + Duration::seconds(1),
    )
    .await
    .expect("retry reconciliation");

    assert!(
        !store.exists(&digest).await.expect("object existence"),
        "a later reconciliation must retry and remove the object"
    );
    assert!(
        intents
            .list_stale(Utc::now() + Duration::seconds(1))
            .await
            .expect("list completed intents")
            .is_empty(),
        "successful retry must remove the intent"
    );

    db.teardown().await;
}

#[tokio::test]
async fn reconciliation_continues_after_an_independent_delete_failure() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let failing_data = b"first cleanup fails";
    let succeeding_data = b"later cleanup succeeds";
    let failing_digest = format!("{:x}", sha2::Sha256::digest(failing_data));
    let succeeding_digest = format!("{:x}", sha2::Sha256::digest(succeeding_data));
    let store = SelectiveFailDeleteStore::new(failing_digest.clone());
    store.put(failing_data).await.expect("seed failing object");
    store
        .put(succeeding_data)
        .await
        .expect("seed succeeding object");

    let intents = PgAttachmentWriteIntentRepo {
        conn: db.conn().clone(),
    };
    intents
        .create(failing_digest.clone())
        .await
        .expect("seed failing intent");
    intents
        .create(succeeding_digest.clone())
        .await
        .expect("seed succeeding intent");

    PgAttachmentLifecycle::reconcile_stale(
        &db.conn().clone(),
        &store,
        Utc::now() + Duration::seconds(1),
    )
    .await
    .expect("one failed intent must not abort the sweep");

    assert!(
        store
            .exists(&failing_digest)
            .await
            .expect("failing object existence"),
        "the failed object remains for retry"
    );
    assert!(
        !store
            .exists(&succeeding_digest)
            .await
            .expect("succeeding object existence"),
        "a later independent intent must reconcile in the same sweep"
    );
    assert_eq!(
        intents
            .list_stale(Utc::now() + Duration::seconds(1))
            .await
            .expect("list retained intents")
            .into_iter()
            .map(|intent| intent.digest)
            .collect::<Vec<_>>(),
        vec![failing_digest],
        "only the failed intent remains retryable"
    );

    db.teardown().await;
}

#[tokio::test]
async fn reconciler_runs_immediately_and_periodically_despite_failures() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let store = AlwaysFailDeleteStore::new();
    let digest = format!("{:x}", sha2::Sha256::digest(b"periodic reconciliation"));
    store
        .put(b"periodic reconciliation")
        .await
        .expect("seed object");
    PgAttachmentWriteIntentRepo {
        conn: db.conn().clone(),
    }
    .create(digest)
    .await
    .expect("seed intent");

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let reconciler = tokio::spawn(PgAttachmentLifecycle::run_reconciler_with_timing(
        db.conn().clone(),
        Arc::new(store.clone()),
        shutdown_rx,
        std::time::Duration::from_millis(10),
        Duration::zero(),
    ));

    store.wait_for_delete_attempts(2).await;
    shutdown_tx.send(true).expect("signal shutdown");
    reconciler.await.expect("reconciler task");

    assert!(
        store.delete_attempts() >= 2,
        "startup-immediate and periodic attempts must both execute"
    );

    db.teardown().await;
}

#[tokio::test]
async fn draft_reconciliation_expires_at_most_one_hundred_and_prunes_only_safe_terminals() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "draft-reconcile-bounds").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "draft-reconcile-bounds", "DR").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let tempdir = TempDir::new().expect("tempdir");
    let store = DiskAttachmentStore::new(tempdir.path())
        .await
        .expect("attachment store");

    for _ in 0..101 {
        seed_draft(
            &db,
            workspace.id.0,
            task.id.0,
            user.id.0,
            "active",
            "2000-01-01T00:00:00Z",
            None,
        )
        .await;
    }
    let prunable = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "cancelled",
        "2000-01-01T00:00:00Z",
        Some("2000-01-01T00:00:00Z"),
    )
    .await;
    let retained = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "expired",
        "2000-01-01T00:00:00Z",
        Some("2000-01-01T00:00:00Z"),
    )
    .await;
    let retained_attachment = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachments \
             (id, workspace_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id, deleted_at) \
             VALUES ('{retained_attachment}', '{}', '{}', 'retained.pdf', 'application/pdf', 1, 'retained-draft-intent', '{}', now())",
            workspace.id.0, retained.0, user.id.0,
        ))
        .await
        .expect("seed retained draft attachment tombstone");
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_draft_uploads \
             (draft_id, upload_token, original_attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes, deleted_at) \
             VALUES ('{}', 'retained-upload', '{retained_attachment}', '\\x{}', '\\x{}', 'retained.pdf', 'application/pdf', 1, now())",
            retained.0,
            "01".repeat(32),
            "02".repeat(32),
        ))
        .await
        .expect("seed retained upload tombstone");
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachment_write_intents (id, digest, created_at) VALUES ('{}', 'retained-draft-intent', now())",
            uuid::Uuid::now_v7(),
        ))
        .await
        .expect("seed retained cleanup intent");

    let report = PgAttachmentLifecycle::reconcile_drafts(&db.conn().clone(), &store)
        .await
        .expect("reconcile draft retention");

    let active = comment_attachment_draft::Entity::find()
        .filter(comment_attachment_draft::Column::State.eq("active"))
        .count(db.conn())
        .await
        .expect("count active drafts");
    let prunable_exists = comment_attachment_draft::Entity::find_by_id(prunable.0)
        .one(db.conn())
        .await
        .expect("lookup prunable draft")
        .is_some();
    let retained_exists = comment_attachment_draft::Entity::find_by_id(retained.0)
        .one(db.conn())
        .await
        .expect("lookup retained draft")
        .is_some();

    assert_eq!(report.claimed_expiries, 100);
    assert_eq!(report.pruned, 1);
    assert_eq!(
        active, 1,
        "the expiry claim must be bounded at one hundred rows"
    );
    assert!(
        !prunable_exists,
        "drained terminal drafts must be pruned after seven days"
    );
    assert!(
        retained_exists,
        "terminal drafts with relevant cleanup intents must be retained"
    );

    db.teardown().await;
}

#[tokio::test]
async fn draft_expiry_keeps_cleanup_intent_after_failure_and_stale_retry_drains_it() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "draft-expiry-cleanup-retry").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "draft-expiry-cleanup-retry", "ER").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let draft = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "active",
        "2000-01-01T00:00:00Z",
        None,
    )
    .await;
    let data = b"expired draft cleanup retry";
    let digest = format!("{:x}", sha2::Sha256::digest(data));
    let attachment_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachments \
             (id, workspace_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
             VALUES ('{attachment_id}', '{}', '{}', 'retry.txt', 'text/plain', {}, '{digest}', '{}'); \
             INSERT INTO comment_attachment_draft_uploads \
             (draft_id, upload_token, original_attachment_id, attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes) \
             VALUES ('{}', 'expiry-retry', '{attachment_id}', '{attachment_id}', '\\x{}', '\\x{}', 'retry.txt', 'text/plain', {})",
            workspace.id.0,
            draft.0,
            data.len(),
            user.id.0,
            draft.0,
            "01".repeat(32),
            "02".repeat(32),
            data.len(),
        ))
        .await
        .expect("seed expired draft attachment and upload");
    let store = FailOnceDeleteStore::new();
    store.put(data).await.expect("seed object");

    let first = PgAttachmentLifecycle::reconcile_drafts(&db.conn().clone(), &store)
        .await
        .expect("expire draft despite cleanup failure");
    assert_eq!(first.claimed_expiries, 1);
    assert_eq!(first.cleanup_failed, 1);
    assert!(
        store
            .exists(&digest)
            .await
            .expect("object remains for retry")
    );
    assert_eq!(
        PgAttachmentWriteIntentRepo {
            conn: db.conn().clone(),
        }
        .list_stale(Utc::now() + Duration::seconds(1))
        .await
        .expect("list retained cleanup intent")
        .len(),
        1
    );

    PgAttachmentLifecycle::reconcile_stale(
        &db.conn().clone(),
        &store,
        Utc::now() + Duration::seconds(1),
    )
    .await
    .expect("retry stale cleanup intent");
    assert!(
        !store
            .exists(&digest)
            .await
            .expect("object removed on retry")
    );
    assert!(
        PgAttachmentWriteIntentRepo {
            conn: db.conn().clone(),
        }
        .list_stale(Utc::now() + Duration::seconds(1))
        .await
        .expect("list drained cleanup intents")
        .is_empty()
    );

    db.teardown().await;
}

#[tokio::test]
async fn terminal_pruning_honors_the_seven_day_boundary_and_live_attachment_guard() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "draft-prune-boundary").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "draft-prune-boundary", "PB").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let not_yet_due = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "cancelled",
        "2000-01-01T00:00:00Z",
        Some("2999-01-01T00:00:00Z"),
    )
    .await;
    let guarded = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "expired",
        "2000-01-01T00:00:00Z",
        Some("2000-01-01T00:00:00Z"),
    )
    .await;
    let attachment_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachments \
             (id, workspace_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
             VALUES ('{attachment_id}', '{}', '{}', 'live.txt', 'text/plain', 1, 'live-prune-guard', '{}')",
            workspace.id.0, guarded.0, user.id.0,
        ))
        .await
        .expect("seed live guarded attachment");

    let tempdir = TempDir::new().expect("tempdir");
    let store = DiskAttachmentStore::new(tempdir.path())
        .await
        .expect("attachment store");
    let report = PgAttachmentLifecycle::reconcile_drafts(&db.conn().clone(), &store)
        .await
        .expect("reconcile terminal drafts");

    assert_eq!(report.pruned, 0);
    assert!(
        comment_attachment_draft::Entity::find_by_id(not_yet_due.0)
            .one(db.conn())
            .await
            .expect("load not-yet-due draft")
            .is_some(),
        "terminal replay metadata must remain before the seven-day boundary"
    );
    assert!(
        comment_attachment_draft::Entity::find_by_id(guarded.0)
            .one(db.conn())
            .await
            .expect("load guarded draft")
            .is_some(),
        "a terminal draft with a live attachment must not prune"
    );

    db.teardown().await;
}

#[tokio::test]
async fn reconciler_shutdown_before_start_claims_no_expired_drafts() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "draft-reconciler-shutdown").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "draft-reconciler-shutdown", "RS").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let draft = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "active",
        "2000-01-01T00:00:00Z",
        None,
    )
    .await;
    let tempdir = TempDir::new().expect("tempdir");
    let store = Arc::new(
        DiskAttachmentStore::new(tempdir.path())
            .await
            .expect("attachment store"),
    );
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(true);
    drop(shutdown_tx);

    PgAttachmentLifecycle::run_reconciler_with_timing(
        db.conn().clone(),
        store,
        shutdown_rx,
        std::time::Duration::from_millis(1),
        Duration::zero(),
    )
    .await;

    let retained = comment_attachment_draft::Entity::find_by_id(draft.0)
        .one(db.conn())
        .await
        .expect("load unclaimed draft")
        .expect("draft remains");
    assert_eq!(retained.state, "active");

    db.teardown().await;
}

#[tokio::test]
async fn reconciler_starts_with_draft_expiry_and_isolates_cleanup_failures_per_item() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "draft-reconciler-isolation").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "draft-reconciler-isolation", "RI").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let first = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "active",
        "2000-01-01T00:00:00Z",
        None,
    )
    .await;
    let second = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "active",
        "2000-01-01T00:00:00Z",
        None,
    )
    .await;
    let failing_data = b"draft cleanup fails";
    let succeeding_data = b"draft cleanup succeeds";
    let failing_digest = format!("{:x}", sha2::Sha256::digest(failing_data));
    let succeeding_digest = format!("{:x}", sha2::Sha256::digest(succeeding_data));
    let store = SelectiveFailDeleteStore::new(failing_digest.clone());
    store.put(failing_data).await.expect("seed failing object");
    store
        .put(succeeding_data)
        .await
        .expect("seed succeeding object");

    for (draft, digest, token) in [
        (first, failing_digest.as_str(), "failing-expiry"),
        (second, succeeding_digest.as_str(), "succeeding-expiry"),
    ] {
        let attachment_id = uuid::Uuid::now_v7();
        db.conn()
            .execute_unprepared(&format!(
                "INSERT INTO attachments \
                 (id, workspace_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
                 VALUES ('{attachment_id}', '{}', '{}', '{token}.txt', 'text/plain', 1, '{digest}', '{}'); \
                 INSERT INTO comment_attachment_draft_uploads \
                 (draft_id, upload_token, original_attachment_id, attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes) \
                 VALUES ('{}', '{token}', '{attachment_id}', '{attachment_id}', '\\x{}', '\\x{}', '{token}.txt', 'text/plain', 1)",
                workspace.id.0,
                draft.0,
                user.id.0,
                draft.0,
                "01".repeat(32),
                "02".repeat(32),
            ))
            .await
            .expect("seed expired draft attachment");
    }

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let reconciler = tokio::spawn(PgAttachmentLifecycle::run_reconciler_with_timing(
        db.conn().clone(),
        Arc::new(store.clone()),
        shutdown_rx,
        std::time::Duration::from_secs(60),
        Duration::zero(),
    ));
    timeout(std::time::Duration::from_secs(5), async {
        loop {
            let expired = comment_attachment_draft::Entity::find()
                .filter(comment_attachment_draft::Column::State.eq("expired"))
                .count(db.conn())
                .await
                .expect("count immediately expired drafts");
            if expired == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("immediate reconciler tick expires both drafts");
    shutdown_tx.send(true).expect("signal shutdown");
    reconciler.await.expect("reconciler task");

    assert!(
        store
            .exists(&failing_digest)
            .await
            .expect("failing object existence"),
        "a failed item remains retryable"
    );
    assert!(
        !store
            .exists(&succeeding_digest)
            .await
            .expect("succeeding object existence"),
        "an independent cleanup failure must not block the later item"
    );

    db.teardown().await;
}

#[tokio::test]
async fn draft_cancellation_cannot_cross_workspace_boundaries() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "draft-cancel-owner").await;
    let owner_ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &owner_ctx, "draft-cancel-owner", "CO").await;
    let task = seed_task(&db, &owner_ctx, project.id, board.id, column.id, "Task").await;
    let draft = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "active",
        "2999-01-01T00:00:00Z",
        None,
    )
    .await;
    let (other_workspace, other_user) = support::seed_workspace(&db, "draft-cancel-other").await;
    let other_ctx = support::ctx(&other_workspace, &other_user);
    let tempdir = TempDir::new().expect("tempdir");
    let store = DiskAttachmentStore::new(tempdir.path())
        .await
        .expect("attachment store");

    let result =
        PgAttachmentLifecycle::cancel_draft(&db.conn().clone(), &other_ctx, draft, &store).await;
    assert!(matches!(result, Err(DomainError::NotFound { .. })));
    assert_eq!(
        comment_attachment_draft::Entity::find_by_id(draft.0)
            .one(db.conn())
            .await
            .expect("load owner draft")
            .expect("draft remains")
            .state,
        "active"
    );

    db.teardown().await;
}

#[tokio::test]
async fn finalized_origin_individual_delete_rolls_back_its_tombstone_on_database_failure() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "finalized-attachment-rollback").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "finalized-attachment-rollback", "FR").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let comment = PgCommentRepo::new(db.conn().clone())
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "finalized draft comment".into(),
            },
        )
        .await
        .expect("create comment");
    let draft = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "finalized",
        "2999-01-01T00:00:00Z",
        None,
    )
    .await;
    let attachment_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE comment_attachment_drafts SET finalized_comment_id = '{}' WHERE id = '{}'; \
             INSERT INTO attachments (id, workspace_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
             VALUES ('{attachment_id}', '{}', '{}', 'rollback.txt', 'text/plain', 1, 'rollback-digest', '{}'); \
             INSERT INTO comment_attachment_draft_uploads \
             (draft_id, upload_token, original_attachment_id, attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes) \
             VALUES ('{}', 'rollback-upload', '{attachment_id}', '{attachment_id}', '\\x{}', '\\x{}', 'rollback.txt', 'text/plain', 1); \
             CREATE FUNCTION fail_finalized_attachment_delete() RETURNS trigger AS $$ \
             BEGIN RAISE EXCEPTION 'injected finalized attachment failure'; END; $$ LANGUAGE plpgsql; \
             CREATE TRIGGER fail_finalized_attachment_delete BEFORE UPDATE OF deleted_at ON attachments \
             FOR EACH ROW WHEN (NEW.deleted_at IS NOT NULL) EXECUTE FUNCTION fail_finalized_attachment_delete()",
            comment.id.0,
            draft.0,
            workspace.id.0,
            comment.id.0,
            user.id.0,
            draft.0,
            "01".repeat(32),
            "02".repeat(32),
        ))
        .await
        .expect("seed finalized origin with failing trigger");
    let result = PgAttachmentLifecycle::delete_comment_attachment(
        &db.conn().clone(),
        &ctx,
        comment.id,
        atlas_domain::AttachmentId(attachment_id),
    )
    .await;
    assert!(matches!(result, Err(DomainError::Internal { .. })));
    let upload = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT attachment_id, deleted_at FROM comment_attachment_draft_uploads WHERE draft_id = $1",
            [draft.0.into()],
        ))
        .await
        .expect("load upload after rollback")
        .expect("upload row");
    assert_eq!(
        upload
            .try_get::<Option<uuid::Uuid>>("", "attachment_id")
            .expect("read live attachment"),
        Some(attachment_id)
    );
    assert!(
        upload
            .try_get::<Option<chrono::DateTime<Utc>>>("", "deleted_at")
            .expect("read upload tombstone")
            .is_none()
    );

    db.teardown().await;
}

#[tokio::test]
async fn direct_attachment_restore_only_clears_the_matching_tombstone() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "attachment-restore-scope").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "attachment-restore-scope", "ARS").await;
    let task = seed_task(
        &db,
        &ctx,
        project.id,
        board.id,
        column.id,
        "Attachment owner",
    )
    .await;
    let attachment_id = atlas_domain::AttachmentId(uuid::Uuid::now_v7());
    let independent_attachment_id = atlas_domain::AttachmentId(uuid::Uuid::now_v7());
    let deleted_at = Utc::now();
    let independently_deleted_at = deleted_at + Duration::microseconds(1);

    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachments \
             (id, workspace_id, task_id, file_name, content_type, size_bytes, sha256, created_by_user_id, deleted_at) \
             VALUES ('{}', '{}', '{}', 'restore.txt', 'text/plain', 1, 'restore', '{}', '{}'), \
                    ('{}', '{}', '{}', 'independent.txt', 'text/plain', 1, 'independent', '{}', '{}')",
            attachment_id.0,
            workspace.id.0,
            task.id.0,
            user.id.0,
            deleted_at.to_rfc3339(),
            independent_attachment_id.0,
            workspace.id.0,
            task.id.0,
            user.id.0,
            independently_deleted_at.to_rfc3339(),
        ))
        .await
        .expect("seed direct attachment tombstones");

    let persisted_deleted_at: chrono::DateTime<Utc> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM attachments WHERE id = $1",
            [attachment_id.0.into()],
        ))
        .await
        .expect("load direct attachment tombstone")
        .expect("direct attachment row")
        .try_get("", "deleted_at")
        .expect("direct attachment tombstone");
    let persisted_independently_deleted_at: chrono::DateTime<Utc> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM attachments WHERE id = $1",
            [independent_attachment_id.0.into()],
        ))
        .await
        .expect("load independent attachment tombstone")
        .expect("independent attachment row")
        .try_get("", "deleted_at")
        .expect("independent attachment tombstone");

    PgAttachmentRepo::restore_at_in(db.conn(), &ctx, attachment_id, persisted_deleted_at)
        .await
        .expect("restore matching direct attachment tombstone");

    let restored_at: Option<chrono::DateTime<Utc>> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM attachments WHERE id = $1",
            [attachment_id.0.into()],
        ))
        .await
        .expect("load restored attachment")
        .expect("restored attachment row")
        .try_get("", "deleted_at")
        .expect("restored attachment tombstone");
    let independent_at: Option<chrono::DateTime<Utc>> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM attachments WHERE id = $1",
            [independent_attachment_id.0.into()],
        ))
        .await
        .expect("load independent attachment")
        .expect("independent attachment row")
        .try_get("", "deleted_at")
        .expect("independent attachment tombstone");

    assert!(restored_at.is_none());
    assert_eq!(independent_at, Some(persisted_independently_deleted_at));

    db.teardown().await;
}

#[tokio::test]
async fn deleting_a_finalized_comment_retains_its_replay_data_and_blob() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "finalized-comment-delete").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "finalized-comment-delete", "FD").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let comment = PgCommentRepo::new(db.conn().clone())
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "finalized draft comment".into(),
            },
        )
        .await
        .expect("create comment");
    let draft_id = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "finalized",
        "2999-01-01T00:00:00Z",
        None,
    )
    .await;
    let attachment_id = uuid::Uuid::now_v7();
    let digest = format!("{:x}", sha2::Sha256::digest(b"finalized attachment"));

    db.conn()
        .execute_unprepared(&format!(
            "UPDATE comment_attachment_drafts SET finalized_comment_id = '{}', final_body_digest = '\\x{}', final_request_digest = '\\x{}' WHERE id = '{}'; \
             INSERT INTO attachments (id, workspace_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
             VALUES ('{attachment_id}', '{}', '{}', 'finalized.txt', 'text/plain', 1, '{digest}', '{}'); \
             INSERT INTO comment_attachment_draft_uploads (draft_id, upload_token, original_attachment_id, attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes) \
             VALUES ('{}', 'finalized-upload', '{attachment_id}', '{attachment_id}', '\\x{}', '\\x{}', 'finalized.txt', 'text/plain', 1)",
            comment.id.0,
            "01".repeat(32),
            "02".repeat(32),
            draft_id.0,
            workspace.id.0,
            comment.id.0,
            user.id.0,
            draft_id.0,
            "03".repeat(32),
            "04".repeat(32),
        ))
        .await
        .expect("seed finalized origin attachment");

    let tempdir = TempDir::new().expect("tempdir");
    let store = Arc::new(
        DiskAttachmentStore::new(tempdir.path())
            .await
            .expect("attachment store"),
    );
    store
        .put(b"finalized attachment")
        .await
        .expect("seed attachment object");
    CommentService::with_attachment_store(db.conn().clone(), store.clone())
        .remove(&ctx, CommentOwner::Task(task.id), comment.id, false)
        .await
        .expect("delete finalized comment");

    let draft = comment_attachment_draft::Entity::find_by_id(draft_id.0)
        .one(db.conn())
        .await
        .expect("load retained draft")
        .expect("draft retained for replay");
    let upload = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT attachment_id, deleted_at FROM comment_attachment_draft_uploads WHERE draft_id = $1",
            [draft_id.0.into()],
        ))
        .await
        .expect("load upload")
        .expect("upload row");
    let attachment_deleted_at: Option<chrono::DateTime<Utc>> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM attachments WHERE id = $1",
            [attachment_id.into()],
        ))
        .await
        .expect("load attachment")
        .expect("attachment row")
        .try_get("", "deleted_at")
        .expect("read attachment tombstone");

    assert_eq!(draft.state, "finalized");
    assert!(
        draft.terminal_at.is_none(),
        "ordinary deletion must not transition a finalized draft to terminal cleanup"
    );
    assert_eq!(draft.finalized_comment_id, Some(comment.id.0));
    assert_eq!(draft.final_body_digest, Some(vec![1; 32]));
    assert_eq!(draft.final_request_digest, Some(vec![2; 32]));
    assert_eq!(
        upload
            .try_get::<Option<uuid::Uuid>>("", "attachment_id")
            .expect("read live attachment"),
        Some(attachment_id),
    );
    assert!(
        upload
            .try_get::<Option<chrono::DateTime<Utc>>>("", "deleted_at")
            .expect("read upload tombstone")
            .is_none(),
    );
    assert!(
        attachment_deleted_at.is_some(),
        "the finalized-origin attachment must be recoverably tombstoned"
    );
    assert!(
        store.exists(&digest).await.expect("object existence"),
        "ordinary deletion must retain the finalized-origin object"
    );

    let comment_deleted_at: chrono::DateTime<Utc> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM comments WHERE id = $1",
            [comment.id.0.into()],
        ))
        .await
        .expect("load comment tombstone")
        .expect("comment row")
        .try_get("", "deleted_at")
        .expect("comment delete timestamp");
    assert_eq!(attachment_deleted_at, Some(comment_deleted_at));

    let matching_attachment_id = uuid::Uuid::now_v7();
    let independent_deleted_at = comment_deleted_at + Duration::microseconds(1);
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachments \
             (id, workspace_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id, deleted_at) \
             VALUES ('{matching_attachment_id}', '{}', '{}', 'matching.txt', 'text/plain', 1, 'matching', '{}', '{}'); \
             UPDATE attachments SET deleted_at = '{}' WHERE id = '{}'",
            workspace.id.0,
            comment.id.0,
            user.id.0,
            comment_deleted_at.to_rfc3339(),
            independent_deleted_at.to_rfc3339(),
            attachment_id,
        ))
        .await
        .expect("seed matching and independent attachment tombstones");

    PgCommentRepo::restore_at_in(
        db.conn(),
        &ctx,
        CommentOwner::Task(task.id),
        comment.id,
        comment_deleted_at,
    )
    .await
    .expect("restore comment and matching attachment tombstones");

    let restored_comment: Option<chrono::DateTime<Utc>> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM comments WHERE id = $1",
            [comment.id.0.into()],
        ))
        .await
        .expect("load restored comment")
        .expect("restored comment row")
        .try_get("", "deleted_at")
        .expect("restored comment tombstone");
    let matching_attachment_deleted_at: Option<chrono::DateTime<Utc>> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM attachments WHERE id = $1",
            [matching_attachment_id.into()],
        ))
        .await
        .expect("load matching attachment")
        .expect("matching attachment row")
        .try_get("", "deleted_at")
        .expect("matching attachment tombstone");
    let independently_deleted_attachment_at: Option<chrono::DateTime<Utc>> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM attachments WHERE id = $1",
            [attachment_id.into()],
        ))
        .await
        .expect("load independently deleted attachment")
        .expect("independently deleted attachment row")
        .try_get("", "deleted_at")
        .expect("independently deleted attachment tombstone");

    assert!(restored_comment.is_none());
    assert!(matching_attachment_deleted_at.is_none());
    assert_eq!(
        independently_deleted_attachment_at,
        Some(independent_deleted_at)
    );

    db.teardown().await;
}

#[tokio::test]
async fn terminal_pruning_hard_deletes_finalized_origin_attachment_rows_before_removing_the_ledger()
{
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "finalized-origin-prune").await;
    let ctx = support::ctx(&workspace, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "finalized-origin-prune", "FP").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let comment = PgCommentRepo::new(db.conn().clone())
        .create(
            &ctx,
            NewComment {
                owner: CommentOwner::Task(task.id),
                body: "deleted finalized comment".into(),
            },
        )
        .await
        .expect("create comment");
    let draft = seed_draft(
        &db,
        workspace.id.0,
        task.id.0,
        user.id.0,
        "deleted_finalized",
        "2000-01-01T00:00:00Z",
        Some("2000-01-01T00:00:00Z"),
    )
    .await;
    let attachment_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE comment_attachment_drafts SET finalized_comment_id = '{}' WHERE id = '{}'; \
             INSERT INTO attachments (id, workspace_id, comment_id, file_name, content_type, size_bytes, sha256, created_by_user_id, deleted_at) \
             VALUES ('{attachment_id}', '{}', '{}', 'retained.txt', 'text/plain', 1, 'finalized-origin-prune', '{}', now()); \
             INSERT INTO comment_attachment_draft_uploads \
             (draft_id, upload_token, original_attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes, deleted_at) \
             VALUES ('{}', 'finalized-prune', '{attachment_id}', '\\x{}', '\\x{}', 'retained.txt', 'text/plain', 1, now())",
            comment.id.0,
            draft.0,
            workspace.id.0,
            comment.id.0,
            user.id.0,
            draft.0,
            "01".repeat(32),
            "02".repeat(32),
        ))
        .await
        .expect("seed retained finalized-origin attachment");
    let tempdir = TempDir::new().expect("tempdir");
    let store = DiskAttachmentStore::new(tempdir.path())
        .await
        .expect("attachment store");

    let report = PgAttachmentLifecycle::reconcile_drafts(&db.conn().clone(), &store)
        .await
        .expect("prune deleted finalized draft");
    let attachment_exists = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT id FROM attachments WHERE id = $1",
            [attachment_id.into()],
        ))
        .await
        .expect("lookup finalized-origin attachment")
        .is_some();

    assert_eq!(report.pruned, 1);
    assert!(
        !attachment_exists,
        "terminal pruning must hard-delete the finalized-origin metadata row before its ledger is removed"
    );

    db.teardown().await;
}

#[tokio::test]
async fn reconciler_shutdown_cancels_an_active_stale_intent_sweep_without_dropping_its_retry_intent()
 {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let store = Arc::new(BlockingDeleteStore::new());
    let digest = "shutdown-active-intent".to_string();
    PgAttachmentWriteIntentRepo {
        conn: db.conn().clone(),
    }
    .create(digest.clone())
    .await
    .expect("seed stale intent");
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let reconciler_store: Arc<dyn AttachmentStore> = store.clone();
    let reconciler = tokio::spawn(PgAttachmentLifecycle::run_reconciler_with_timing(
        db.conn().clone(),
        reconciler_store,
        shutdown_rx,
        std::time::Duration::from_secs(60),
        Duration::zero(),
    ));

    store.delete_started.notified().await;
    shutdown_tx.send(true).expect("signal shutdown");
    timeout(std::time::Duration::from_millis(250), reconciler)
        .await
        .expect("shutdown must not wait for a blocked stale-intent sweep")
        .expect("reconciler task");

    assert_eq!(
        PgAttachmentWriteIntentRepo {
            conn: db.conn().clone(),
        }
        .list_stale(Utc::now() + Duration::seconds(1))
        .await
        .expect("list retained intent")
        .into_iter()
        .map(|intent| intent.digest)
        .collect::<Vec<_>>(),
        vec![digest],
        "cancelling in-flight cleanup must leave the durable retry intent intact"
    );

    db.teardown().await;
}

#[tokio::test]
async fn upload_commits_write_intent_before_object_put() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "intent-before-put-user").await;
    let ctx = support::ctx(&ws, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "intent-before-put-proj", "IB").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let tempdir = TempDir::new().expect("tempdir");
    let store = IntentCheckingStore {
        inner: DiskAttachmentStore::new(tempdir.path())
            .await
            .expect("attachment store"),
        conn: db.conn().clone(),
        delete_started: Notify::new(),
        allow_delete: Notify::new(),
    };
    let data = b"intent must precede object put";

    let attachment = PgAttachmentLifecycle::store_and_record(
        &db.conn().clone(),
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: Some(task.id),
            comment_id: None,
            file_name: "intent.txt".into(),
            content_type: "text/plain".into(),
            size_bytes: data.len() as i64,
            sha256: String::new(),
        },
        data,
        &store,
    )
    .await
    .expect("upload");

    assert!(
        store
            .exists(&attachment.sha256)
            .await
            .expect("stored object exists"),
        "put must only run after the durable intent is externally observable"
    );

    db.teardown().await;
}

#[tokio::test]
async fn failed_finalization_leaves_object_for_later_reconciliation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "failed-finalization-user").await;
    let ctx = support::ctx(&ws, &user);
    let tempdir = TempDir::new().expect("tempdir");
    let store = DiskAttachmentStore::new(tempdir.path())
        .await
        .expect("attachment store");
    let data = b"object survives failed finalization";
    let digest = format!("{:x}", sha2::Sha256::digest(data));
    let intents = PgAttachmentWriteIntentRepo {
        conn: db.conn().clone(),
    };

    let upload = PgAttachmentLifecycle::store_and_record(
        &db.conn().clone(),
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: None,
            comment_id: None,
            file_name: "failed.txt".into(),
            content_type: "text/plain".into(),
            size_bytes: data.len() as i64,
            sha256: String::new(),
        },
        data,
        &store,
    )
    .await;

    assert!(upload.is_err(), "an ownerless attachment cannot finalize");
    assert!(
        store.exists(&digest).await.expect("object existence"),
        "a post-put finalization failure leaves a recoverable object"
    );
    assert_eq!(
        intents
            .list_stale(Utc::now() + Duration::seconds(1))
            .await
            .expect("list intents")
            .len(),
        1,
        "the durable intent must survive a failed finalization"
    );

    PgAttachmentLifecycle::reconcile_stale(
        &db.conn().clone(),
        &store,
        Utc::now() + Duration::seconds(1),
    )
    .await
    .expect("reconcile failed finalization");

    assert!(
        !store.exists(&digest).await.expect("object existence"),
        "reconciliation removes the orphaned object"
    );
    assert!(
        intents
            .list_stale(Utc::now() + Duration::seconds(1))
            .await
            .expect("list intents")
            .is_empty(),
        "reconciliation removes the recovered intent"
    );

    db.teardown().await;
}

#[tokio::test]
async fn concurrent_same_digest_uploads_finalize_without_residual_intent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "concurrent-finalization-user").await;
    let ctx = support::ctx(&ws, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "concurrent-finalization-proj", "CF").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let tempdir = TempDir::new().expect("tempdir");
    let store = Arc::new(
        DiskAttachmentStore::new(tempdir.path())
            .await
            .expect("attachment store"),
    );
    let data = b"concurrent same digest finalization";
    let digest = format!("{:x}", sha2::Sha256::digest(data));
    let first_conn = db.conn().clone();
    let second_conn = db.conn().clone();

    let first = PgAttachmentLifecycle::store_and_record(
        &first_conn,
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: Some(task.id),
            comment_id: None,
            file_name: "first.txt".into(),
            content_type: "text/plain".into(),
            size_bytes: data.len() as i64,
            sha256: String::new(),
        },
        data,
        store.as_ref(),
    );
    let second = PgAttachmentLifecycle::store_and_record(
        &second_conn,
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: Some(task.id),
            comment_id: None,
            file_name: "second.txt".into(),
            content_type: "text/plain".into(),
            size_bytes: data.len() as i64,
            sha256: String::new(),
        },
        data,
        store.as_ref(),
    );

    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first upload");
    let second = second.expect("second upload");

    assert_eq!(first.sha256, digest);
    assert_eq!(second.sha256, digest);
    assert!(
        store.exists(&digest).await.expect("object existence"),
        "both uploads must retain their shared object"
    );
    assert!(
        PgAttachmentWriteIntentRepo {
            conn: db.conn().clone(),
        }
        .list_stale(Utc::now() + Duration::seconds(1))
        .await
        .expect("list intents")
        .is_empty(),
        "both successful finalizations must clear their shared intent"
    );

    db.teardown().await;
}

#[tokio::test]
async fn delayed_put_does_not_hold_a_transaction_while_serializing_same_digest_uploads() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "session-lock-upload-user").await;
    let ctx = support::ctx(&ws, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "session-lock-upload-proj", "SL").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let store = Arc::new(DelayedPutStore::new());
    let data = b"session lock upload";

    let first_started = store.put_started.notified();
    let first_store = Arc::clone(&store);
    let first_conn = db.conn().clone();
    let first_ctx = ctx.clone();
    let first = tokio::spawn(async move {
        PgAttachmentLifecycle::store_and_record(
            &first_conn,
            &first_ctx,
            NewAttachment {
                document_id: None,
                task_id: Some(task.id),
                comment_id: None,
                file_name: "first.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: data.len() as i64,
                sha256: String::new(),
            },
            data,
            first_store.as_ref(),
        )
        .await
    });
    first_started.await;

    let second_store = Arc::clone(&store);
    let second_conn = db.conn().clone();
    let second_ctx = ctx.clone();
    let mut second = tokio::spawn(async move {
        PgAttachmentLifecycle::store_and_record(
            &second_conn,
            &second_ctx,
            NewAttachment {
                document_id: None,
                task_id: Some(task.id),
                comment_id: None,
                file_name: "second.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: data.len() as i64,
                sha256: String::new(),
            },
            data,
            second_store.as_ref(),
        )
        .await
    });

    assert!(
        timeout(std::time::Duration::from_millis(100), &mut second)
            .await
            .is_err(),
        "a same-digest upload must wait for the delayed upload to finalize"
    );

    let has_open_transaction = database_has_open_transaction(&db).await;

    let second_started = store.put_started.notified();
    store.allow_put.notify_one();
    first
        .await
        .expect("first upload task")
        .expect("first upload");
    second_started.await;
    store.allow_put.notify_one();
    second
        .await
        .expect("second upload task")
        .expect("second upload");

    db.teardown().await;

    assert!(
        !has_open_transaction,
        "delayed object-store I/O must not retain an open database transaction"
    );
}

#[tokio::test]
async fn cross_workspace_live_attachment_preserves_shared_object() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (first_ws, first_user) = support::seed_workspace(&db, "first-live-reference-user").await;
    let first_ctx = support::ctx(&first_ws, &first_user);
    let (second_ws, second_user) = support::seed_workspace(&db, "second-live-reference-user").await;
    let second_ctx = support::ctx(&second_ws, &second_user);
    let (project, board, column) =
        seed_project_board_column(&db, &second_ctx, "second-live-reference-proj", "LR").await;
    let task = seed_task(&db, &second_ctx, project.id, board.id, column.id, "Task").await;
    let tempdir = TempDir::new().expect("tempdir");
    let store = DiskAttachmentStore::new(tempdir.path())
        .await
        .expect("attachment store");
    let data = b"cross workspace shared object";

    let attachment = PgAttachmentLifecycle::store_and_record(
        &db.conn().clone(),
        &second_ctx,
        NewAttachment {
            document_id: None,
            task_id: Some(task.id),
            comment_id: None,
            file_name: "shared.txt".into(),
            content_type: "text/plain".into(),
            size_bytes: data.len() as i64,
            sha256: String::new(),
        },
        data,
        &store,
    )
    .await
    .expect("record live attachment in second workspace");
    let intents = PgAttachmentWriteIntentRepo {
        conn: db.conn().clone(),
    };
    intents
        .create(attachment.sha256.clone())
        .await
        .expect("seed stale intent in first workspace context");

    PgAttachmentLifecycle::reconcile_stale(
        &db.conn().clone(),
        &store,
        Utc::now() + Duration::seconds(1),
    )
    .await
    .expect("reconcile stale intent");

    assert!(
        store
            .exists(&attachment.sha256)
            .await
            .expect("object existence"),
        "a live attachment in another workspace must preserve the shared object"
    );
    assert!(
        intents
            .list_stale(Utc::now() + Duration::seconds(1))
            .await
            .expect("list intents")
            .is_empty(),
        "the stale intent is removed without deleting a globally live object"
    );

    assert_ne!(first_ctx.workspace_id, second_ctx.workspace_id);
    db.teardown().await;
}

struct IntentCheckingStore {
    inner: DiskAttachmentStore,
    conn: sea_orm::DatabaseConnection,
    delete_started: Notify,
    allow_delete: Notify,
}

#[derive(Clone)]
struct DelayedPutStore {
    inner: MemoryDeleteStore,
    put_started: Arc<Notify>,
    allow_put: Arc<Notify>,
}

impl DelayedPutStore {
    fn new() -> Self {
        Self {
            inner: MemoryDeleteStore::new(),
            put_started: Arc::new(Notify::new()),
            allow_put: Arc::new(Notify::new()),
        }
    }
}

#[async_trait]
impl AttachmentStore for DelayedPutStore {
    async fn put(&self, data: &[u8]) -> Result<String, DomainError> {
        self.put_started.notify_one();
        self.allow_put.notified().await;
        Ok(self.inner.store(data).await)
    }

    async fn get(&self, _digest: &str) -> Result<bytes::Bytes, DomainError> {
        Err(DomainError::NotFound {
            entity: "attachment",
            id: uuid::Uuid::nil(),
        })
    }

    async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
        Ok(self.inner.object_exists(digest).await)
    }

    async fn delete(&self, digest: &str) -> Result<(), DomainError> {
        self.inner.remove(digest).await;
        Ok(())
    }
}

#[derive(Clone)]
struct MemoryDeleteStore {
    objects: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
}

impl MemoryDeleteStore {
    fn new() -> Self {
        Self {
            objects: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    async fn store(&self, data: &[u8]) -> String {
        let digest = format!("{:x}", sha2::Sha256::digest(data));
        self.objects.lock().await.insert(digest.clone());
        digest
    }

    async fn object_exists(&self, digest: &str) -> bool {
        self.objects.lock().await.contains(digest)
    }

    async fn remove(&self, digest: &str) {
        self.objects.lock().await.remove(digest);
    }
}

#[derive(Clone)]
struct FailOnceDeleteStore {
    inner: MemoryDeleteStore,
    failures_remaining: Arc<tokio::sync::Mutex<u8>>,
}

impl FailOnceDeleteStore {
    fn new() -> Self {
        Self {
            inner: MemoryDeleteStore::new(),
            failures_remaining: Arc::new(tokio::sync::Mutex::new(1)),
        }
    }
}

#[async_trait]
impl AttachmentStore for FailOnceDeleteStore {
    async fn put(&self, data: &[u8]) -> Result<String, DomainError> {
        Ok(self.inner.store(data).await)
    }

    async fn get(&self, _digest: &str) -> Result<bytes::Bytes, DomainError> {
        Err(DomainError::NotFound {
            entity: "attachment",
            id: uuid::Uuid::nil(),
        })
    }

    async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
        Ok(self.inner.object_exists(digest).await)
    }

    async fn delete(&self, digest: &str) -> Result<(), DomainError> {
        let mut failures_remaining = self.failures_remaining.lock().await;
        if *failures_remaining > 0 {
            *failures_remaining -= 1;
            return Err(DomainError::Internal {
                message: "injected delete failure".into(),
            });
        }
        drop(failures_remaining);

        self.inner.remove(digest).await;
        Ok(())
    }
}

#[derive(Clone)]
struct SelectiveFailDeleteStore {
    inner: MemoryDeleteStore,
    failing_digest: String,
}

impl SelectiveFailDeleteStore {
    fn new(failing_digest: String) -> Self {
        Self {
            inner: MemoryDeleteStore::new(),
            failing_digest,
        }
    }
}

#[async_trait]
impl AttachmentStore for SelectiveFailDeleteStore {
    async fn put(&self, data: &[u8]) -> Result<String, DomainError> {
        Ok(self.inner.store(data).await)
    }

    async fn get(&self, _digest: &str) -> Result<bytes::Bytes, DomainError> {
        Err(DomainError::NotFound {
            entity: "attachment",
            id: uuid::Uuid::nil(),
        })
    }

    async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
        Ok(self.inner.object_exists(digest).await)
    }

    async fn delete(&self, digest: &str) -> Result<(), DomainError> {
        if digest == self.failing_digest {
            return Err(DomainError::Internal {
                message: "injected selected delete failure".into(),
            });
        }

        self.inner.remove(digest).await;
        Ok(())
    }
}

#[derive(Clone)]
struct AlwaysFailDeleteStore {
    inner: MemoryDeleteStore,
    delete_attempts: Arc<std::sync::atomic::AtomicUsize>,
    delete_attempted: Arc<Notify>,
}

#[derive(Clone)]
struct BlockingDeleteStore {
    delete_started: Arc<Notify>,
    allow_delete: Arc<Notify>,
}

impl BlockingDeleteStore {
    fn new() -> Self {
        Self {
            delete_started: Arc::new(Notify::new()),
            allow_delete: Arc::new(Notify::new()),
        }
    }
}

#[async_trait]
impl AttachmentStore for BlockingDeleteStore {
    async fn put(&self, data: &[u8]) -> Result<String, DomainError> {
        Ok(format!("{:x}", sha2::Sha256::digest(data)))
    }

    async fn get(&self, _digest: &str) -> Result<bytes::Bytes, DomainError> {
        Err(DomainError::NotFound {
            entity: "attachment",
            id: uuid::Uuid::nil(),
        })
    }

    async fn exists(&self, _digest: &str) -> Result<bool, DomainError> {
        Ok(false)
    }

    async fn delete(&self, _digest: &str) -> Result<(), DomainError> {
        self.delete_started.notify_waiters();
        self.allow_delete.notified().await;
        Ok(())
    }
}

impl AlwaysFailDeleteStore {
    fn new() -> Self {
        Self {
            inner: MemoryDeleteStore::new(),
            delete_attempts: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            delete_attempted: Arc::new(Notify::new()),
        }
    }

    async fn wait_for_delete_attempts(&self, minimum: usize) {
        while self.delete_attempts() < minimum {
            self.delete_attempted.notified().await;
        }
    }

    fn delete_attempts(&self) -> usize {
        self.delete_attempts
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl AttachmentStore for AlwaysFailDeleteStore {
    async fn put(&self, data: &[u8]) -> Result<String, DomainError> {
        Ok(self.inner.store(data).await)
    }

    async fn get(&self, _digest: &str) -> Result<bytes::Bytes, DomainError> {
        Err(DomainError::NotFound {
            entity: "attachment",
            id: uuid::Uuid::nil(),
        })
    }

    async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
        Ok(self.inner.object_exists(digest).await)
    }

    async fn delete(&self, _digest: &str) -> Result<(), DomainError> {
        self.delete_attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.delete_attempted.notify_waiters();

        Err(DomainError::Internal {
            message: "injected periodic delete failure".into(),
        })
    }
}

#[async_trait]
impl AttachmentStore for IntentCheckingStore {
    async fn put(&self, data: &[u8]) -> Result<String, DomainError> {
        let row = self
            .conn
            .query_one_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "SELECT EXISTS(SELECT 1 FROM attachment_write_intents) AS has_intent",
                [],
            ))
            .await
            .map_err(|error| DomainError::Internal {
                message: error.to_string(),
            })?
            .ok_or_else(|| DomainError::Internal {
                message: "intent query returned no row".into(),
            })?;

        let has_intent: bool =
            row.try_get("", "has_intent")
                .map_err(|error| DomainError::Internal {
                    message: error.to_string(),
                })?;

        if !has_intent {
            return Err(DomainError::Internal {
                message: "write intent was removed before put".into(),
            });
        }

        self.inner.put(data).await
    }

    async fn get(&self, digest: &str) -> Result<bytes::Bytes, DomainError> {
        self.inner.get(digest).await
    }

    async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
        self.inner.exists(digest).await
    }

    async fn delete(&self, digest: &str) -> Result<(), DomainError> {
        self.delete_started.notify_one();
        self.allow_delete.notified().await;
        self.inner.delete(digest).await
    }
}

async fn wait_for_advisory_lock_waiter(db: &support::TestDb) {
    for _ in 0..100 {
        let row = db
            .conn()
            .query_one_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "SELECT EXISTS(\
                    SELECT 1 FROM pg_stat_activity \
                    WHERE wait_event = 'advisory' \
                      AND query LIKE 'SELECT pg_advisory%lock%'\
                ) AS waiting",
                [],
            ))
            .await
            .expect("query advisory lock waiters")
            .expect("waiter query row");

        let waiting: bool = row.try_get("", "waiting").expect("read waiter state");
        if waiting {
            return;
        }

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    panic!("upload did not wait for the reconciler digest lock");
}

async fn database_has_open_transaction(db: &support::TestDb) -> bool {
    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT EXISTS(\
                SELECT 1 \
                FROM pg_stat_activity activity \
                JOIN pg_locks locks ON locks.pid = activity.pid \
                WHERE activity.datname = current_database() \
                  AND activity.xact_start IS NOT NULL \
                  AND locks.locktype = 'advisory' \
                  AND locks.granted \
                  AND activity.query LIKE 'SELECT pg_advisory%lock%'\
            ) AS has_open_transaction",
        ))
        .await
        .expect("query open transactions")
        .expect("open transaction query row");

    row.try_get("", "has_open_transaction")
        .expect("read open transaction state")
}

#[tokio::test]
async fn same_digest_reconciler_race_recommits_intent_before_put() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "comment-intent-race-user").await;
    let ctx = support::ctx(&ws, &user);
    let (project, board, column) =
        seed_project_board_column(&db, &ctx, "comment-intent-race-proj", "CR").await;
    let task = seed_task(&db, &ctx, project.id, board.id, column.id, "Task").await;
    let tempdir = TempDir::new().expect("tempdir");
    let store = Arc::new(IntentCheckingStore {
        inner: DiskAttachmentStore::new(tempdir.path())
            .await
            .expect("attachment store"),
        conn: db.conn().clone(),
        delete_started: Notify::new(),
        allow_delete: Notify::new(),
    });
    let data = b"same digest race";
    let digest = format!("{:x}", sha2::Sha256::digest(data));
    let intents = PgAttachmentWriteIntentRepo {
        conn: db.conn().clone(),
    };
    intents.create(digest).await.expect("seed stale intent");

    let reconcile_store = Arc::clone(&store);
    let reconcile_conn = db.conn().clone();
    let reconcile = tokio::spawn(async move {
        PgAttachmentLifecycle::reconcile_stale(
            &reconcile_conn,
            reconcile_store.as_ref(),
            Utc::now() + Duration::seconds(1),
        )
        .await
    });

    store.delete_started.notified().await;

    let upload_store = Arc::clone(&store);
    let upload_conn = db.conn().clone();
    let upload_ctx = ctx.clone();
    let upload = tokio::spawn(async move {
        PgAttachmentLifecycle::store_and_record(
            &upload_conn,
            &upload_ctx,
            NewAttachment {
                document_id: None,
                task_id: Some(task.id),
                comment_id: None,
                file_name: "race.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: data.len() as i64,
                sha256: String::new(),
            },
            data,
            upload_store.as_ref(),
        )
        .await
    });

    wait_for_advisory_lock_waiter(&db).await;
    store.allow_delete.notify_one();

    reconcile
        .await
        .expect("reconciler task")
        .expect("reconciler result");
    let attachment = upload.await.expect("upload task").expect("upload result");

    assert_eq!(
        attachment.sha256,
        format!("{:x}", sha2::Sha256::digest(data))
    );
    assert!(
        store
            .exists(&attachment.sha256)
            .await
            .expect("stored object exists"),
        "the uploader must retain the object after finalization"
    );

    db.teardown().await;
}
