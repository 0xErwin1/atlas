//! Integration tests for PgSearchRepo against a live Postgres database.
//!
//! These tests prove the security and correctness contracts that cannot be
//! verified through unit tests alone:
//!
//! 1. Cross-tenant isolation: workspace B's content never leaks to workspace A.
//! 2. Intra-workspace permission scoping: default-deny holds; grants are required.
//! 3. Pagination determinism: both relevance and updated sorts page to exhaustion
//!    with no duplicates or gaps, including tie-breaking by id.
//! 4. Filter predicates: status/tag/type actually narrow results as specified.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::{
    WorkspaceCtx,
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        documents::NewDocument,
        permissions::NewPermissionGrant,
        workspace_core::NewProject,
    },
    ids::{BoardId, DocumentId, ProjectId, UserId, WorkspaceId},
    permissions::{Principal, ResourceRole, Visibility, VisibilityRole},
    ports::search::{SearchAfter, SearchRepo, SortKey},
    search::{SearchQuery, SearchSort, TypeSet},
};
use atlas_server::persistence::repos::{
    BoardRepo, DocumentRepo, FolderRepo, PermissionGrantRepo, PgBoardRepo, PgDocumentRepo,
    PgFolderRepo, PgPermissionGrantRepo, PgProjectRepo, PgSearchRepo, PgTaskRepo, ProjectRepo,
    TaskRepo, UserRepo,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_search_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        filters: vec![],
        sort: SearchSort::Relevance,
        type_filter: TypeSet::all(),
        warnings: vec![],
        prefix: false,
    }
}

fn make_updated_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        filters: vec![],
        sort: SearchSort::UpdatedDesc,
        type_filter: TypeSet::all(),
        warnings: vec![],
        prefix: false,
    }
}

fn make_doc_only_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        filters: vec![],
        sort: SearchSort::Relevance,
        type_filter: TypeSet {
            notes: true,
            tasks: false,
        },
        warnings: vec![],
        prefix: false,
    }
}

fn make_task_only_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        filters: vec![],
        sort: SearchSort::Relevance,
        type_filter: TypeSet {
            notes: false,
            tasks: true,
        },
        warnings: vec![],
        prefix: false,
    }
}

fn make_prefix_query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        filters: vec![],
        sort: SearchSort::Relevance,
        type_filter: TypeSet::all(),
        warnings: vec![],
        prefix: true,
    }
}

