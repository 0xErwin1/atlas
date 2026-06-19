#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::documents::CreateDocumentRequest;
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx};
use atlas_server::persistence::repos::{DocumentRepo, FolderRepo};

fn doc_req(title: &str, content: Option<&str>) -> CreateDocumentRequest {
    CreateDocumentRequest {
        title: title.to_string(),
        folder_id: None,
        content: content.map(|c| c.to_string()),
    }
}

// ---- Copy document ---------------------------------------------------------

#[tokio::test]
async fn copy_document_creates_independent_copy() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "copy-doc-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "copy-doc-proj-1".to_string(),
                task_prefix: "CD1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let source = client
        .create_document(
            &ws.slug,
            &project.slug,
            doc_req("Source Doc", Some("Hello [[World]] body.")),
        )
        .await
        .expect("create source");

    let source_slug = source.slug.as_deref().expect("source slug");

    let copy = client
        .copy_document(&ws.slug, source_slug, None)
        .await
        .expect("copy document");

    assert_eq!(copy.title, "Source Doc (copy)");
    assert_eq!(
        copy.content, source.content,
        "content must be copied verbatim"
    );
    assert_ne!(copy.id, source.id, "copy must be a new document");
    assert_ne!(copy.slug, source.slug, "copy must get a fresh slug");
    assert_eq!(copy.project_id, source.project_id);
    assert_eq!(copy.folder_id, source.folder_id);
    assert_ne!(
        copy.head_revision_id, source.head_revision_id,
        "copy must have its own first revision"
    );

    // Source remains untouched.
    let refetched = client
        .get_document(&ws.slug, source_slug)
        .await
        .expect("refetch source");
    assert_eq!(refetched.title, "Source Doc");

    // Copy appears in the project's document list.
    let page = client
        .list_documents(&ws.slug, &project.slug, None, None)
        .await
        .expect("list documents");
    assert!(
        page.items.iter().any(|d| d.title == "Source Doc (copy)"),
        "copy must appear in the project's document list"
    );

    db.teardown().await;
}

#[tokio::test]
async fn copy_document_into_specific_folder() {
    use atlas_domain::ids::ProjectId;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "copy-doc-2").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "copy-doc-proj-2".to_string(),
                task_prefix: "CD2".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let target_folder = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(ws.id, Actor::User(user.id)),
            atlas_domain::entities::workspace_core::NewFolder {
                project_id: Some(ProjectId(project.id)),
                parent_folder_id: None,
                name: "target".to_string(),
            },
        )
        .await
        .expect("create target folder");

    let source = client
        .create_document(&ws.slug, &project.slug, doc_req("To Copy", None))
        .await
        .expect("create source");

    let source_slug = source.slug.as_deref().expect("source slug");

    let copy = client
        .copy_document(&ws.slug, source_slug, Some(target_folder.id.0))
        .await
        .expect("copy document into folder");

    assert_eq!(copy.folder_id, Some(target_folder.id.0));
    assert_eq!(copy.project_id, Some(project.id));

    db.teardown().await;
}

// ---- Copy folder -----------------------------------------------------------

#[tokio::test]
async fn copy_folder_recursively_duplicates_subtree() {
    use atlas_domain::ids::{FolderId, ProjectId};

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "copy-folder-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "copy-folder-proj-1".to_string(),
                task_prefix: "CF1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    let top = db
        .folder_repo()
        .create(
            &ctx,
            atlas_domain::entities::workspace_core::NewFolder {
                project_id: Some(ProjectId(project.id)),
                parent_folder_id: None,
                name: "Top".to_string(),
            },
        )
        .await
        .expect("create top folder");

    let child = db
        .folder_repo()
        .create(
            &ctx,
            atlas_domain::entities::workspace_core::NewFolder {
                project_id: Some(ProjectId(project.id)),
                parent_folder_id: Some(top.id),
                name: "Child".to_string(),
            },
        )
        .await
        .expect("create child folder");

    // Document directly in Top.
    let doc_repo = db.doc_repo();
    let top_doc = doc_repo
        .create(
            &ctx,
            atlas_domain::entities::documents::NewDocument {
                title: "Top Doc".to_string(),
                slug: Some("top-doc".to_string()),
                content: "top content".to_string(),
                folder_id: Some(top.id),
                project_id: Some(ProjectId(project.id)),
                frontmatter: None,
            },
        )
        .await
        .expect("create top doc");

    // Document nested in Child.
    let child_doc = doc_repo
        .create(
            &ctx,
            atlas_domain::entities::documents::NewDocument {
                title: "Child Doc".to_string(),
                slug: Some("child-doc".to_string()),
                content: "child content".to_string(),
                folder_id: Some(child.id),
                project_id: Some(ProjectId(project.id)),
                frontmatter: None,
            },
        )
        .await
        .expect("create child doc");

    let copy = client
        .copy_folder(&ws.slug, top.id.0, None)
        .await
        .expect("copy folder");

    assert_eq!(copy.name, "Top (copy)");
    assert_ne!(copy.id, top.id.0, "copied top folder must have a new id");
    assert_eq!(copy.parent_folder_id, top.parent_folder_id.map(|f| f.0));
    assert_eq!(copy.project_id, Some(project.id));

    // The copied subtree: list folders in the project and find the new Child.
    let folders = client
        .list_folders(&ws.slug, &project.slug, None, None)
        .await
        .expect("list folders");

    let copied_child = folders
        .items
        .iter()
        .find(|f| f.name == "Child" && f.parent_folder_id == Some(copy.id))
        .expect("copied child folder under the copied top");
    assert_ne!(
        copied_child.id, child.id.0,
        "copied child must have a new id"
    );

    // Documents copied with same titles (descendants keep their title, no suffix).
    let copied_top_doc = doc_repo
        .list_in_folder(&ctx, FolderId(copy.id))
        .await
        .expect("list copied top docs");
    assert_eq!(copied_top_doc.len(), 1);
    assert_eq!(copied_top_doc[0].title, "Top Doc");
    assert_eq!(copied_top_doc[0].content, "top content");
    assert_ne!(
        copied_top_doc[0].id, top_doc.id,
        "copied doc must have a new id"
    );

    let copied_child_doc = doc_repo
        .list_in_folder(&ctx, FolderId(copied_child.id))
        .await
        .expect("list copied child docs");
    assert_eq!(copied_child_doc.len(), 1);
    assert_eq!(copied_child_doc[0].title, "Child Doc");
    assert_eq!(copied_child_doc[0].content, "child content");
    assert_ne!(
        copied_child_doc[0].id, child_doc.id,
        "copied nested doc must have a new id"
    );

    // Original subtree untouched: originals still present.
    let original_top_docs = doc_repo
        .list_in_folder(&ctx, top.id)
        .await
        .expect("list original top docs");
    assert_eq!(original_top_docs.len(), 1);
    assert_eq!(original_top_docs[0].id, top_doc.id);

    db.teardown().await;
}

// ---- Unauthenticated -------------------------------------------------------

#[tokio::test]
async fn copy_document_unauthenticated_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let anon = atlas_client::AtlasClient::new(server.base_url().to_string());

    let result = anon.copy_document("any-ws", "any-slug", None).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated copy_document must return 401, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn copy_folder_unauthenticated_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let anon = atlas_client::AtlasClient::new(server.base_url().to_string());

    let result = anon.copy_folder("any-ws", uuid::Uuid::now_v7(), None).await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "unauthenticated copy_folder must return 401, got: {result:?}"
    );

    db.teardown().await;
}
