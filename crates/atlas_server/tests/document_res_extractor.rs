#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;

use atlas_domain::{entities::documents::NewDocument, ids::DocumentId, permissions::ResourceRef};
use atlas_server::{
    authz::authorized::{DocumentRes, FolderRes, ResolvedResource},
    error::ApiError,
    persistence::repos::{DocumentRepo, PgDocumentRepo},
};

fn doc_params(id: DocumentId) -> HashMap<String, String> {
    let mut map = HashMap::new();
    map.insert("doc_id".to_string(), id.0.to_string());
    map
}

async fn create_doc(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    title: &str,
) -> atlas_domain::entities::documents::Document {
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let ctx = support::ctx(ws, user);
    repo.create(
        &ctx,
        NewDocument {
            title: title.into(),
            slug: None,
            content: "".into(),
            folder_id: None,
            project_id: None,
            frontmatter: None,
        },
    )
    .await
    .expect("create doc")
}

#[tokio::test]
async fn document_res_resolve_unknown_id_returns_not_found() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, _user) = support::seed_workspace(&db, "docres-missing").await;

    let unknown_id = DocumentId::new();
    let result = DocumentRes::resolve(db.conn(), &ws, doc_params(unknown_id)).await;

    assert!(
        matches!(result, Err(ApiError::NotFound)),
        "unknown document id must return NotFound"
    );

    db.teardown().await;
}

#[tokio::test]
async fn document_res_resolve_cross_tenant_id_returns_not_found() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws1, user1) = support::seed_workspace(&db, "docres-tenant1").await;
    let (ws2, user2) = support::seed_workspace(&db, "docres-tenant2").await;

    let doc = create_doc(&db, &ws2, &user2, "WS2 Doc").await;

    let result = DocumentRes::resolve(db.conn(), &ws1, doc_params(doc.id)).await;

    assert!(
        matches!(result, Err(ApiError::NotFound)),
        "cross-tenant document must return NotFound"
    );

    let _ = user1;

    db.teardown().await;
}

#[tokio::test]
async fn document_res_resolve_returns_chain_with_document_and_workspace_segments() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "docres-chain").await;

    let doc = create_doc(&db, &ws, &user, "Chain Doc").await;

    let (res, chain) = DocumentRes::resolve(db.conn(), &ws, doc_params(doc.id))
        .await
        .expect("resolve must succeed for existing doc");

    assert_eq!(res.0.id, doc.id, "DocumentRes must carry the resolved doc");

    let has_workspace = chain
        .segments
        .iter()
        .any(|s| s.resource == ResourceRef::Workspace);
    assert!(has_workspace, "chain must include a workspace segment");

    let has_document = chain
        .segments
        .iter()
        .any(|s| matches!(&s.resource, ResourceRef::Document(did) if did.0 == doc.id.0));
    assert!(has_document, "chain must include the document segment");

    let _ = user;

    db.teardown().await;
}

#[tokio::test]
async fn folder_res_stub_is_defined() {
    let _: Option<FolderRes> = None;
}
