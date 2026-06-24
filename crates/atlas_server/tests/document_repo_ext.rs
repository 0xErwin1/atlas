#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{entities::documents::NewDocument, permissions::Principal};
use atlas_server::persistence::repos::{DocumentRepo, PgDocumentRepo};

fn make_doc_repo(db: &support::TestDb) -> PgDocumentRepo {
    PgDocumentRepo::new(db.conn().clone(), 50)
}

fn user_principal(user: &atlas_server::persistence::repos::User) -> Principal {
    Principal::User(user.id)
}

async fn create_doc(
    repo: &PgDocumentRepo,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    title: &str,
    slug: Option<&str>,
) -> atlas_domain::entities::documents::Document {
    let ctx = support::ctx(ws, user);
    repo.create(
        &ctx,
        NewDocument {
            title: title.into(),
            slug: slug.map(str::to_string),
            content: "".into(),
            folder_id: None,
            project_id: None,
            frontmatter: None,
        },
    )
    .await
    .expect("create doc")
}

// --- list_visible ---

#[tokio::test]
async fn list_visible_returns_workspace_docs_for_member() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "lv-basic").await;
    let repo = make_doc_repo(&db);
    let ctx = support::ctx(&ws, &user);

    create_doc(&repo, &ws, &user, "Doc A", Some("doc-a")).await;
    create_doc(&repo, &ws, &user, "Doc B", Some("doc-b")).await;

    let principal = user_principal(&user);
    let docs = repo
        .list_visible(&ctx, &principal, None, None, 10)
        .await
        .expect("list_visible");

    assert_eq!(docs.len(), 2, "member must see all workspace docs");

    db.teardown().await;
}

#[tokio::test]
async fn list_visible_excludes_other_workspace_docs() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws1, user1) = support::seed_workspace(&db, "lv-tenant1").await;
    let (ws2, user2) = support::seed_workspace(&db, "lv-tenant2").await;

    let repo = make_doc_repo(&db);
    create_doc(&repo, &ws1, &user1, "WS1 Doc", Some("ws1-doc")).await;
    create_doc(&repo, &ws2, &user2, "WS2 Doc", Some("ws2-doc")).await;

    let ctx1 = support::ctx(&ws1, &user1);
    let principal1 = user_principal(&user1);
    let docs = repo
        .list_visible(&ctx1, &principal1, None, None, 10)
        .await
        .expect("list_visible ws1");

    assert_eq!(docs.len(), 1, "workspace 1 must not see workspace 2 docs");
    assert_eq!(docs[0].title, "WS1 Doc");

    db.teardown().await;
}

#[tokio::test]
async fn list_visible_cursor_pagination_works() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "lv-cursor").await;
    let repo = make_doc_repo(&db);

    create_doc(&repo, &ws, &user, "Doc 1", Some("doc-1")).await;
    create_doc(&repo, &ws, &user, "Doc 2", Some("doc-2")).await;
    create_doc(&repo, &ws, &user, "Doc 3", Some("doc-3")).await;

    let ctx = support::ctx(&ws, &user);
    let principal = user_principal(&user);

    let page1 = repo
        .list_visible(&ctx, &principal, None, None, 2)
        .await
        .expect("page1");
    assert_eq!(page1.len(), 2, "first page must have 2 docs");

    let cursor = page1.last().map(|d| d.id.0);
    let page2 = repo
        .list_visible(&ctx, &principal, None, cursor, 2)
        .await
        .expect("page2");
    assert_eq!(page2.len(), 1, "second page must have the remaining doc");

    db.teardown().await;
}

// --- list_visible: scope grants for a non-member principal ---

