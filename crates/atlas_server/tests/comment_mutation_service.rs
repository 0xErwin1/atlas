#![allow(clippy::expect_used)]

mod support;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use atlas_domain::{
    AttachmentStore,
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        comments::{CommentFeedEntry, CommentLinkEventKind, CommentLinkTarget, CommentOwner},
        documents::{AttachmentOwner, NewAttachment, NewDocument},
        workspace_core::NewProject,
    },
    permissions::{Visibility, VisibilityRole},
    ports::comments::CommentLinkRepo,
};
use atlas_server::{
    persistence::repos::{
        AttachmentRepo, BoardRepo, DiskAttachmentStore, DocumentRepo, PgAttachmentLifecycle,
        PgAttachmentRepo, PgBoardRepo, PgCommentLinkRepo, PgDocumentRepo, PgProjectRepo,
        PgTaskRepo, ProjectRepo, TaskRepo,
    },
    services::{CommentMutationFault, CommentService},
};
use sea_orm::{ConnectionTrait, Statement};

async fn seed_task(
    db: &support::TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    title: &str,
) -> atlas_domain::entities::boards_tasks::Task {
    let project_id = uuid::Uuid::now_v7();
    let project_suffix = &project_id.to_string()[28..];
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        ctx,
        NewProject {
            name: format!("Comment mutations {project_suffix}"),
            slug: format!("comment-mutations-{project_suffix}"),
            task_prefix: format!("CM{project_suffix}").to_uppercase(),
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
    let column = PgBoardRepo::new(db.conn().clone())
        .add_column(
            ctx,
            board.id,
            "Todo".into(),
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
                column_id: column.id,
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
async fn document_comment_delete_retains_events_and_preserves_shared_digest_for_retry() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "comment-mutation-document").await;
    let ctx = support::ctx(&workspace, &user);
    let shared_task = seed_task(&db, &ctx, "Shared digest owner").await;
    let task_target = seed_task(&db, &ctx, "Document comment target").await;
    let parent = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &ctx,
            NewDocument {
                title: "Comment parent".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed parent document");
    let store_root =
        std::env::temp_dir().join(format!("atlas-comment-purge-{}", uuid::Uuid::now_v7()));
    let store = DiskAttachmentStore::new(&store_root)
        .await
        .expect("create attachment store");
    let digest = store
        .put(b"shared comment attachment")
        .await
        .expect("store object");
    let attachments = PgAttachmentRepo {
        conn: db.conn().clone(),
    };
    attachments
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: Some(shared_task.id),
                comment_id: None,
                file_name: "shared.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 25,
                sha256: digest.clone(),
            },
        )
        .await
        .expect("record shared attachment");
    let service = CommentService::new(db.conn().clone());
    let comment = service
        .create(
            &ctx,
            CommentOwner::Document(parent.id),
            format!("[[{}|Task target]]", task_target.id.0),
        )
        .await
        .expect("create document comment");
    attachments
        .record(
            &ctx,
            NewAttachment {
                document_id: None,
                task_id: None,
                comment_id: Some(comment.id),
                file_name: "comment.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 25,
                sha256: digest.clone(),
            },
        )
        .await
        .expect("record comment attachment");

    service
        .remove(&ctx, CommentOwner::Document(parent.id), comment.id, false)
        .await
        .expect("delete document comment");

    assert!(
        attachments
            .list_for_owner(&ctx, AttachmentOwner::Comment(comment.id))
            .await
            .expect("list removed comment attachments")
            .is_empty(),
        "comment-owned attachment rows must be hard-deleted before the comment is hidden"
    );
    assert!(
        PgCommentLinkRepo::new(db.conn().clone())
            .links_for_comments(&ctx, &[comment.id])
            .await
            .expect("load removed links")
            .is_empty(),
        "the live graph must be removed before the comment is soft-deleted"
    );
    let event_kinds = PgCommentLinkRepo::new(db.conn().clone())
        .feed_for_owner(&ctx, CommentOwner::Document(parent.id), None, 20)
        .await
        .expect("load retained events")
        .entries
        .into_iter()
        .filter_map(|entry| match entry {
            CommentFeedEntry::Event(event) => Some(event.kind),
            CommentFeedEntry::Comment(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        event_kinds,
        vec![
            CommentLinkEventKind::LinkAdded,
            CommentLinkEventKind::LinkRemoved,
            CommentLinkEventKind::CommentDeleted,
        ]
    );

    PgAttachmentLifecycle::finish_purge_digest(db.conn(), &store, &digest)
        .await
        .expect("retry durable cleanup");
    assert!(
        store.exists(&digest).await.expect("check shared object"),
        "a live attachment sharing the digest must prevent purge"
    );

    std::fs::remove_dir_all(store_root).expect("remove attachment store");
    db.teardown().await;
}

#[tokio::test]
async fn task_comment_mutations_keep_markdown_verbatim_and_replace_only_derived_links() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "comment-mutation-task").await;
    let ctx = support::ctx(&workspace, &user);
    let parent = seed_task(&db, &ctx, "Parent").await;
    let task_target = seed_task(&db, &ctx, "Task target").await;
    let document_target = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &ctx,
            NewDocument {
                title: "Document target".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document target");
    let body = format!(
        "before [[{}|Original title]] [[{}|Task]] [[{}|Unknown]] after",
        document_target.id.0,
        task_target.id.0,
        uuid::Uuid::now_v7(),
    );
    let service = CommentService::new(db.conn().clone());

    let comment = service
        .create(&ctx, CommentOwner::Task(parent.id), body.clone())
        .await
        .expect("create linked comment");

    assert_eq!(comment.body, body, "comment Markdown must remain verbatim");

    let links = PgCommentLinkRepo::new(db.conn().clone())
        .links_for_comments(&ctx, &[comment.id])
        .await
        .expect("load derived links");
    assert_eq!(
        links
            .into_iter()
            .map(|link| link.target)
            .collect::<Vec<_>>(),
        vec![
            CommentLinkTarget::Document(document_target.id),
            CommentLinkTarget::Task(task_target.id),
        ]
    );

    let initial_typed_reference_count = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT count(*) AS count FROM task_references WHERE source_task_id = '{}'",
                parent.id.0
            ),
        ))
        .await
        .expect("count typed references")
        .expect("count row")
        .try_get::<i64>("", "count")
        .expect("typed reference count");
    assert_eq!(
        initial_typed_reference_count, 0,
        "comment links must not mutate typed references"
    );
    assert_eq!(
        document_link_count(&db).await,
        0,
        "comment links must not mutate document links"
    );

    let updated_body = format!("[[{}|Task remains]]", task_target.id.0);
    let updated = service
        .update(
            &ctx,
            CommentOwner::Task(parent.id),
            comment.id,
            updated_body.clone(),
        )
        .await
        .expect("replace comment links");
    assert_eq!(updated.body, updated_body);

    let links = PgCommentLinkRepo::new(db.conn().clone())
        .links_for_comments(&ctx, &[comment.id])
        .await
        .expect("load replacement links");
    assert_eq!(
        links
            .into_iter()
            .map(|link| link.target)
            .collect::<Vec<_>>(),
        vec![CommentLinkTarget::Task(task_target.id)]
    );
    assert_eq!(
        comment_event_kinds(&db, comment.id).await,
        vec!["link_added", "link_added", "link_removed"],
        "create/update must append the exact derived-link event diff"
    );
    assert_eq!(typed_reference_count(&db).await, 0);
    assert_eq!(document_link_count(&db).await, 0);

    db.teardown().await;
}

