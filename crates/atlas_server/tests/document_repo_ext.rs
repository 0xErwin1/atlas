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
        .list_visible(&ctx, &principal, None, 10)
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
        .list_visible(&ctx1, &principal1, None, 10)
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
        .list_visible(&ctx, &principal, None, 2)
        .await
        .expect("page1");
    assert_eq!(page1.len(), 2, "first page must have 2 docs");

    let cursor = page1.last().map(|d| d.id.0);
    let page2 = repo
        .list_visible(&ctx, &principal, cursor, 2)
        .await
        .expect("page2");
    assert_eq!(page2.len(), 1, "second page must have the remaining doc");

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
async fn rename_updates_title_and_slug() {
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
    assert_eq!(renamed.slug, Some("new-title".to_string()));

    db.teardown().await;
}

#[tokio::test]
async fn rename_stable_slug_collision_appends_suffix() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "rename-collision").await;
    let repo = make_doc_repo(&db);

    create_doc(&repo, &ws, &user, "Already Exists", Some("already-exists")).await;
    let doc2 = create_doc(&repo, &ws, &user, "Different", Some("different")).await;
    let ctx = support::ctx(&ws, &user);

    let renamed = repo
        .rename(&ctx, doc2.id, "Already Exists".to_string())
        .await
        .expect("rename with collision");

    assert_eq!(renamed.title, "Already Exists");
    assert_eq!(
        renamed.slug,
        Some("already-exists-2".to_string()),
        "collision must add suffix"
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
