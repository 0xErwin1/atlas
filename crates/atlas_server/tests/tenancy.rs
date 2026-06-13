#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::entities::boards_tasks::{NewBoard, NewTask, PositionBetween};
use atlas_domain::entities::documents::NewDocument;
use atlas_domain::entities::workspace_core::{
    AppliesTo, NewFolder, NewProject, NewPropertyDefinition, PropertyKind,
};
use atlas_domain::permissions::{Visibility, VisibilityRole};
use atlas_server::persistence::repos::{
    ApiKeyRepo, AttachmentRepo, BoardRepo, DocumentLinkRepo, DocumentRepo, FolderRepo,
    MembershipRepo, PgAttachmentRepo, PgBoardRepo, PgDocumentLinkRepo, PgDocumentRepo,
    PgFolderRepo, PgMembershipRepo, PgPropertyDefinitionRepo, PgTaskReferenceRepo, PgTaskRepo,
    ProjectRepo, PropertyDefinitionRepo, TaskReferenceRepo, TaskRepo, UserRepo,
};

#[tokio::test]
async fn api_key_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice").await;
    let (_ws_b, _user_b) = support::seed_workspace(&db, "bob").await;

    let ctx_a = support::ctx(&ws_a, &user_a);

    let repo = db.api_key_repo();

    let keys_a = repo.list(&ctx_a).await.expect("list keys for workspace A");
    assert!(
        keys_a.is_empty(),
        "workspace A must not see workspace B's api keys"
    );

    db.teardown().await;
}

#[tokio::test]
async fn project_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-proj").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-proj").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);
    let repo = db.project_repo();

    repo.create(
        &ctx_b,
        NewProject {
            name: "Bob's Project".into(),
            slug: "bobs-project".into(),
            task_prefix: "BP".into(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("create project in ws_b");

    let projects_a = repo.list(&ctx_a).await.expect("list for ws_a");
    assert!(
        projects_a.is_empty(),
        "workspace A must not see workspace B's projects"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-task").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-task").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);

    let proj_repo = db.project_repo();
    let board_repo = PgBoardRepo::new(db.conn().clone());
    let task_repo = PgTaskRepo::new(db.conn().clone());

    let proj_b = proj_repo
        .create(
            &ctx_b,
            NewProject {
                name: "Bob Project".into(),
                slug: "bob-task-proj".into(),
                task_prefix: "BTKP".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project in ws_b");

    let board_b = board_repo
        .create_board(
            &ctx_b,
            NewBoard {
                project_id: proj_b.id,
                name: "Main".into(),
            },
        )
        .await
        .expect("create board in ws_b");

    let col_b = board_repo
        .add_column(
            &ctx_b,
            board_b.id,
            "Backlog".into(),
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column in ws_b");

    task_repo
        .create(
            &ctx_b,
            NewTask {
                project_id: proj_b.id,
                board_id: board_b.id,
                column_id: col_b.id,
                title: "Bob's Task".into(),
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
        .expect("create task in ws_b");

    let tasks_a = task_repo
        .list_by_column(&ctx_a, col_b.id)
        .await
        .expect("list tasks from ws_a perspective");

    assert!(
        tasks_a.is_empty(),
        "workspace A must not see workspace B's tasks"
    );

    db.teardown().await;
}

#[tokio::test]
async fn folder_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-folder").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-folder").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);
    let folder_repo = PgFolderRepo {
        conn: db.conn().clone(),
    };

    folder_repo
        .create(
            &ctx_b,
            NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "Bob's Folder".into(),
            },
        )
        .await
        .expect("create folder in ws_b");

    let folders_a = folder_repo
        .list_children(&ctx_a, None)
        .await
        .expect("list children from ws_a");

    assert!(
        folders_a.is_empty(),
        "workspace A must not see workspace B's folders"
    );

    db.teardown().await;
}

#[tokio::test]
async fn property_definition_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-prop").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-prop").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);
    let repo = PgPropertyDefinitionRepo {
        conn: db.conn().clone(),
    };

    repo.create(
        &ctx_b,
        NewPropertyDefinition {
            key: "status".into(),
            name: "Status".into(),
            kind: PropertyKind::Select,
            options: None,
            applies_to: AppliesTo::Task,
        },
    )
    .await
    .expect("create property definition in ws_b");

    let defs_a = repo.list(&ctx_a).await.expect("list from ws_a");

    assert!(
        defs_a.is_empty(),
        "workspace A must not see workspace B's property definitions"
    );

    db.teardown().await;
}