#[tokio::test]
async fn document_comment_mutations_keep_markdown_verbatim_and_isolate_typed_links() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "comment-mutation-document-success").await;
    let ctx = support::ctx(&workspace, &user);
    let task_target = seed_task(&db, &ctx, "Task target").await;
    let document_parent = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &ctx,
            NewDocument {
                title: "Document parent".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document parent");
    let document_target = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &ctx,
            NewDocument {
                title: "Document target".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document target");
    let service = CommentService::new(db.conn().clone());
    let body = format!("before [[{}|Document]] after", document_target.id.0);

    let comment = service
        .create(
            &ctx,
            CommentOwner::Document(document_parent.id),
            body.clone(),
        )
        .await
        .expect("create document comment");
    assert_eq!(comment.body, body);
    assert_eq!(
        PgCommentLinkRepo::new(db.conn().clone())
            .links_for_comments(&ctx, &[comment.id])
            .await
            .expect("load document comment links")
            .into_iter()
            .map(|link| link.target)
            .collect::<Vec<_>>(),
        vec![CommentLinkTarget::Document(document_target.id)]
    );

    let updated_body = format!("updated [[{}|Task]]", task_target.id.0);
    let updated = service
        .update(
            &ctx,
            CommentOwner::Document(document_parent.id),
            comment.id,
            updated_body.clone(),
        )
        .await
        .expect("update document comment");
    assert_eq!(updated.body, updated_body);
    assert_eq!(
        PgCommentLinkRepo::new(db.conn().clone())
            .links_for_comments(&ctx, &[comment.id])
            .await
            .expect("load replacement links")
            .into_iter()
            .map(|link| link.target)
            .collect::<Vec<_>>(),
        vec![CommentLinkTarget::Task(task_target.id)]
    );
    assert_eq!(
        comment_event_kinds(&db, comment.id).await,
        vec!["link_added", "link_removed", "link_added"],
        "document create/update must append the exact derived-link event diff"
    );

    service
        .remove(
            &ctx,
            CommentOwner::Document(document_parent.id),
            comment.id,
            false,
        )
        .await
        .expect("delete document comment");
    assert_eq!(
        comment_event_kinds(&db, comment.id).await,
        vec![
            "link_added",
            "link_removed",
            "link_added",
            "link_removed",
            "comment_deleted"
        ],
        "delete must retain removal and deletion events"
    );
    assert_eq!(typed_reference_count(&db).await, 0);
    assert_eq!(document_link_count(&db).await, 0);

    db.teardown().await;
}

#[tokio::test]
async fn attachment_urls_require_the_canonical_task_comment_owner_chain() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "comment-owner-chain").await;
    let ctx = support::ctx(&workspace, &user);
    let attachment_parent = seed_task(&db, &ctx, "Attachment parent").await;
    let claimed_parent = seed_task(&db, &ctx, "Claimed parent").await;
    let source_parent = seed_task(&db, &ctx, "Source parent").await;
    let service = CommentService::new(db.conn().clone());

    let attachment_comment = service
        .create(
            &ctx,
            CommentOwner::Task(attachment_parent.id),
            "attachment owner".into(),
        )
        .await
        .expect("create attachment owner comment");
    let attachment = PgAttachmentRepo {
        conn: db.conn().clone(),
    }
    .record(
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: None,
            comment_id: Some(attachment_comment.id),
            file_name: "attachment.txt".into(),
            content_type: "text/plain".into(),
            size_bytes: 1,
            sha256: "a".repeat(64),
        },
    )
    .await
    .expect("record comment attachment");

    let valid_source = service
        .create(
            &ctx,
            CommentOwner::Task(source_parent.id),
            format!(
                "[matching owner](/api/workspaces/ws-comment-owner-chain/tasks/{}/comments/{}/attachments/{}/content)",
                attachment_parent.readable_id, attachment_comment.id.0, attachment.id.0
            ),
        )
        .await
        .expect("create comment with matching attachment URL");
    assert_eq!(
        PgCommentLinkRepo::new(db.conn().clone())
            .links_for_comments(&ctx, &[valid_source.id])
            .await
            .expect("load matching derived link")
            .into_iter()
            .map(|link| link.target)
            .collect::<Vec<_>>(),
        vec![CommentLinkTarget::Attachment(attachment.id)],
        "a canonical URL must resolve when it names the actual task parent"
    );

    let source = service
        .create(
            &ctx,
            CommentOwner::Task(source_parent.id),
            format!(
                "[mismatched owner](/api/workspaces/ws-comment-owner-chain/tasks/{}/comments/{}/attachments/{}/content)",
                claimed_parent.readable_id, attachment_comment.id.0, attachment.id.0
            ),
        )
        .await
        .expect("create comment with mismatched attachment URL");

    assert!(
        PgCommentLinkRepo::new(db.conn().clone())
            .links_for_comments(&ctx, &[source.id])
            .await
            .expect("load derived links")
            .is_empty(),
        "a canonical URL must name the actual task parent of its comment attachment"
    );

    db.teardown().await;
}