async fn create_folder(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    project_id: Option<atlas_domain::ids::ProjectId>,
    parent: Option<atlas_domain::ids::FolderId>,
) -> atlas_domain::entities::workspace_core::Folder {
    use atlas_server::persistence::repos::{FolderRepo, PgFolderRepo};
    let repo = PgFolderRepo {
        conn: db.conn().clone(),
    };
    let ctx = support::ctx(ws, user);
    repo.create(
        &ctx,
        atlas_domain::entities::workspace_core::NewFolder {
            project_id,
            parent_folder_id: parent,
            name: "scope-folder".into(),
        },
    )
    .await
    .expect("create folder")
}

async fn create_doc_in_folder(
    repo: &PgDocumentRepo,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    title: &str,
    folder_id: Option<atlas_domain::ids::FolderId>,
    project_id: Option<atlas_domain::ids::ProjectId>,
) -> atlas_domain::entities::documents::Document {
    let ctx = support::ctx(ws, user);
    repo.create(
        &ctx,
        NewDocument {
            title: title.into(),
            slug: Some(atlas_domain::slugify(title)),
            content: "".into(),
            folder_id,
            project_id,
            frontmatter: None,
        },
    )
    .await
    .expect("create doc in folder")
}

async fn create_project(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    slug: &str,
) -> atlas_domain::entities::workspace_core::Project {
    use atlas_server::persistence::repos::{PgProjectRepo, ProjectRepo};
    let repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    let ctx = support::ctx(ws, user);

    // task_prefix is globally unique and must match ^[A-Z][A-Z0-9]{1,9}$; derive
    // a fresh one per call so a test can create several projects without
    // colliding on projects_task_prefix_uq.
    let simple = uuid::Uuid::now_v7().as_simple().to_string();
    let task_prefix = format!("P{}", simple[simple.len() - 5..].to_uppercase());

    repo.create(
        &ctx,
        atlas_domain::entities::workspace_core::NewProject {
            name: slug.into(),
            slug: slug.into(),
            task_prefix,
            visibility: atlas_domain::permissions::Visibility::Private,
        },
    )
    .await
    .expect("create project")
}

async fn create_api_key(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
) -> atlas_domain::ids::ApiKeyId {
    use atlas_server::persistence::repos::{ApiKeyRepo, NewApiKey, PgApiKeyRepo};
    let repo = PgApiKeyRepo {
        conn: db.conn().clone(),
    };
    let ctx = support::ctx(ws, user);
    let created = repo
        .create(
            &ctx,
            NewApiKey {
                name: "scope-key".into(),
                token_hash: format!("hash-{}", uuid::Uuid::now_v7()),
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
            },
        )
        .await
        .expect("create api key");
    created.id
}

async fn grant_to_api_key(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    api_key_id: atlas_domain::ids::ApiKeyId,
    project_id: Option<atlas_domain::ids::ProjectId>,
    folder_id: Option<atlas_domain::ids::FolderId>,
) {
    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_domain::permissions::ResourceRole;
    use atlas_server::persistence::repos::{PermissionGrantRepo, PgPermissionGrantRepo};
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws.id,
        user_id: None,
        api_key_id: Some(api_key_id),
        group_id: None,
        project_id,
        folder_id,
        document_id: None,
        board_id: None,
        role: ResourceRole::Viewer,
        created_by_user_id: None,
        created_by_api_key_id: None,
    })
    .await
    .expect("grant to api key");
}

#[tokio::test]
async fn list_visible_includes_doc_via_folder_scope_grant() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, owner) = support::seed_workspace(&db, "lv-folder-grant").await;
    let repo = make_doc_repo(&db);

    let folder = create_folder(&db, &ws, &owner, None, None).await;
    let doc = create_doc_in_folder(&repo, &ws, &owner, "In Folder", Some(folder.id), None).await;
    create_doc_in_folder(&repo, &ws, &owner, "Outside Folder", None, None).await;

    let key_id = create_api_key(&db, &ws, &owner).await;
    grant_to_api_key(&db, &ws, key_id, None, Some(folder.id)).await;

    let ctx = support::ctx(&ws, &owner);
    let principal = Principal::ApiKey(key_id);
    let docs = repo
        .list_visible(&ctx, &principal, None, None, 10)
        .await
        .expect("list_visible");

    assert_eq!(
        docs.len(),
        1,
        "api key with only a folder-scope grant must see exactly the doc in that folder"
    );
    assert_eq!(docs[0].id, doc.id);

    db.teardown().await;
}

