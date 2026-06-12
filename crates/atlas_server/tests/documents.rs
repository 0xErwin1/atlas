#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::entities::documents::{DocumentFilter, NewDocument};
use atlas_server::persistence::repos::{DocumentRepo, PgDocumentRepo};

fn make_doc_repo(db: &support::TestDb, anchor_interval: u32) -> PgDocumentRepo {
    PgDocumentRepo::new(db.conn().clone(), anchor_interval)
}

#[tokio::test]
async fn document_create_and_get_roundtrip() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "doc-user-1").await;
    let ctx = support::ctx(&ws, &user);
    let repo = make_doc_repo(&db, 50);

    let doc = repo
        .create(
            &ctx,
            NewDocument {
                title: "My First Doc".into(),
                content: "Hello, world!".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    assert_eq!(doc.title, "My First Doc");
    assert_eq!(doc.content, "Hello, world!");

    let fetched = repo
        .get(&ctx, doc.id)
        .await
        .expect("get document")
        .expect("document must exist");

    assert_eq!(fetched.id, doc.id);
    assert_eq!(fetched.content, "Hello, world!");

    db.teardown().await;
}

#[tokio::test]
async fn cas_stale_revision_returns_conflict() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "doc-user-cas").await;
    let ctx = support::ctx(&ws, &user);
    let repo = make_doc_repo(&db, 50);

    let doc = repo
        .create(
            &ctx,
            NewDocument {
                title: "CAS Doc".into(),
                content: "version one".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    let rev1 = doc.current_revision_id;

    repo.update_content(&ctx, doc.id, rev1, "version two")
        .await
        .expect("first update succeeds");

    let result = repo
        .update_content(&ctx, doc.id, rev1, "version three from stale")
        .await;

    assert!(result.is_err(), "stale revision must return conflict");
    match result.unwrap_err() {
        atlas_domain::DomainError::Conflict(conflict) => {
            assert_eq!(conflict.document_id, doc.id);
        }
        other => panic!("expected Conflict, got {:?}", other),
    }

    db.teardown().await;
}

#[tokio::test]
async fn anchor_roundtrip_across_boundary() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "doc-user-anchor").await;
    let ctx = support::ctx(&ws, &user);
    let repo = make_doc_repo(&db, 3);

    let doc = repo
        .create(
            &ctx,
            NewDocument {
                title: "Anchor Doc".into(),
                content: "v1".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    let r1 = doc.current_revision_id;
    let d2 = repo
        .update_content(&ctx, doc.id, r1, "v2")
        .await
        .expect("update to v2");
    let r2 = d2.current_revision_id;

    let d3 = repo
        .update_content(&ctx, doc.id, r2, "v3")
        .await
        .expect("update to v3");
    let r3 = d3.current_revision_id;

    let d4 = repo
        .update_content(&ctx, doc.id, r3, "v4")
        .await
        .expect("update to v4");
    let _ = d4;

    let content_at_3 = repo
        .content_at(&ctx, doc.id, 3)
        .await
        .expect("content_at seq 3");

    assert_eq!(
        content_at_3, "v3",
        "content_at must reconstruct seq 3 correctly"
    );

    db.teardown().await;
}

#[tokio::test]
async fn document_list_returns_summaries_without_content() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "doc-user-list").await;
    let ctx = support::ctx(&ws, &user);
    let repo = make_doc_repo(&db, 50);

    repo.create(
        &ctx,
        NewDocument {
            title: "Summary Test".into(),
            content: "large content body".into(),
            folder_id: None,
            project_id: None,
            frontmatter: None,
        },
    )
    .await
    .expect("create document");

    let summaries = repo
        .list(&ctx, DocumentFilter::default())
        .await
        .expect("list documents");

    assert_eq!(summaries.len(), 1);
    let first = summaries.first().expect("summaries must not be empty");
    assert_eq!(first.title, "Summary Test");

    db.teardown().await;
}