#[tokio::test]
async fn comment_mutation_faults_rollback_task_and_document_bodies_graphs_and_events() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "comment-mutation-faults").await;
    let ctx = support::ctx(&workspace, &user);
    let task_parent = seed_task(&db, &ctx, "Task parent").await;
    let task_target = seed_task(&db, &ctx, "Task target").await;
    let document_parent = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &ctx,
            NewDocument {
                title: "Document parent".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document parent");

    for owner in [
        CommentOwner::Task(task_parent.id),
        CommentOwner::Document(document_parent.id),
    ] {
        for fault in [
            CommentMutationFault::AfterBodyWrite,
            CommentMutationFault::AfterGraphReplace,
            CommentMutationFault::AfterEventAppend,
        ] {
            let service = CommentService::with_fault_injection(db.conn().clone(), fault);
            let result = service
                .create(
                    &ctx,
                    owner,
                    format!("verbatim [[{}|Task target]]", task_target.id.0),
                )
                .await;

            assert!(result.is_err(), "{fault:?} must fail the transaction");

            let counts = db
                .conn()
                .query_one_raw(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT \
                        (SELECT count(*) FROM comments WHERE workspace_id = $1) AS comments, \
                        (SELECT count(*) FROM comment_links WHERE workspace_id = $1) AS links, \
                        (SELECT count(*) FROM comment_link_events WHERE workspace_id = $1) AS events",
                    [workspace.id.0.into()],
                ))
                .await
                .expect("count rolled back rows")
                .expect("count row");

            assert_eq!(
                counts
                    .try_get::<i64>("", "comments")
                    .expect("comment count"),
                0,
                "{fault:?} must roll back the comment body"
            );
            assert_eq!(
                counts.try_get::<i64>("", "links").expect("link count"),
                0,
                "{fault:?} must roll back the derived graph"
            );
            assert_eq!(
                counts.try_get::<i64>("", "events").expect("event count"),
                0,
                "{fault:?} must roll back retained events"
            );
        }
    }

    db.teardown().await;
}