#[tokio::test]
async fn list_visible_includes_nested_doc_via_ancestor_folder_grant() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, owner) = support::seed_workspace(&db, "lv-nested-grant").await;
    let repo = make_doc_repo(&db);

    let parent = create_folder(&db, &ws, &owner, None, None).await;
    let child = create_folder(&db, &ws, &owner, None, Some(parent.id)).await;
    let doc = create_doc_in_folder(&repo, &ws, &owner, "Nested Doc", Some(child.id), None).await;

    let key_id = create_api_key(&db, &ws, &owner).await;
    grant_to_api_key(&db, &ws, key_id, None, Some(parent.id)).await;

    let ctx = support::ctx(&ws, &owner);
    let principal = Principal::ApiKey(key_id);
    let docs = repo
        .list_visible(&ctx, &principal, None, None, 10)
        .await
        .expect("list_visible");

    assert_eq!(
        docs.len(),
        1,
        "ancestor folder-scope grant must reveal a doc in a nested folder"
    );
    assert_eq!(docs[0].id, doc.id);

    db.teardown().await;
}

#[tokio::test]
async fn list_visible_includes_doc_via_project_scope_grant() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, owner) = support::seed_workspace(&db, "lv-project-grant").await;
    let repo = make_doc_repo(&db);

    let project = create_project(&db, &ws, &owner, "proj-lv").await;

    let doc = create_doc_in_folder(&repo, &ws, &owner, "In Project", None, Some(project.id)).await;
    create_doc_in_folder(&repo, &ws, &owner, "No Project", None, None).await;

    let key_id = create_api_key(&db, &ws, &owner).await;
    grant_to_api_key(&db, &ws, key_id, Some(project.id), None).await;

    let ctx = support::ctx(&ws, &owner);
    let principal = Principal::ApiKey(key_id);
    let docs = repo
        .list_visible(&ctx, &principal, None, None, 10)
        .await
        .expect("list_visible");

    assert_eq!(
        docs.len(),
        1,
        "api key with only a project-scope grant must see exactly the doc in that project"
    );
    assert_eq!(docs[0].id, doc.id);

    db.teardown().await;
}

#[tokio::test]
async fn list_visible_excludes_doc_when_no_grant_for_non_member() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, owner) = support::seed_workspace(&db, "lv-no-grant").await;
    let repo = make_doc_repo(&db);

    let folder = create_folder(&db, &ws, &owner, None, None).await;
    create_doc_in_folder(&repo, &ws, &owner, "Hidden", Some(folder.id), None).await;

    let key_id = create_api_key(&db, &ws, &owner).await;

    let ctx = support::ctx(&ws, &owner);
    let principal = Principal::ApiKey(key_id);
    let docs = repo
        .list_visible(&ctx, &principal, None, None, 10)
        .await
        .expect("list_visible");

    assert!(
        docs.is_empty(),
        "api key without any grant must not see documents, got {} docs",
        docs.len()
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_visible_terminates_on_folder_cycle() {
    use sea_orm::{ConnectionTrait, Statement};

    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, owner) = support::seed_workspace(&db, "lv-folder-cycle").await;
    let repo = make_doc_repo(&db);

    let folder_a = create_folder(&db, &ws, &owner, None, None).await;
    let folder_b = create_folder(&db, &ws, &owner, None, Some(folder_a.id)).await;

    let doc = create_doc_in_folder(&repo, &ws, &owner, "In Cycle", Some(folder_b.id), None).await;

    // Force a parent cycle A -> B -> A. No HTTP route can create this; the
    // folder-ancestry CTE in list_visible must still terminate.
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE folders SET parent_folder_id = $1 WHERE id = $2",
            [folder_b.id.0.into(), folder_a.id.0.into()],
        ))
        .await
        .expect("induce folder cycle");

    let key_id = create_api_key(&db, &ws, &owner).await;
    grant_to_api_key(&db, &ws, key_id, None, Some(folder_a.id)).await;

    let ctx = support::ctx(&ws, &owner);
    let principal = Principal::ApiKey(key_id);

    let docs = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        repo.list_visible(&ctx, &principal, None, None, 10),
    )
    .await
    .expect("list_visible must not hang on a folder cycle")
    .expect("list_visible");

    assert_eq!(
        docs.len(),
        1,
        "folder-scope grant must still reveal the doc despite the cycle"
    );
    assert_eq!(docs[0].id, doc.id);

    db.teardown().await;
}

