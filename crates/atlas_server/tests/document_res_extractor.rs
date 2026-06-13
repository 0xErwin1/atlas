#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;

use atlas_domain::{
    entities::documents::NewDocument,
    entities::permissions::NewPermissionGrant,
    entities::workspace_core::NewFolder,
    ids::{DocumentId, FolderId},
    permissions::{ResourceRef, ResourceRole},
};
use atlas_server::{
    authz::authorized::{DocumentRes, FolderRes, ResolvedResource},
    error::ApiError,
    persistence::repos::{
        DocumentRepo, FolderRepo, PermissionGrantRepo, PgDocumentRepo, PgFolderRepo,
        PgPermissionGrantRepo,
    },
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
    create_doc_in_folder(db, ws, user, title, None).await
}

async fn create_doc_in_folder(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    title: &str,
    folder_id: Option<FolderId>,
) -> atlas_domain::entities::documents::Document {
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let ctx = support::ctx(ws, user);
    repo.create(
        &ctx,
        NewDocument {
            title: title.into(),
            slug: None,
            content: "".into(),
            folder_id,
            project_id: None,
            frontmatter: None,
        },
    )
    .await
    .expect("create doc")
}

async fn create_folder(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    parent: Option<FolderId>,
) -> atlas_domain::entities::workspace_core::Folder {
    let repo = PgFolderRepo {
        conn: db.conn().clone(),
    };
    let ctx = support::ctx(ws, user);
    repo.create(
        &ctx,
        NewFolder {
            project_id: None,
            parent_folder_id: parent,
            name: "test-folder".into(),
        },
    )
    .await
    .expect("create folder")
}

async fn grant_folder_role(
    db: &support::TestDb,
    ws: &atlas_server::persistence::repos::Workspace,
    user: &atlas_server::persistence::repos::User,
    folder_id: FolderId,
    role: ResourceRole,
) {
    let repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    repo.upsert(NewPermissionGrant {
        workspace_id: ws.id,
        user_id: Some(user.id),
        api_key_id: None,
        project_id: None,
        folder_id: Some(folder_id),
        document_id: None,
        board_id: None,
        role,
        created_by_user_id: Some(user.id),
        created_by_api_key_id: None,
    })
    .await
    .expect("upsert folder grant");
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

// --- folder ancestry in DocumentRes chain ---

#[tokio::test]
async fn document_res_chain_includes_folder_segment_for_document_in_folder() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "docres-folder-chain").await;

    let folder = create_folder(&db, &ws, &user, None).await;
    let doc = create_doc_in_folder(&db, &ws, &user, "Folder Doc", Some(folder.id)).await;

    let (_, chain) = DocumentRes::resolve(db.conn(), &ws, doc_params(doc.id))
        .await
        .expect("resolve must succeed");

    let has_folder = chain
        .segments
        .iter()
        .any(|s| matches!(&s.resource, ResourceRef::Folder(fid) if fid.0 == folder.id.0));
    assert!(
        has_folder,
        "chain must include a Folder segment for a document inside a folder"
    );

    db.teardown().await;
}

#[tokio::test]
async fn document_res_chain_includes_all_ancestor_folders_for_nested_document() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "docres-nested-chain").await;

    let parent_folder = create_folder(&db, &ws, &user, None).await;
    let child_folder = create_folder(&db, &ws, &user, Some(parent_folder.id)).await;
    let doc = create_doc_in_folder(&db, &ws, &user, "Nested Doc", Some(child_folder.id)).await;

    let (_, chain) = DocumentRes::resolve(db.conn(), &ws, doc_params(doc.id))
        .await
        .expect("resolve must succeed");

    let has_child = chain
        .segments
        .iter()
        .any(|s| matches!(&s.resource, ResourceRef::Folder(fid) if fid.0 == child_folder.id.0));
    let has_parent = chain
        .segments
        .iter()
        .any(|s| matches!(&s.resource, ResourceRef::Folder(fid) if fid.0 == parent_folder.id.0));

    assert!(
        has_child,
        "chain must include the immediate folder segment (child)"
    );
    assert!(
        has_parent,
        "chain must include the ancestor folder segment (parent)"
    );

    let child_pos = chain
        .segments
        .iter()
        .position(|s| matches!(&s.resource, ResourceRef::Folder(fid) if fid.0 == child_folder.id.0))
        .expect("child pos");
    let parent_pos = chain
        .segments
        .iter()
        .position(
            |s| matches!(&s.resource, ResourceRef::Folder(fid) if fid.0 == parent_folder.id.0),
        )
        .expect("parent pos");
    assert!(
        child_pos < parent_pos,
        "child folder must appear before ancestor folder (most-specific-first)"
    );

    db.teardown().await;
}

#[tokio::test]
async fn folder_scope_grant_resolves_for_document_in_that_folder() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "docres-folder-grant").await;

    let folder = create_folder(&db, &ws, &user, None).await;
    let doc = create_doc_in_folder(&db, &ws, &user, "Granted Doc", Some(folder.id)).await;

    grant_folder_role(&db, &ws, &user, folder.id, ResourceRole::Viewer).await;

    let (_, chain) = DocumentRes::resolve(db.conn(), &ws, doc_params(doc.id))
        .await
        .expect("resolve must succeed");

    let has_folder_in_chain = chain
        .segments
        .iter()
        .any(|s| matches!(&s.resource, ResourceRef::Folder(fid) if fid.0 == folder.id.0));
    assert!(
        has_folder_in_chain,
        "folder-scope grant must be reachable: folder must appear in chain"
    );

    db.teardown().await;
}