#[tokio::test]
async fn comment_update_faults_preserve_existing_task_and_document_state() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "comment-update-faults").await;
    let ctx = support::ctx(&workspace, &user);
    let task_parent = seed_task(&db, &ctx, "Task parent").await;
    let task_target = seed_task(&db, &ctx, "Task target").await;
    let document_target = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &ctx,
            NewDocument {
                title: "Document target".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document target");
    let document_parent = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &ctx,
            NewDocument {
                title: "Document parent".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document parent");

    for owner in [
        CommentOwner::Task(task_parent.id),
        CommentOwner::Document(document_parent.id),
    ] {
        for fault in [
            CommentMutationFault::AfterBodyWrite,
            CommentMutationFault::AfterGraphReplace,
            CommentMutationFault::AfterEventAppend,
        ] {
            let original_body = format!("original {fault:?} [[{}|Task]]", task_target.id.0);
            let comment = CommentService::new(db.conn().clone())
                .create(&ctx, owner, original_body.clone())
                .await
                .expect("seed linked comment");
            let before_event_count = comment_event_count(&db, comment.id).await;

            let result = CommentService::with_fault_injection(db.conn().clone(), fault)
                .update(
                    &ctx,
                    owner,
                    comment.id,
                    format!("updated [[{}|Document]]", document_target.id.0),
                )
                .await;

            assert!(result.is_err(), "{fault:?} must fail the update");
            assert_eq!(comment_body(&db, comment.id).await, original_body);
            assert_eq!(
                comment_event_count(&db, comment.id).await,
                before_event_count
            );
            assert_eq!(
                PgCommentLinkRepo::new(db.conn().clone())
                    .links_for_comments(&ctx, &[comment.id])
                    .await
                    .expect("load preserved links")
                    .into_iter()
                    .map(|link| link.target)
                    .collect::<Vec<_>>(),
                vec![CommentLinkTarget::Task(task_target.id)],
                "{fault:?} must preserve the old derived graph"
            );
        }
    }

    db.teardown().await;
}

#[tokio::test]
async fn task_comment_delete_keeps_rows_unreachable_when_post_commit_object_cleanup_fails() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (workspace, user) = support::seed_workspace(&db, "comment-delete-retry").await;
    let ctx = support::ctx(&workspace, &user);
    let parent = seed_task(&db, &ctx, "Task parent").await;
    let target = seed_task(&db, &ctx, "Task target").await;
    let store_root =
        std::env::temp_dir().join(format!("atlas-comment-retry-{}", uuid::Uuid::now_v7()));
    let store = FailOnceDeleteStore::new(&store_root).await;
    let service = CommentService::with_attachment_store(db.conn().clone(), Arc::new(store.clone()));
    let comment = service
        .create(
            &ctx,
            CommentOwner::Task(parent.id),
            format!("linked [[{}|Target]]", target.id.0),
        )
        .await
        .expect("create linked comment");
    let digest = store
        .put(b"purge after commit")
        .await
        .expect("store object");
    PgAttachmentRepo {
        conn: db.conn().clone(),
    }
    .record(
        &ctx,
        NewAttachment {
            document_id: None,
            task_id: None,
            comment_id: Some(comment.id),
            file_name: "comment.txt".into(),
            content_type: "text/plain".into(),
            size_bytes: 18,
            sha256: digest.clone(),
        },
    )
    .await
    .expect("record comment attachment");

    service
        .remove(&ctx, CommentOwner::Task(parent.id), comment.id, false)
        .await
        .expect("deletion commits despite object cleanup failure");

    assert!(comment_deleted(&db, comment.id).await);
    assert!(
        PgCommentLinkRepo::new(db.conn().clone())
            .links_for_comments(&ctx, &[comment.id])
            .await
            .expect("load deleted links")
            .is_empty()
    );
    assert!(
        PgAttachmentRepo {
            conn: db.conn().clone(),
        }
        .list_for_owner(&ctx, AttachmentOwner::Comment(comment.id))
        .await
        .expect("list deleted attachment")
        .is_empty()
    );
    assert!(
        store
            .exists(&digest)
            .await
            .expect("object remains for retry")
    );
    assert!(digest_has_cleanup_intent(&db, &digest).await);
    assert_eq!(typed_reference_count(&db).await, 0);
    assert_eq!(document_link_count(&db).await, 0);

    PgAttachmentLifecycle::finish_purge_digest(db.conn(), &store, &digest)
        .await
        .expect("durable retry succeeds");
    assert!(
        !store
            .exists(&digest)
            .await
            .expect("object removed by retry")
    );
    assert!(!digest_has_cleanup_intent(&db, &digest).await);

    std::fs::remove_dir_all(store_root).expect("remove attachment store");
    db.teardown().await;
}