// --- list_visible: project scoping ---

#[tokio::test]
async fn list_visible_scoped_to_project_excludes_other_and_projectless_docs() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "lv-proj-scope").await;
    let repo = make_doc_repo(&db);

    let project_a = create_project(&db, &ws, &user, "proj-a").await;
    let project_b = create_project(&db, &ws, &user, "proj-b").await;

    let in_a = create_doc_in_folder(&repo, &ws, &user, "In A", None, Some(project_a.id)).await;
    create_doc_in_folder(&repo, &ws, &user, "In B", None, Some(project_b.id)).await;
    create_doc_in_folder(&repo, &ws, &user, "No Project", None, None).await;

    let ctx = support::ctx(&ws, &user);
    let principal = user_principal(&user);

    let docs = repo
        .list_visible(&ctx, &principal, Some(project_a.id), None, 10)
        .await
        .expect("list_visible");

    assert_eq!(
        docs.len(),
        1,
        "project-scoped listing must return only that project's docs"
    );
    assert_eq!(docs[0].id, in_a.id);

    db.teardown().await;
}

#[tokio::test]
async fn list_visible_project_scope_pagination_works() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "lv-proj-page").await;
    let repo = make_doc_repo(&db);

    let project = create_project(&db, &ws, &user, "proj-page").await;

    create_doc_in_folder(&repo, &ws, &user, "P1", None, Some(project.id)).await;
    create_doc_in_folder(&repo, &ws, &user, "P2", None, Some(project.id)).await;
    create_doc_in_folder(&repo, &ws, &user, "P3", None, Some(project.id)).await;
    create_doc_in_folder(&repo, &ws, &user, "Other", None, None).await;

    let ctx = support::ctx(&ws, &user);
    let principal = user_principal(&user);

    let page1 = repo
        .list_visible(&ctx, &principal, Some(project.id), None, 2)
        .await
        .expect("page1");
    assert_eq!(page1.len(), 2, "first page must have 2 project docs");

    let cursor = page1.last().map(|d| d.id.0);
    let page2 = repo
        .list_visible(&ctx, &principal, Some(project.id), cursor, 2)
        .await
        .expect("page2");
    assert_eq!(
        page2.len(),
        1,
        "second page must have the remaining project doc"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_visible_project_scope_still_applies_permission_predicate() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, owner) = support::seed_workspace(&db, "lv-proj-perm").await;
    let repo = make_doc_repo(&db);

    let project = create_project(&db, &ws, &owner, "proj-perm").await;
    create_doc_in_folder(&repo, &ws, &owner, "In Project", None, Some(project.id)).await;

    // Non-member api key with no grant: the permission predicate must still
    // exclude the doc even within the project scope.
    let key_id = create_api_key(&db, &ws, &owner).await;

    let ctx = support::ctx(&ws, &owner);
    let principal = Principal::ApiKey(key_id);

    let docs = repo
        .list_visible(&ctx, &principal, Some(project.id), None, 10)
        .await
        .expect("list_visible");

    assert!(
        docs.is_empty(),
        "project scope must not bypass the permission predicate, got {} docs",
        docs.len()
    );

    db.teardown().await;
}

