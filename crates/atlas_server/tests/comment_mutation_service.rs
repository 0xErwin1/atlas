#![allow(clippy::expect_used)]

mod support;

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
    services::CommentService,
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

    let typed_reference_count = db
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
        typed_reference_count, 0,
        "comment links must not mutate typed references"
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