async fn comment_body(db: &support::TestDb, comment_id: atlas_domain::ids::CommentId) -> String {
    db.conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT body FROM comments WHERE id = $1",
            [comment_id.0.into()],
        ))
        .await
        .expect("load comment body")
        .expect("comment body row")
        .try_get("", "body")
        .expect("comment body")
}

async fn comment_event_count(
    db: &support::TestDb,
    comment_id: atlas_domain::ids::CommentId,
) -> i64 {
    db.conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM comment_link_events WHERE comment_id = $1",
            [comment_id.0.into()],
        ))
        .await
        .expect("count comment events")
        .expect("comment event count row")
        .try_get("", "count")
        .expect("comment event count")
}

async fn comment_event_kinds(
    db: &support::TestDb,
    comment_id: atlas_domain::ids::CommentId,
) -> Vec<String> {
    db.conn()
        .query_all_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT event_kind FROM comment_link_events WHERE comment_id = $1 ORDER BY created_at, id",
            [comment_id.0.into()],
        ))
        .await
        .expect("load comment events")
        .into_iter()
        .map(|row| row.try_get("", "event_kind").expect("event kind"))
        .collect()
}

async fn typed_reference_count(db: &support::TestDb) -> i64 {
    table_count(db, "task_references").await
}

async fn document_link_count(db: &support::TestDb) -> i64 {
    table_count(db, "document_links").await
}

async fn table_count(db: &support::TestDb, table: &str) -> i64 {
    db.conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT count(*) AS count FROM {table}"),
        ))
        .await
        .expect("count table rows")
        .expect("table count row")
        .try_get("", "count")
        .expect("table count")
}

async fn comment_deleted(db: &support::TestDb, comment_id: atlas_domain::ids::CommentId) -> bool {
    db.conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at IS NOT NULL AS deleted FROM comments WHERE id = $1",
            [comment_id.0.into()],
        ))
        .await
        .expect("load deleted comment")
        .expect("deleted comment row")
        .try_get("", "deleted")
        .expect("deleted state")
}

async fn digest_has_cleanup_intent(db: &support::TestDb, digest: &str) -> bool {
    db.conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT EXISTS(SELECT 1 FROM attachment_write_intents WHERE digest = $1) AS exists",
            [digest.into()],
        ))
        .await
        .expect("load cleanup intent")
        .expect("cleanup intent row")
        .try_get("", "exists")
        .expect("cleanup intent state")
}

#[derive(Clone)]
struct FailOnceDeleteStore {
    inner: Arc<DiskAttachmentStore>,
    should_fail: Arc<AtomicBool>,
}

impl FailOnceDeleteStore {
    async fn new(root: &std::path::Path) -> Self {
        Self {
            inner: Arc::new(
                DiskAttachmentStore::new(root)
                    .await
                    .expect("create attachment store"),
            ),
            should_fail: Arc::new(AtomicBool::new(true)),
        }
    }
}

#[async_trait]
impl AttachmentStore for FailOnceDeleteStore {
    async fn put(&self, data: &[u8]) -> Result<String, atlas_domain::DomainError> {
        self.inner.put(data).await
    }

    async fn get(&self, digest: &str) -> Result<bytes::Bytes, atlas_domain::DomainError> {
        self.inner.get(digest).await
    }

    async fn exists(&self, digest: &str) -> Result<bool, atlas_domain::DomainError> {
        self.inner.exists(digest).await
    }

    async fn delete(&self, digest: &str) -> Result<(), atlas_domain::DomainError> {
        if self.should_fail.swap(false, Ordering::SeqCst) {
            return Err(atlas_domain::DomainError::Internal {
                message: "injected post-commit object deletion failure".into(),
            });
        }

        self.inner.delete(digest).await
    }
}