// --- find_by_slug ---

#[tokio::test]
async fn find_by_slug_returns_correct_document() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "fbs-basic").await;
    let repo = make_doc_repo(&db);

    let created = create_doc(&repo, &ws, &user, "My Doc", Some("my-doc")).await;
    let ctx = support::ctx(&ws, &user);

    let found = repo
        .find_by_slug(&ctx, "my-doc")
        .await
        .expect("find_by_slug")
        .expect("must find doc");

    assert_eq!(found.id, created.id);
    assert_eq!(found.title, "My Doc");

    db.teardown().await;
}

#[tokio::test]
async fn find_by_slug_returns_none_for_unknown_slug() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "fbs-missing").await;
    let repo = make_doc_repo(&db);
    let ctx = support::ctx(&ws, &user);

    let result = repo
        .find_by_slug(&ctx, "no-such-doc")
        .await
        .expect("find_by_slug");

    assert!(result.is_none(), "unknown slug must return None");

    db.teardown().await;
}

#[tokio::test]
async fn find_by_slug_is_cross_tenant_safe() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws1, user1) = support::seed_workspace(&db, "fbs-tenant1").await;
    let (ws2, user2) = support::seed_workspace(&db, "fbs-tenant2").await;
    let repo = make_doc_repo(&db);

    create_doc(&repo, &ws2, &user2, "Same Slug", Some("same-slug")).await;
    let ctx1 = support::ctx(&ws1, &user1);

    let result = repo
        .find_by_slug(&ctx1, "same-slug")
        .await
        .expect("find_by_slug cross-tenant");

    assert!(
        result.is_none(),
        "cross-tenant slug must not be visible from another workspace"
    );

    db.teardown().await;
}

// --- rename ---

#[tokio::test]
async fn rename_updates_title_but_preserves_slug() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "rename-basic").await;
    let repo = make_doc_repo(&db);

    let doc = create_doc(&repo, &ws, &user, "Old Title", Some("old-title")).await;
    let ctx = support::ctx(&ws, &user);

    let renamed = repo
        .rename(&ctx, doc.id, "New Title".to_string())
        .await
        .expect("rename");

    assert_eq!(renamed.title, "New Title");
    assert_eq!(
        renamed.slug,
        Some("old-title".to_string()),
        "slug must be stable across title renames"
    );

    db.teardown().await;
}

#[tokio::test]
async fn rename_to_colliding_title_preserves_slug() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "rename-collision").await;
    let repo = make_doc_repo(&db);

    create_doc(&repo, &ws, &user, "Already Exists", Some("already-exists")).await;
    let doc2 = create_doc(&repo, &ws, &user, "Different", Some("different")).await;
    let ctx = support::ctx(&ws, &user);

    let renamed = repo
        .rename(&ctx, doc2.id, "Already Exists".to_string())
        .await
        .expect("rename with colliding title");

    assert_eq!(renamed.title, "Already Exists");
    assert_eq!(
        renamed.slug,
        Some("different".to_string()),
        "slug must remain the original slug even when the title would collide"
    );

    db.teardown().await;
}

#[tokio::test]
async fn rename_cross_tenant_not_found() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws1, user1) = support::seed_workspace(&db, "rename-tenant1").await;
    let (ws2, user2) = support::seed_workspace(&db, "rename-tenant2").await;
    let repo = make_doc_repo(&db);

    let doc = create_doc(&repo, &ws2, &user2, "WS2 Doc", Some("ws2-doc")).await;
    let ctx1 = support::ctx(&ws1, &user1);

    let result = repo.rename(&ctx1, doc.id, "Hacked".to_string()).await;

    assert!(
        matches!(result, Err(atlas_domain::DomainError::NotFound { .. })),
        "cross-tenant rename must return NotFound"
    );

    let _ = user2;

    db.teardown().await;
}
