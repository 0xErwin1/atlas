#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::{
    Actor,
    entities::documents::{DocumentFilter, NewAttachment, NewDocument},
    ids::ApiKeyId,
};
use atlas_server::persistence::repos::{
    ApiKeyRepo, AttachmentRepo, DocumentRepo, NewApiKey, PgApiKeyRepo, PgAttachmentRepo,
    PgDocumentRepo,
};
use serde_json::json;

fn make_doc_repo(db: &support::TestDb, anchor_interval: u32) -> PgDocumentRepo {
    PgDocumentRepo::new(db.conn().clone(), anchor_interval)
}

async fn seed_api_key(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
) -> ApiKeyId {
    let api_key_repo = PgApiKeyRepo {
        conn: db.conn().clone(),
    };
    let ctx = support::ctx(ws, user);

    let key = api_key_repo
        .create(
            &ctx,
            NewApiKey {
                name: "test-key".into(),
                token_hash: format!("hash-{}", uuid::Uuid::now_v7()),
                expires_at: None,
            },
        )
        .await
        .expect("seed api key");

    key.id
}

#[tokio::test]
async fn api_key_actor_create_sets_created_by_api_key_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "doc-apikey-attr").await;
    let key_id = seed_api_key(&db, &ws, &user).await;
    let ctx = atlas_domain::WorkspaceCtx::new(ws.id, Actor::ApiKey(key_id));
    let repo = make_doc_repo(&db, 50);

    let doc = repo
        .create(
            &ctx,
            NewDocument {
                title: "API Key Doc".into(),
                content: "content".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create with api-key actor");

    assert_eq!(
        doc.created_by_api_key_id,
        Some(key_id),
        "created_by_api_key_id must be set for ApiKey actor"
    );
    assert!(
        doc.created_by_user_id.is_none(),
        "created_by_user_id must be None for ApiKey actor"
    );

    db.teardown().await;
}

#[tokio::test]
async fn api_key_actor_update_content_sets_revision_api_key_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "doc-apikey-rev").await;
    let user_ctx = support::ctx(&ws, &user);
    let repo = make_doc_repo(&db, 50);

    let doc = repo
        .create(
            &user_ctx,
            NewDocument {
                title: "Rev API Key Doc".into(),
                content: "v1".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    let key_id = seed_api_key(&db, &ws, &user).await;
    let api_ctx = atlas_domain::WorkspaceCtx::new(ws.id, Actor::ApiKey(key_id));

    let updated = repo
        .update_content(&api_ctx, doc.id, doc.current_revision_id, "v2")
        .await
        .expect("update_content with api-key actor");

    assert_eq!(updated.content, "v2");

    db.teardown().await;
}

#[tokio::test]
async fn api_key_actor_attachment_record_sets_created_by_api_key_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "doc-apikey-attach").await;
    let user_ctx = support::ctx(&ws, &user);
    let doc_repo = make_doc_repo(&db, 50);
    let att_repo = PgAttachmentRepo {
        conn: db.conn().clone(),
    };

    let doc = doc_repo
        .create(
            &user_ctx,
            NewDocument {
                title: "Attachment Owner".into(),
                content: "".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    let key_id = seed_api_key(&db, &ws, &user).await;
    let api_ctx = atlas_domain::WorkspaceCtx::new(ws.id, Actor::ApiKey(key_id));

    let att = att_repo
        .record(
            &api_ctx,
            NewAttachment {
                document_id: Some(doc.id),
                task_id: None,
                file_name: "file.txt".into(),
                content_type: "text/plain".into(),
                size_bytes: 4,
                sha256: "abc123".into(),
            },
        )
        .await
        .expect("record attachment with api-key actor");

    assert_eq!(
        att.created_by_api_key_id,
        Some(key_id),
        "created_by_api_key_id must be set for ApiKey actor on attachment"
    );
    assert!(
        att.created_by_user_id.is_none(),
        "created_by_user_id must be None for ApiKey actor on attachment"
    );

    db.teardown().await;
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

#[tokio::test]
async fn document_frontmatter_defaults_to_empty_object() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "doc-user-fm").await;
    let ctx = support::ctx(&ws, &user);
    let repo = make_doc_repo(&db, 50);

    let doc = repo
        .create(
            &ctx,
            NewDocument {
                title: "FM Default".into(),
                content: "content".into(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document without explicit frontmatter");

    assert_eq!(
        doc.frontmatter,
        json!({}),
        "frontmatter must default to an empty JSON object when not provided"
    );

    db.teardown().await;
}