#[tokio::test]
async fn document_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-doc").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-doc").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 10);

    let doc_b = doc_repo
        .create(
            &ctx_b,
            NewDocument {
                title: "Bob's Doc".into(),
                slug: None,
                content: "hello".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document in ws_b");

    let found = doc_repo
        .get(&ctx_a, doc_b.id)
        .await
        .expect("get from ws_a perspective");

    assert!(
        found.is_none(),
        "workspace A must not see workspace B's document by ID"
    );

    db.teardown().await;
}

#[tokio::test]
async fn board_repo_workspace_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-board").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-board").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);

    let proj_repo = db.project_repo();
    let board_repo = PgBoardRepo::new(db.conn().clone());

    let proj_b = proj_repo
        .create(
            &ctx_b,
            NewProject {
                name: "Bob Board Project".into(),
                slug: "bob-board-proj".into(),
                task_prefix: "BBP".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project in ws_b");

    let board_b = board_repo
        .create_board(
            &ctx_b,
            NewBoard {
                project_id: proj_b.id,
                name: "Bob Board".into(),
            },
        )
        .await
        .expect("create board in ws_b");

    let found = board_repo
        .find_board(&ctx_a, board_b.id)
        .await
        .expect("find board from ws_a perspective");

    assert!(
        found.is_none(),
        "workspace A must not see workspace B's board by ID"
    );

    db.teardown().await;
}

#[tokio::test]
async fn task_reference_repo_workspace_isolation() {
    use atlas_domain::entities::boards_tasks::NewTaskReference;
    use atlas_domain::entities::boards_tasks::ReferenceKind;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-taskref").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-taskref").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);

    let proj_repo = db.project_repo();
    let board_repo = PgBoardRepo::new(db.conn().clone());
    let task_repo = PgTaskRepo::new(db.conn().clone());
    let ref_repo = PgTaskReferenceRepo::new(db.conn().clone());

    let proj_b = proj_repo
        .create(
            &ctx_b,
            NewProject {
                name: "Bob Ref Project".into(),
                slug: "bob-ref-proj".into(),
                task_prefix: "BRP".into(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("create project in ws_b");

    let board_b = board_repo
        .create_board(
            &ctx_b,
            NewBoard {
                project_id: proj_b.id,
                name: "Main".into(),
            },
        )
        .await
        .expect("create board in ws_b");

    let col_b = board_repo
        .add_column(
            &ctx_b,
            board_b.id,
            "Backlog".into(),
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column in ws_b");

    let source_task_b = task_repo
        .create(
            &ctx_b,
            NewTask {
                project_id: proj_b.id,
                board_id: board_b.id,
                column_id: col_b.id,
                title: "Source Task".into(),
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
        .expect("create source task in ws_b");

    let target_task_b = task_repo
        .create(
            &ctx_b,
            NewTask {
                project_id: proj_b.id,
                board_id: board_b.id,
                column_id: col_b.id,
                title: "Target Task".into(),
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
        .expect("create target task in ws_b");

    ref_repo
        .create(
            &ctx_b,
            NewTaskReference {
                source_task_id: source_task_b.id,
                kind: ReferenceKind::Blocks,
                target_task_id: Some(target_task_b.id),
                target_document_id: None,
            },
        )
        .await
        .expect("create task reference in ws_b");

    let refs_a = ref_repo
        .list_for_task(&ctx_a, source_task_b.id)
        .await
        .expect("list task references from ws_a perspective");

    assert!(
        refs_a.is_empty(),
        "workspace A must not see workspace B's task references"
    );

    db.teardown().await;
}

#[tokio::test]
async fn document_link_repo_workspace_isolation() {
    use atlas_domain::entities::documents::ExtractedLink;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-doclink").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-doclink").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);

    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 10);
    let link_repo = PgDocumentLinkRepo {
        conn: db.conn().clone(),
    };

    let source_doc_b = doc_repo
        .create(
            &ctx_b,
            NewDocument {
                title: "Bob Source Doc".into(),
                slug: None,
                content: "hello".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create source doc in ws_b");

    let target_doc_b = doc_repo
        .create(
            &ctx_b,
            NewDocument {
                title: "Bob Target Doc".into(),
                slug: None,
                content: "world".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create target doc in ws_b");

    link_repo
        .replace_for_source(
            &ctx_b,
            source_doc_b.id,
            vec![ExtractedLink {
                target_document_id: Some(target_doc_b.id),
                target_title: "Bob Target Doc".into(),
            }],
        )
        .await
        .expect("create document link in ws_b");

    let backlinks_a = link_repo
        .backlinks(&ctx_a, target_doc_b.id)
        .await
        .expect("backlinks from ws_a perspective");

    assert!(
        backlinks_a.is_empty(),
        "workspace A must not see workspace B's document links"
    );

    db.teardown().await;
}

#[tokio::test]
async fn attachment_repo_workspace_isolation() {
    use atlas_domain::entities::documents::NewAttachment;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-attach").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-attach").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);

    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 10);
    let attach_repo = PgAttachmentRepo {
        conn: db.conn().clone(),
    };

    let doc_b = doc_repo
        .create(
            &ctx_b,
            NewDocument {
                title: "Attach Host Doc".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create doc in ws_b for attachment");

    let attachment_b = attach_repo
        .record(
            &ctx_b,
            NewAttachment {
                document_id: Some(doc_b.id),
                task_id: None,
                file_name: "photo.png".into(),
                content_type: "image/png".into(),
                size_bytes: 1024,
                sha256: "abc123".into(),
            },
        )
        .await
        .expect("record attachment in ws_b");

    let found = attach_repo
        .find(&ctx_a, attachment_b.id)
        .await
        .expect("find attachment from ws_a perspective");

    assert!(
        found.is_none(),
        "workspace A must not see workspace B's attachment by ID"
    );

    db.teardown().await;
}

#[tokio::test]
async fn membership_repo_workspace_isolation() {
    use atlas_domain::entities::identity::MemberRole;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws_a, user_a) = support::seed_workspace(&db, "alice-member").await;
    let (ws_b, user_b) = support::seed_workspace(&db, "bob-member").await;

    let ctx_a = support::ctx(&ws_a, &user_a);
    let ctx_b = support::ctx(&ws_b, &user_b);

    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };

    let extra_member = db
        .user_repo()
        .create(atlas_server::persistence::repos::NewUser {
            username: "extra-member-isolation".into(),
            display_name: "Extra".into(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$hash".into(),
            is_root: false,
        })
        .await
        .expect("create extra user");

    membership_repo
        .add(&ctx_b, extra_member.id, MemberRole::Member)
        .await
        .expect("add extra user to ws_b");

    let found = membership_repo
        .find(&ctx_a, extra_member.id)
        .await
        .expect("find membership from ws_a perspective");

    assert!(
        found.is_none(),
        "workspace A must not see workspace B's membership"
    );

    db.teardown().await;
}
