#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use async_trait::async_trait;
use atlas_domain::ports::comments::CommentLinkRepo;
use atlas_domain::{
    Actor, AttachmentStore, DomainError,
    entities::boards_tasks::{NewBoard, NewTask, PositionBetween},
    entities::comments::{CommentFeedEntry, CommentLinkTarget, CommentOwner, NewComment},
    entities::documents::NewAttachment,
    entities::workspace_core::NewProject,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    AttachmentWriteIntentRepo, BoardRepo, CommentRepo, DiskAttachmentStore, PgAttachmentLifecycle,
    PgAttachmentWriteIntentRepo, PgBoardRepo, PgCommentLinkRepo, PgCommentRepo, PgProjectRepo,
    PgTaskRepo, ProjectRepo, TaskRepo,
};
use chrono::{Duration, Utc};
use sea_orm::{ConnectionTrait, Statement};
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
            definition.contains("CHECK ((num_nonnulls(document_id, task_id, comment_id) = 1))")
        }),
        "attachments must require exactly one document, task, or comment owner"
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
        .expect("parent feed");
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