async fn seed_doc(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    title: &str,
    content: &str,
) -> DocumentId {
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let doc = repo
        .create(
            ctx,
            NewDocument {
                title: title.to_string(),
                slug: None,
                content: content.to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");
    doc.id
}

async fn seed_project_and_board(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    slug: &str,
    prefix: &str,
) -> (
    atlas_domain::entities::workspace_core::Project,
    atlas_domain::entities::boards_tasks::Board,
    atlas_domain::entities::boards_tasks::BoardColumn,
) {
    let project_repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    let board_repo = PgBoardRepo::new(db.conn().clone());

    let project = project_repo
        .create(
            ctx,
            NewProject {
                name: format!("Project {slug}"),
                slug: slug.to_string(),
                task_prefix: prefix.to_string(),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("seed project");

    let board = board_repo
        .create_board(
            ctx,
            NewBoard {
                folder_id: None,
                project_id: project.id,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("seed board");

    let col = board_repo
        .add_column(
            ctx,
            board.id,
            "Backlog".to_string(),
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
    ctx: &WorkspaceCtx,
    col_id: atlas_domain::ids::ColumnId,
    board_id: BoardId,
    project_id: ProjectId,
    title: &str,
    description: &str,
) -> atlas_domain::ids::TaskId {
    let repo = PgTaskRepo::new(db.conn().clone());
    let task = repo
        .create(
            ctx,
            NewTask {
                column_id: col_id,
                board_id,
                project_id,
                title: title.to_string(),
                description: description.to_string(),
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
        .expect("seed task");
    task.id
}

async fn grant_ws_scope(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    user_id: UserId,
    grantor_id: UserId,
) {
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: Some(user_id),
        api_key_id: None,
        group_id: None,
        project_id: None,
        folder_id: None,
        document_id: None,
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: Some(grantor_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant ws-scope");
}

async fn grant_doc(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    user_id: UserId,
    doc_id: DocumentId,
    grantor_id: UserId,
) {
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: Some(user_id),
        api_key_id: None,
        group_id: None,
        project_id: None,
        folder_id: None,
        document_id: Some(doc_id),
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: Some(grantor_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant document");
}

async fn grant_board(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    user_id: UserId,
    board_id: BoardId,
    grantor_id: UserId,
) {
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: Some(user_id),
        api_key_id: None,
        group_id: None,
        project_id: None,
        folder_id: None,
        document_id: None,
        board_id: Some(board_id),
        role: ResourceRole::Viewer,
        created_by_user_id: Some(grantor_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant board");
}

async fn grant_project(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    user_id: UserId,
    project_id: ProjectId,
    grantor_id: UserId,
) {
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws_id,
        user_id: Some(user_id),
        api_key_id: None,
        group_id: None,
        project_id: Some(project_id),
        folder_id: None,
        document_id: None,
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: Some(grantor_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("grant project");
}

async fn add_member(
    db: &support::TestDb,
    ws_id: WorkspaceId,
    user_id: UserId,
    role: atlas_domain::entities::identity::MemberRole,
) {
    use atlas_server::persistence::repos::MembershipRepo;
    let ctx = atlas_domain::WorkspaceCtx::new(ws_id, atlas_domain::Actor::User(user_id));
    db.membership_repo()
        .add(&ctx, user_id, role)
        .await
        .expect("add member");
}

async fn seed_user(db: &support::TestDb, username: &str) -> UserId {
    let user = db
        .user_repo()
        .create(atlas_server::persistence::repos::NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed user");
    user.id
}

async fn seed_project_with_visibility(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    slug: &str,
    prefix: &str,
    visibility: Visibility,
) -> ProjectId {
    let project_repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    let project = project_repo
        .create(
            ctx,
            NewProject {
                name: format!("Project {slug}"),
                slug: slug.to_string(),
                task_prefix: prefix.to_string(),
                visibility,
            },
        )
        .await
        .expect("seed project");
    project.id
}

async fn seed_doc_in_project(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    project_id: ProjectId,
    title: &str,
    content: &str,
) -> DocumentId {
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let doc = repo
        .create(
            ctx,
            NewDocument {
                title: title.to_string(),
                slug: None,
                content: content.to_string(),
                folder_id: None,
                project_id: Some(project_id),
                frontmatter: None,
            },
        )
        .await
        .expect("seed document in project");
    doc.id
}

// ---------------------------------------------------------------------------
// DISCRIMINATING: a plain workspace Member (not Owner/Admin) must NOT see a
// document in a Private project owned by someone else without an explicit grant.
//
// This mirrors the domain `resolve()` authority (atlas_domain::permissions):
// a member's only access to a Private-project resource is an explicit grant;
// Private visibility contributes nothing (resolve() Rule 2, Visibility::Private
// match arm yields no candidate).
//
// This test is driven at the repo level on purpose: a plain member with no
// workspace-scope grant resolves to None on the workspace-only chain, so the
// route gate would return 404 before the SQL row filter ever runs. Testing the
// row filter directly is what proves the SQL mirrors resolve(); the gate
// behavior is covered separately by the HTTP tests.
//
// It would go RED against the previous over-broad `EXISTS membership` clause,
// which returned EVERY workspace row to any member regardless of visibility.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plain_member_does_not_see_private_project_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-priv-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let member_id = seed_user(&db, "srch-priv-member").await;
    add_member(
        &db,
        ws.id,
        member_id,
        atlas_domain::entities::identity::MemberRole::Member,
    )
    .await;

    let private_project = seed_project_with_visibility(
        &db,
        &ctx_owner,
        "srch-priv-proj",
        "SPP",
        Visibility::Private,
    )
    .await;

    let doc_id = seed_doc_in_project(
        &db,
        &ctx_owner,
        private_project,
        "Private Project Doc",
        "uniquetoken_privproj",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let member_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(member_id));
    let hits = repo
        .search(
            &member_ctx,
            &Principal::User(member_id),
            &make_search_query("uniquetoken_privproj"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("member search");

    assert!(
        !hits.iter().any(|h| h.id == doc_id.0),
        "a plain member must NOT see a Private-project document without a grant; got: {hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// A plain member WITH an explicit project grant on the Private project sees its
// document (resolve() Rule 2: explicit grant on the project segment).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plain_member_with_project_grant_sees_private_project_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-privgrant-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let member_id = seed_user(&db, "srch-privgrant-member").await;
    add_member(
        &db,
        ws.id,
        member_id,
        atlas_domain::entities::identity::MemberRole::Member,
    )
    .await;

    let private_project = seed_project_with_visibility(
        &db,
        &ctx_owner,
        "srch-privgrant-proj",
        "SPG",
        Visibility::Private,
    )
    .await;

    let doc_id = seed_doc_in_project(
        &db,
        &ctx_owner,
        private_project,
        "Granted Private Doc",
        "uniquetoken_privgrant",
    )
    .await;

    grant_project(&db, ws.id, member_id, private_project, owner.id).await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let member_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(member_id));
    let hits = repo
        .search(
            &member_ctx,
            &Principal::User(member_id),
            &make_search_query("uniquetoken_privgrant"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("member search");

    assert!(
        hits.iter().any(|h| h.id == doc_id.0),
        "a plain member WITH a project grant must see the Private-project document; got: {hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// A plain member sees a non-Private (Workspace-visibility) project document
// without any grant (resolve() Rule 2: visibility contribution for members).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plain_member_sees_workspace_visible_project_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-wsvis-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let member_id = seed_user(&db, "srch-wsvis-member").await;
    add_member(
        &db,
        ws.id,
        member_id,
        atlas_domain::entities::identity::MemberRole::Member,
    )
    .await;

    let visible_project = seed_project_with_visibility(
        &db,
        &ctx_owner,
        "srch-wsvis-proj",
        "SWV",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    let doc_id = seed_doc_in_project(
        &db,
        &ctx_owner,
        visible_project,
        "Workspace Visible Doc",
        "uniquetoken_wsvis",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let member_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(member_id));
    let hits = repo
        .search(
            &member_ctx,
            &Principal::User(member_id),
            &make_search_query("uniquetoken_wsvis"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("member search");

    assert!(
        hits.iter().any(|h| h.id == doc_id.0),
        "a plain member must see a Workspace-visibility project document; got: {hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// An Owner/Admin member sees a Private-project document with no grant
// (resolve() Rule 1: implicit admin sees everything).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn owner_sees_private_project_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-ownerpriv-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let private_project = seed_project_with_visibility(
        &db,
        &ctx_owner,
        "srch-ownerpriv-proj",
        "SOP",
        Visibility::Private,
    )
    .await;

    let doc_id = seed_doc_in_project(
        &db,
        &ctx_owner,
        private_project,
        "Owner Private Doc",
        "uniquetoken_ownerpriv",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let hits = repo
        .search(
            &ctx_owner,
            &Principal::User(owner.id),
            &make_search_query("uniquetoken_ownerpriv"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("owner search");

    assert!(
        hits.iter().any(|h| h.id == doc_id.0),
        "an Owner must see a Private-project document via implicit admin; got: {hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// A plain member must NOT see a task in a Private project without a grant, and
// MUST see one in a non-Private project (tasks arm visibility contribution).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plain_member_task_visibility_follows_project() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-taskvis-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let member_id = seed_user(&db, "srch-taskvis-member").await;
    add_member(
        &db,
        ws.id,
        member_id,
        atlas_domain::entities::identity::MemberRole::Member,
    )
    .await;

    // Private project task (member must NOT see it).
    let private_project = seed_project_with_visibility(
        &db,
        &ctx_owner,
        "srch-taskvis-priv",
        "STVP",
        Visibility::Private,
    )
    .await;
    let board_repo = PgBoardRepo::new(db.conn().clone());
    let private_board = board_repo
        .create_board(
            &ctx_owner,
            NewBoard {
                folder_id: None,
                project_id: private_project,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("private board");
    let private_col = board_repo
        .add_column(
            &ctx_owner,
            private_board.id,
            "Backlog".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("private column");
    let private_task = seed_task(
        &db,
        &ctx_owner,
        private_col.id,
        private_board.id,
        private_project,
        "Private Task",
        "uniquetoken_taskvis",
    )
    .await;

    // Workspace-visible project task (member MUST see it).
    let (vis_project, vis_board, vis_col) =
        seed_project_and_board(&db, &ctx_owner, "srch-taskvis-vis", "STVV").await;
    let vis_task = seed_task(
        &db,
        &ctx_owner,
        vis_col.id,
        vis_board.id,
        vis_project.id,
        "Visible Task",
        "uniquetoken_taskvis",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let member_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(member_id));
    let hits = repo
        .search(
            &member_ctx,
            &Principal::User(member_id),
            &make_task_only_query("uniquetoken_taskvis"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("member task search");

    let ids: Vec<uuid::Uuid> = hits.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&vis_task.0),
        "member must see a task in a Workspace-visibility project; got: {ids:?}"
    );
    assert!(
        !ids.contains(&private_task.0),
        "member must NOT see a task in a Private project without a grant; got: {ids:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// REQ-7: the snippet is computed by the two-stage headline and carries the
// <mark>...</mark> highlight around the matched term. This guards the headline
// restructure (ts_headline moved out of the UNION arms to the outer page query).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn snippet_carries_mark_highlight() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-headline-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let doc_id = seed_doc(
        &db,
        &ctx_owner,
        "Headline Doc",
        "the quick uniquetoken_headline brown fox jumps over the lazy dog",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let hits = repo
        .search(
            &ctx_owner,
            &Principal::User(owner.id),
            &make_search_query("uniquetoken_headline"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("owner search");

    let hit = hits
        .iter()
        .find(|h| h.id == doc_id.0)
        .expect("doc must surface");
    let snippet = hit.snippet.as_deref().expect("snippet must be present");
    assert!(
        snippet.contains("<mark>") && snippet.contains("</mark>"),
        "snippet must carry the <mark> highlight; got: {snippet:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Soft-deleted project consistency: a live task whose project was soft-deleted
// must still surface for an Owner, with a NULL project_slug. The tasks arm uses
// a LEFT JOIN on projects (matching the documents arm and the task-list
// endpoint, which never drops a task when its project is soft-deleted).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_of_soft_deleted_project_still_surfaces_for_owner() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-softdel-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let (project, board, col) =
        seed_project_and_board(&db, &ctx_owner, "srch-softdel-proj", "SSD").await;

    let task_id = seed_task(
        &db,
        &ctx_owner,
        col.id,
        board.id,
        project.id,
        "Soft Deleted Project Task",
        "uniquetoken_softdel",
    )
    .await;

    let project_repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    project_repo
        .soft_delete(&ctx_owner, project.id)
        .await
        .expect("soft delete project");

    let repo = PgSearchRepo::new(db.conn().clone());
    let hits = repo
        .search(
            &ctx_owner,
            &Principal::User(owner.id),
            &make_task_only_query("uniquetoken_softdel"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("owner search");

    let hit = hits
        .iter()
        .find(|h| h.id == task_id.0)
        .expect("task of a soft-deleted project must still surface");
    assert!(
        hit.project_slug.is_none(),
        "a task whose project was soft-deleted must carry a NULL project_slug; got: {:?}",
        hit.project_slug
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 1: cross-tenant isolation — documents in workspace B never appear for
// a principal in workspace A, even when searching the same text.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cross_tenant_document_isolation() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    // workspace A: alice is owner (member), so she can see her own docs.
    let (ws_a, alice) = support::seed_workspace(&db, "srch-xten-alice").await;
    let ctx_a = support::ctx(&ws_a, &alice);

    // workspace B: bob is owner, has a doc with matching text.
    let (ws_b, bob) = support::seed_workspace(&db, "srch-xten-bob").await;
    let ctx_b = support::ctx(&ws_b, &bob);

    seed_doc(
        &db,
        &ctx_b,
        "Cross Tenant Secret",
        "uniquetoken_xten_secret",
    )
    .await;
    let alice_doc_id = seed_doc(&db, &ctx_a, "Alice Own Doc", "uniquetoken_xten_secret").await;

    // FTS index is GENERATED ALWAYS STORED, so it updates on insert.
    // Wait for any in-flight WAL flush (not needed for STORED, but be safe).
    let repo = PgSearchRepo::new(db.conn().clone());
    let ctx = atlas_domain::WorkspaceCtx::new(ws_a.id, atlas_domain::Actor::User(alice.id));
    let principal = Principal::User(alice.id);
    let query = make_search_query("uniquetoken_xten_secret");

    let hits = repo
        .search(&ctx, &principal, &query, 50, None, false, true, true)
        .await
        .expect("search");

    let ids: Vec<uuid::Uuid> = hits.iter().map(|h| h.id).collect();

    assert!(
        ids.contains(&alice_doc_id.0),
        "alice must see her own document"
    );
    assert!(
        !ids.iter().any(|id| {
            // bob's doc is in ws_b; alice searching ws_a must not see it
            // We can check by workspace: all returned hits must be in ws_a.
            // Since we can't directly get workspace_id from SearchHit, we
            // verify indirectly: the only doc with this text in ws_a is alice_doc_id.
            // Any extra hit would be from ws_b.
            *id != alice_doc_id.0
        }),
        "alice must not see workspace B documents; extra hits: {ids:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 2: intra-workspace permission scoping — a workspace member without a
// grant on a private document must not see it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn intra_workspace_no_grant_excludes_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-perm-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    // Create a second user, but do NOT add them to the workspace.
    // A non-member cannot see any docs through membership.
    let user_repo = db.user_repo();
    let stranger = user_repo
        .create(atlas_server::persistence::repos::NewUser {
            username: "srch-perm-stranger".to_string(),
            display_name: "Stranger".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed stranger");

    // Seed a doc visible to the owner (workspace member).
    let _doc_id = seed_doc(
        &db,
        &ctx_owner,
        "Private Document",
        "uniquetoken_perm_private",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());

    // Owner (workspace member) can see the doc.
    let owner_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(owner.id));
    let hits = repo
        .search(
            &owner_ctx,
            &Principal::User(owner.id),
            &make_search_query("uniquetoken_perm_private"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("owner search");
    assert!(!hits.is_empty(), "owner must see the document");

    // Stranger (not a workspace member, no grant) cannot see the doc.
    let stranger_ctx =
        atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(stranger.id));
    let stranger_hits = repo
        .search(
            &stranger_ctx,
            &Principal::User(stranger.id),
            &make_search_query("uniquetoken_perm_private"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("stranger search");
    assert!(
        stranger_hits.is_empty(),
        "non-member without grant must not see any documents; got: {stranger_hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 3: a direct-grant on a specific document surfaces it to a non-member.
// (proves the document-grant branch of the permission disjunction)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn direct_document_grant_surfaces_hit() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-docgrant-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let user_repo = db.user_repo();
    let grantee = user_repo
        .create(atlas_server::persistence::repos::NewUser {
            username: "srch-docgrant-grantee".to_string(),
            display_name: "Grantee".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed grantee");

    let doc_id = seed_doc(&db, &ctx_owner, "Granted Document", "uniquetoken_docgrant").await;

    // Grant grantee access to this specific document.
    grant_doc(&db, ws.id, grantee.id, doc_id, owner.id).await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let grantee_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(grantee.id));
    let hits = repo
        .search(
            &grantee_ctx,
            &Principal::User(grantee.id),
            &make_search_query("uniquetoken_docgrant"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("grantee search");

    assert!(
        hits.iter().any(|h| h.id == doc_id.0),
        "grantee with direct document grant must see the document"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 4: task visibility — board-grant surfaces a task; non-member without
// grant sees nothing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_board_grant_surfaces_hit() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-board-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let user_repo = db.user_repo();
    let grantee = user_repo
        .create(atlas_server::persistence::repos::NewUser {
            username: "srch-board-grantee".to_string(),
            display_name: "BoardGrantee".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed grantee");
    let stranger = user_repo
        .create(atlas_server::persistence::repos::NewUser {
            username: "srch-board-stranger".to_string(),
            display_name: "BoardStranger".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed stranger");

    let (project, board, col) =
        seed_project_and_board(&db, &ctx_owner, "srch-board-proj", "SBP").await;

    let task_id = seed_task(
        &db,
        &ctx_owner,
        col.id,
        board.id,
        project.id,
        "Board Task",
        "uniquetoken_boardtask",
    )
    .await;

    // Grant grantee access to the board.
    grant_board(&db, ws.id, grantee.id, board.id, owner.id).await;

    let repo = PgSearchRepo::new(db.conn().clone());

    // Grantee (board-level grant) can see the task.
    let grantee_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(grantee.id));
    let grantee_hits = repo
        .search(
            &grantee_ctx,
            &Principal::User(grantee.id),
            &make_search_query("uniquetoken_boardtask"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("grantee search");
    assert!(
        grantee_hits.iter().any(|h| h.id == task_id.0),
        "grantee with board grant must see the task"
    );

    // Stranger (no grant, not a member) cannot see the task.
    let stranger_ctx =
        atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(stranger.id));
    let stranger_hits = repo
        .search(
            &stranger_ctx,
            &Principal::User(stranger.id),
            &make_search_query("uniquetoken_boardtask"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("stranger search");
    assert!(
        stranger_hits.is_empty(),
        "non-member without grant must not see any tasks"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 5: workspace-scope grant surfaces ALL docs in the workspace.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn workspace_scope_grant_surfaces_all_documents() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-wsscope-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let user_repo = db.user_repo();
    let grantee = user_repo
        .create(atlas_server::persistence::repos::NewUser {
            username: "srch-wsscope-grantee".to_string(),
            display_name: "WsScopeGrantee".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed grantee");

    let doc_id1 = seed_doc(&db, &ctx_owner, "WsScope Doc A", "uniquetoken_wsscope").await;
    let doc_id2 = seed_doc(&db, &ctx_owner, "WsScope Doc B", "uniquetoken_wsscope").await;

    // Grant grantee a workspace-scope grant (project/folder/doc/board all NULL).
    grant_ws_scope(&db, ws.id, grantee.id, owner.id).await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let grantee_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(grantee.id));
    let hits = repo
        .search(
            &grantee_ctx,
            &Principal::User(grantee.id),
            &make_search_query("uniquetoken_wsscope"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("grantee search");

    let ids: Vec<uuid::Uuid> = hits.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&doc_id1.0) && ids.contains(&doc_id2.0),
        "workspace-scope grant must surface all docs; got: {ids:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 6: pagination determinism — relevance sort, no duplicates or gaps.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pagination_relevance_no_duplicates_or_gaps() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-page-rel-owner").await;
    let ctx = support::ctx(&ws, &owner);

    // Seed 5 documents with unique content tokens so they each match.
    let mut expected_ids = Vec::new();
    for i in 0..5_usize {
        let doc_id = seed_doc(
            &db,
            &ctx,
            &format!("PagRelTitle uniquetoken_pagrel {i}"),
            &format!("PagRelBody uniquetoken_pagrel body content {i}"),
        )
        .await;
        expected_ids.push(doc_id.0);
    }

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);
    let query = make_search_query("uniquetoken_pagrel");

    // Page through with limit=2.
    let mut all_ids = Vec::new();
    let mut after: Option<SearchAfter> = None;

    loop {
        let hits = repo
            .search(&ctx, &principal, &query, 2 + 1, after, false, true, true)
            .await
            .expect("search page");

        let has_more = hits.len() > 2;
        let page: Vec<_> = hits.into_iter().take(2).collect();

        if let Some(last) = page.last() {
            after = Some(SearchAfter {
                key: SortKey::Relevance(last.score),
                id: last.id,
            });
        }

        all_ids.extend(page.iter().map(|h| h.id));

        if !has_more {
            break;
        }
    }

    // Every expected doc must appear exactly once.
    for expected in &expected_ids {
        let count = all_ids.iter().filter(|id| *id == expected).count();
        assert_eq!(
            count, 1,
            "document {expected} appeared {count} times (expected exactly once)"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 7: pagination determinism — updated sort, no duplicates or gaps.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pagination_updated_no_duplicates_or_gaps() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-page-upd-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let mut expected_ids = Vec::new();
    for i in 0..5_usize {
        let doc_id = seed_doc(
            &db,
            &ctx,
            &format!("PagUpdTitle uniquetoken_pagupdated {i}"),
            &format!("PagUpdBody content {i}"),
        )
        .await;
        expected_ids.push(doc_id.0);
    }

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);
    let query = make_updated_query("uniquetoken_pagupdated");

    let mut all_ids = Vec::new();
    let mut after: Option<SearchAfter> = None;

    loop {
        let hits = repo
            .search(&ctx, &principal, &query, 2 + 1, after, false, true, true)
            .await
            .expect("search page");

        let has_more = hits.len() > 2;
        let page: Vec<_> = hits.into_iter().take(2).collect();

        if let Some(last) = page.last() {
            after = Some(SearchAfter {
                key: SortKey::Updated(last.updated_at.timestamp_micros()),
                id: last.id,
            });
        }

        all_ids.extend(page.iter().map(|h| h.id));

        if !has_more {
            break;
        }
    }

    for expected in &expected_ids {
        let count = all_ids.iter().filter(|id| *id == expected).count();
        assert_eq!(
            count, 1,
            "document {expected} appeared {count} times in updated pagination (expected once)"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 8: type filter — Documents only excludes tasks.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn type_filter_documents_excludes_tasks() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-typef-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let (project, board, col) = seed_project_and_board(&db, &ctx, "srch-typef-proj", "STP").await;

    let doc_id = seed_doc(&db, &ctx, "TypeFilter Doc", "uniquetoken_typef").await;
    let task_id = seed_task(
        &db,
        &ctx,
        col.id,
        board.id,
        project.id,
        "TypeFilter Task",
        "uniquetoken_typef",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);

    // type=Documents: must include doc, must exclude task.
    let doc_hits = repo
        .search(
            &ctx,
            &principal,
            &make_doc_only_query("uniquetoken_typef"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("doc-only search");
    assert!(
        doc_hits.iter().any(|h| h.id == doc_id.0),
        "document must appear in type=Documents search"
    );
    assert!(
        !doc_hits.iter().any(|h| h.id == task_id.0),
        "task must NOT appear in type=Documents search"
    );

    // type=Tasks: must include task, must exclude doc.
    let task_hits = repo
        .search(
            &ctx,
            &principal,
            &make_task_only_query("uniquetoken_typef"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("task-only search");
    assert!(
        task_hits.iter().any(|h| h.id == task_id.0),
        "task must appear in type=Tasks search"
    );
    assert!(
        !task_hits.iter().any(|h| h.id == doc_id.0),
        "document must NOT appear in type=Tasks search"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 9: status filter narrows tasks by board column name.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn status_filter_narrows_tasks_by_column_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-statusf-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let (project, board, col_backlog) =
        seed_project_and_board(&db, &ctx, "srch-statusf-proj", "SSP").await;

    // Add a second column "Done".
    let board_repo = PgBoardRepo::new(db.conn().clone());
    let col_done = board_repo
        .add_column(
            &ctx,
            board.id,
            "Done".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("seed done column");

    let task_backlog_id = seed_task(
        &db,
        &ctx,
        col_backlog.id,
        board.id,
        project.id,
        "Backlog Task",
        "uniquetoken_statusf",
    )
    .await;
    let task_done_id = seed_task(
        &db,
        &ctx,
        col_done.id,
        board.id,
        project.id,
        "Done Task",
        "uniquetoken_statusf",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);

    let mut status_query = make_task_only_query("uniquetoken_statusf");
    status_query
        .filters
        .push(atlas_domain::search::SearchFilter::Status(
            "Backlog".to_string(),
        ));

    let hits = repo
        .search(&ctx, &principal, &status_query, 50, None, false, true, true)
        .await
        .expect("status search");

    let ids: Vec<uuid::Uuid> = hits.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&task_backlog_id.0),
        "Backlog task must appear when status:Backlog"
    );
    assert!(
        !ids.contains(&task_done_id.0),
        "Done task must NOT appear when status:Backlog"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 10: tag filter narrows documents (frontmatter) and tasks (labels).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tag_filter_narrows_documents_and_tasks() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-tagf-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let (project, board, col) = seed_project_and_board(&db, &ctx, "srch-tagf-proj", "TGP").await;

    // Seed a doc with frontmatter tags.
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let doc_tagged = doc_repo
        .create(
            &ctx,
            NewDocument {
                title: "Tagged Doc".to_string(),
                slug: None,
                content: "uniquetoken_tagf content".to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: Some(serde_json::json!({"tags": ["rust"]})),
            },
        )
        .await
        .expect("seed tagged doc");

    let doc_untagged = doc_repo
        .create(
            &ctx,
            NewDocument {
                title: "Untagged Doc".to_string(),
                slug: None,
                content: "uniquetoken_tagf content".to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed untagged doc");

    // Seed a task with labels.
    let task_repo = PgTaskRepo::new(db.conn().clone());
    let task_tagged = task_repo
        .create(
            &ctx,
            NewTask {
                column_id: col.id,
                board_id: board.id,
                project_id: project.id,
                title: "Tagged Task".to_string(),
                description: "uniquetoken_tagf task description".to_string(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec!["rust".to_string()],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("seed tagged task");

    let task_untagged = task_repo
        .create(
            &ctx,
            NewTask {
                column_id: col.id,
                board_id: board.id,
                project_id: project.id,
                title: "Untagged Task".to_string(),
                description: "uniquetoken_tagf task description".to_string(),
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
        .expect("seed untagged task");

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);

    let mut tag_query = make_search_query("uniquetoken_tagf");
    tag_query
        .filters
        .push(atlas_domain::search::SearchFilter::Tag("rust".to_string()));

    let hits = repo
        .search(&ctx, &principal, &tag_query, 50, None, false, true, true)
        .await
        .expect("tag search");

    let ids: Vec<uuid::Uuid> = hits.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&doc_tagged.id.0),
        "tagged document must appear with tag:rust"
    );
    assert!(
        !ids.contains(&doc_untagged.id.0),
        "untagged document must NOT appear with tag:rust"
    );
    assert!(
        ids.contains(&task_tagged.id.0),
        "tagged task must appear with tag:rust"
    );
    assert!(
        !ids.contains(&task_untagged.id.0),
        "untagged task must NOT appear with tag:rust"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// REQ-7: a title-only match must produce an ABSENT snippet (None). When the
// search term appears only in the title — not in the body — ts_headline returns
// a leading body fragment with no <mark> markers. The spec contract requires
// snippet = None in that case; the wire response must omit the field entirely.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn title_only_match_yields_absent_snippet() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-titlesnip-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    // Term is in the title only; body is intentionally empty so ts_headline
    // has no fragment to highlight. The hit must still be returned (title
    // match), but the snippet field must be None.
    let doc_id = seed_doc(&db, &ctx_owner, "uniquetoken_titlesnip9xqz Document", "").await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let hits = repo
        .search(
            &ctx_owner,
            &Principal::User(owner.id),
            &make_search_query("uniquetoken_titlesnip9xqz"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("owner search");

    let hit = hits
        .iter()
        .find(|h| h.id == doc_id.0)
        .expect("doc must surface via title match");

    assert!(
        hit.snippet.is_none(),
        "title-only match must yield an absent snippet; got: {:?}",
        hit.snippet
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Test 11: readable_id is populated for tasks and absent for documents.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_hits_carry_readable_id_documents_do_not() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-rid-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let (project, board, col) = seed_project_and_board(&db, &ctx, "srch-rid-proj", "RID").await;

    let _doc_id = seed_doc(&db, &ctx, "Rid Doc", "uniquetoken_rid").await;
    let _task_id = seed_task(
        &db,
        &ctx,
        col.id,
        board.id,
        project.id,
        "Rid Task",
        "uniquetoken_rid",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);
    let hits = repo
        .search(
            &ctx,
            &principal,
            &make_search_query("uniquetoken_rid"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("search");

    for hit in &hits {
        match hit.kind {
            atlas_domain::search::SearchKind::Task => {
                assert!(
                    hit.readable_id.is_some(),
                    "task hit must have readable_id; id={}",
                    hit.id
                );
            }
            atlas_domain::search::SearchKind::Document => {
                assert!(
                    hit.readable_id.is_none(),
                    "document hit must NOT have readable_id; id={}",
                    hit.id
                );
            }
        }
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// REQ-3 member visibility — Fix 2(a)
//
// A plain MemberRole::Member (no explicit grant) who has access to a folder
// inside a NON-Private (Workspace-visibility) project MUST see the document
// in that folder via the visibility contribution (p.visibility <> 'private').
// The document carries `project_id` from its folder's project, so the LEFT
// JOIN resolves `p.visibility` and the member clause fires.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn member_sees_in_folder_non_private_project_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-foldvis-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let member_id = seed_user(&db, "srch-foldvis-member").await;
    add_member(
        &db,
        ws.id,
        member_id,
        atlas_domain::entities::identity::MemberRole::Member,
    )
    .await;

    let visible_project_id = seed_project_with_visibility(
        &db,
        &ctx_owner,
        "srch-foldvis-proj",
        "SFV",
        Visibility::Workspace(VisibilityRole::Editor),
    )
    .await;

    let folder_repo = PgFolderRepo {
        conn: db.conn().clone(),
    };
    let folder = folder_repo
        .create(
            &ctx_owner,
            atlas_domain::entities::workspace_core::NewFolder {
                project_id: Some(visible_project_id),
                parent_folder_id: None,
                name: "srch-foldvis-folder".to_string(),
            },
        )
        .await
        .expect("seed folder");

    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let doc = doc_repo
        .create(
            &ctx_owner,
            atlas_domain::entities::documents::NewDocument {
                title: "Folder Doc".to_string(),
                slug: None,
                content: "uniquetoken_foldvis".to_string(),
                folder_id: Some(folder.id),
                project_id: Some(visible_project_id),
                frontmatter: None,
            },
        )
        .await
        .expect("seed doc in folder");

    let repo = PgSearchRepo::new(db.conn().clone());
    let member_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(member_id));
    let hits = repo
        .search(
            &member_ctx,
            &Principal::User(member_id),
            &make_search_query("uniquetoken_foldvis"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("member search");

    assert!(
        hits.iter().any(|h| h.id == doc.id.0),
        "plain member must see a document in a folder of a non-Private project via visibility contribution; got: {hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// D-SEARCHHIT: column_name is Some for task hits and None for document hits.
//
// This is the UNION ALL shape regression guard: if either arm omits the
// column_name projection at the correct ordinal, or the inner/outer page
// subqueries drop it, the field silently binds as NULL for all rows (a
// mis-bind, not an error). The test forces a distinct value so a mis-bind
// is caught immediately.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn task_hit_carries_column_name_document_hit_does_not() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-colname-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let column_name = "In Progress";

    let (project, board, _default_col) =
        seed_project_and_board(&db, &ctx, "srch-colname-proj", "SCN").await;

    let board_repo = PgBoardRepo::new(db.conn().clone());
    let named_col = board_repo
        .add_column(
            &ctx,
            board.id,
            column_name.to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("seed named column");

    let task_id = seed_task(
        &db,
        &ctx,
        named_col.id,
        board.id,
        project.id,
        "Column Name Task",
        "uniquetoken_colname",
    )
    .await;

    let _doc_id = seed_doc(&db, &ctx, "Column Name Doc", "uniquetoken_colname").await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);
    let hits = repo
        .search(
            &ctx,
            &principal,
            &make_search_query("uniquetoken_colname"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("search");

    let task_hit = hits
        .iter()
        .find(|h| h.id == task_id.0)
        .expect("task hit must be present");

    assert_eq!(
        task_hit.column_name.as_deref(),
        Some(column_name),
        "task hit must carry column_name equal to the seeded column name; got: {:?}",
        task_hit.column_name
    );

    for hit in &hits {
        if hit.id != task_id.0 {
            assert!(
                hit.column_name.is_none(),
                "document hit must have column_name = None; id={} got: {:?}",
                hit.id,
                hit.column_name
            );
        }
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// REQ-3 member visibility — Fix 2(b)
//
// A plain MemberRole::Member (no explicit grant) must NOT see a document that
// has no project (project_id IS NULL). The visibility contribution clause
// requires `p.id IS NOT NULL` (the LEFT JOIN must resolve a project row),
// so workspace-root documents are grants-only.
//
// `seed_doc` creates documents with project_id = None, which models this
// "workspace-root document" case at the repo level.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn member_gets_nothing_via_visibility_for_workspace_root_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-rootdoc-owner").await;
    let ctx_owner = support::ctx(&ws, &owner);

    let member_id = seed_user(&db, "srch-rootdoc-member").await;
    add_member(
        &db,
        ws.id,
        member_id,
        atlas_domain::entities::identity::MemberRole::Member,
    )
    .await;

    // Workspace-root document: project_id = None, folder_id = None.
    let doc_id = seed_doc(
        &db,
        &ctx_owner,
        "Root Doc No Project",
        "uniquetoken_rootdoc9qzx",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let member_ctx = atlas_domain::WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(member_id));
    let hits = repo
        .search(
            &member_ctx,
            &Principal::User(member_id),
            &make_search_query("uniquetoken_rootdoc9qzx"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("member search");

    assert!(
        !hits.iter().any(|h| h.id == doc_id.0),
        "plain member must NOT see a workspace-root document (no project) via visibility; got: {hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Opt-in prefix matching — the reference picker's typeahead.
//
// A partial word ("atl") must match a task containing "atlas" when prefix=true,
// and the match must be workspace-wide: the matching task lives on a DIFFERENT
// board/project than the "current" board, proving cross-board reach. Without
// prefix the SAME partial word must NOT match (whole-word behaviour preserved).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn prefix_search_matches_partial_word_across_boards() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-prefix-owner").await;
    let ctx = support::ctx(&ws, &owner);

    // "current" board — the one the user is looking at; its task does NOT carry
    // the target word, so a hit can only come from the OTHER board.
    let (cur_project, cur_board, cur_col) =
        seed_project_and_board(&db, &ctx, "srch-prefix-current", "SPC").await;
    let _current_task = seed_task(
        &db,
        &ctx,
        cur_col.id,
        cur_board.id,
        cur_project.id,
        "Current Board Task",
        "unrelated body text",
    )
    .await;

    // other board — holds a task whose description contains "atlas".
    let (other_project, other_board, other_col) =
        seed_project_and_board(&db, &ctx, "srch-prefix-other", "SPO").await;
    let atlas_task = seed_task(
        &db,
        &ctx,
        other_col.id,
        other_board.id,
        other_project.id,
        "Other Board Task",
        "this references the atlas project plan",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);

    // With prefix=true: "atl" matches "atlas" on the OTHER board.
    let prefix_hits = repo
        .search(
            &ctx,
            &principal,
            &make_prefix_query("atl"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("prefix search");
    assert!(
        prefix_hits.iter().any(|h| h.id == atlas_task.0),
        "prefix search for 'atl' must match the cross-board task containing 'atlas'; got: {prefix_hits:?}"
    );

    // Without prefix: "atl" is a whole word and must NOT match "atlas".
    let whole_word_hits = repo
        .search(
            &ctx,
            &principal,
            &make_search_query("atl"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("whole-word search");
    assert!(
        !whole_word_hits.iter().any(|h| h.id == atlas_task.0),
        "whole-word search for 'atl' must NOT match 'atlas'; got: {whole_word_hits:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// A code-like query (e.g. ATL-10) resolves a task by its readable_id, which the
// FTS vector does not index. The title/description deliberately omit the code,
// so a hit can only come from the readable_id match.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_by_task_code_matches_readable_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "srch-code-owner").await;
    let ctx = support::ctx(&ws, &owner);

    let (project, board, col) = seed_project_and_board(&db, &ctx, "srch-code", "COD").await;
    let coded_task = seed_task(
        &db,
        &ctx,
        col.id,
        board.id,
        project.id,
        "Refactor the storage layer",
        "no code mentioned in the body",
    )
    .await;

    let repo = PgSearchRepo::new(db.conn().clone());
    let principal = Principal::User(owner.id);

    // The first task of a fresh project carries readable_id "COD-1".
    let exact_hits = repo
        .search(
            &ctx,
            &principal,
            &make_search_query("COD-1"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("code search");
    assert!(
        exact_hits.iter().any(|h| h.id == coded_task.0),
        "search for 'COD-1' must resolve the task by readable_id; got: {exact_hits:?}"
    );

    // Case-insensitive prefix on the code resolves it too.
    let prefix_hits = repo
        .search(
            &ctx,
            &principal,
            &make_search_query("cod-"),
            50,
            None,
            false,
            true,
            true,
        )
        .await
        .expect("code prefix search");
    assert!(
        prefix_hits.iter().any(|h| h.id == coded_task.0),
        "search for 'cod-' must resolve the task by readable_id (case-insensitive prefix); got: {prefix_hits:?}"
    );

    db.teardown().await;
}
