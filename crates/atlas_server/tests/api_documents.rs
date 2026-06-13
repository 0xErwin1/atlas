#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateApiKeyRequest,
    documents::{
        CreateDocumentRequest, MoveDocumentRequest, UpdateContentRequest, UpdateDocumentRequest,
    },
};
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, PermissionGrantRepo, UserRepo};

fn doc_req(title: &str) -> CreateDocumentRequest {
    CreateDocumentRequest {
        title: title.to_string(),
        folder_id: None,
        content: None,
    }
}

// ---- CRUD ------------------------------------------------------------------

#[tokio::test]
async fn create_document_returns_201_with_generated_slug() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-crud-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Test Project".to_string(),
                slug: "test-proj-1".to_string(),
                task_prefix: "TP1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Hello World"))
        .await
        .expect("create document");

    assert_eq!(doc.title, "Hello World");
    assert!(
        doc.slug.as_deref() == Some("hello-world"),
        "slug must be server-generated from title, got: {:?}",
        doc.slug
    );
    assert_eq!(doc.workspace_id, ws.id.0);
    assert_eq!(doc.project_id, Some(project.id));

    db.teardown().await;
}

#[tokio::test]
async fn create_ignores_client_slug_field() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-crud-2").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-2".to_string(),
                task_prefix: "P2".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let req = CreateDocumentRequest {
        title: "My Doc".to_string(),
        folder_id: None,
        content: None,
    };

    let doc = client
        .create_document(&ws.slug, &project.slug, req)
        .await
        .expect("create document");

    assert_eq!(
        doc.slug.as_deref(),
        Some("my-doc"),
        "slug must be server-generated, not client-supplied"
    );

    db.teardown().await;
}

#[tokio::test]
async fn get_document_returns_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-get-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-get-1".to_string(),
                task_prefix: "PG1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let created = client
        .create_document(&ws.slug, &project.slug, doc_req("Fetch Me"))
        .await
        .expect("create document");

    let slug = created.slug.as_deref().expect("slug");
    let fetched = client
        .get_document(&ws.slug, slug)
        .await
        .expect("get document");

    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.title, "Fetch Me");

    db.teardown().await;
}

#[tokio::test]
async fn get_unknown_document_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-404-1").await;

    let result = client.get_document(&ws.slug, "does-not-exist").await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "unknown slug must return 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_documents_returns_created_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-list-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-list-1".to_string(),
                task_prefix: "PL1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    client
        .create_document(&ws.slug, &project.slug, doc_req("Listed Doc"))
        .await
        .expect("create document");

    let page = client
        .list_documents(&ws.slug, &project.slug, None, None)
        .await
        .expect("list documents");

    assert!(
        page.items.iter().any(|d| d.title == "Listed Doc"),
        "created document must appear in list"
    );

    db.teardown().await;
}

#[tokio::test]
async fn update_document_changes_title() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-upd-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-upd-1".to_string(),
                task_prefix: "PU1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Old Title"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let updated = client
        .update_document(
            &ws.slug,
            slug,
            UpdateDocumentRequest {
                title: Some("New Title".to_string()),
                folder_id: None,
            },
        )
        .await
        .expect("update document");

    assert_eq!(updated.title, "New Title");
    assert_eq!(updated.slug, doc.slug, "rename must not change the slug");

    db.teardown().await;
}

#[tokio::test]
async fn delete_document_soft_deletes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-del-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-del-1".to_string(),
                task_prefix: "PD1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("To Delete"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    client
        .delete_document(&ws.slug, slug)
        .await
        .expect("delete document");

    let result = client.get_document(&ws.slug, slug).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "deleted document must return 404 on get, got: {result:?}"
    );

    db.teardown().await;
}

// ---- Slug collision --------------------------------------------------------

#[tokio::test]
async fn slug_collision_appends_numeric_suffix() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-slug-col").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-slug-col".to_string(),
                task_prefix: "PSC".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc1 = client
        .create_document(&ws.slug, &project.slug, doc_req("Collision"))
        .await
        .expect("create first document");

    let doc2 = client
        .create_document(&ws.slug, &project.slug, doc_req("Collision"))
        .await
        .expect("create second document");

    assert_eq!(doc1.slug.as_deref(), Some("collision"));
    assert_eq!(doc2.slug.as_deref(), Some("collision-2"));

    db.teardown().await;
}

// ---- Rename stability ------------------------------------------------------

#[tokio::test]
async fn rename_does_not_change_slug() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-rename-stab").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-rename-stab".to_string(),
                task_prefix: "PRS".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Original Title"))
        .await
        .expect("create document");

    let original_slug = doc.slug.clone();

    let updated = client
        .update_document(
            &ws.slug,
            original_slug.as_deref().expect("slug"),
            UpdateDocumentRequest {
                title: Some("Completely Different Title".to_string()),
                folder_id: None,
            },
        )
        .await
        .expect("update document");

    assert_eq!(
        updated.slug, original_slug,
        "slug must be stable after rename"
    );

    db.teardown().await;
}

// ---- CAS content updates ---------------------------------------------------

#[tokio::test]
async fn update_content_succeeds_with_matching_base_revision() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-cas-ok").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-cas-ok".to_string(),
                task_prefix: "PCO".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("CAS Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let updated = client
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "new content".to_string(),
                base_revision_id: doc.head_revision_id,
            },
        )
        .await
        .expect("update content");

    assert_eq!(updated.content, "new content");
    assert_ne!(updated.head_revision_id, doc.head_revision_id);

    db.teardown().await;
}

#[tokio::test]
async fn update_content_stale_base_returns_409() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-cas-409").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-cas-409".to_string(),
                task_prefix: "PC9".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Stale CAS Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let stale_revision_id = doc.head_revision_id;

    client
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "first update".to_string(),
                base_revision_id: stale_revision_id,
            },
        )
        .await
        .expect("first update succeeds");

    let result = client
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "concurrent update".to_string(),
                base_revision_id: stale_revision_id,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 409),
        "stale base revision must return 409, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn update_content_empty_string_is_valid() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-cas-empty").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-cas-empty".to_string(),
                task_prefix: "PCE".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Empty Content Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let updated = client
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "".to_string(),
                base_revision_id: doc.head_revision_id,
            },
        )
        .await
        .expect("empty content must be accepted");

    assert_eq!(updated.content, "");

    db.teardown().await;
}

// ---- History & revisions ---------------------------------------------------

#[tokio::test]
async fn history_shows_actor_and_is_newest_first() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-hist-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-hist-1".to_string(),
                task_prefix: "PH1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("History Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    client
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "v2".to_string(),
                base_revision_id: doc.head_revision_id,
            },
        )
        .await
        .expect("update to v2");

    let history = client
        .list_document_history(&ws.slug, slug, None, None)
        .await
        .expect("list history");

    assert!(
        history.items.len() >= 2,
        "must have at least 2 revisions, got: {}",
        history.items.len()
    );

    assert!(
        history.items[0].seq >= history.items[1].seq,
        "history must be returned newest-first"
    );

    assert!(
        history.items[0].actor.is_some(),
        "revision must carry actor attribution"
    );

    db.teardown().await;
}

#[tokio::test]
async fn get_revision_content_returns_historical_content() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-rev-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-rev-1".to_string(),
                task_prefix: "PR1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Rev Doc".to_string(),
                folder_id: None,
                content: Some("initial content".to_string()),
            },
        )
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    client
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "updated content".to_string(),
                base_revision_id: doc.head_revision_id,
            },
        )
        .await
        .expect("update content");

    let rev1 = client
        .get_revision_content(&ws.slug, slug, 1)
        .await
        .expect("get revision 1");

    assert_eq!(rev1.seq, 1);
    assert_eq!(rev1.content, "initial content");

    db.teardown().await;
}

// ---- Backlinks -------------------------------------------------------------

#[tokio::test]
async fn backlinks_appear_after_wikilink_write() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-back-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-back-1".to_string(),
                task_prefix: "PB1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let target = client
        .create_document(&ws.slug, &project.slug, doc_req("Target Doc"))
        .await
        .expect("create target");

    let source = client
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Source Doc".to_string(),
                folder_id: None,
                content: Some("See [[Target Doc]] for details.".to_string()),
            },
        )
        .await
        .expect("create source");

    let target_slug = target.slug.as_deref().expect("target slug");
    let backlinks = client
        .list_backlinks(&ws.slug, target_slug, None, None)
        .await
        .expect("list backlinks");

    assert!(
        backlinks
            .items
            .iter()
            .any(|b| b.source_document_id == source.id),
        "source doc must appear as a backlink of target"
    );

    db.teardown().await;
}

#[tokio::test]
async fn broken_wikilink_is_stored_without_target() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-brok-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-brok-1".to_string(),
                task_prefix: "PBR".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Broken Links Doc".to_string(),
                folder_id: None,
                content: Some("See [[Nonexistent Page]] here.".to_string()),
            },
        )
        .await
        .expect("create document with broken wikilink");

    assert_eq!(
        doc.title, "Broken Links Doc",
        "doc must be created successfully even with broken wikilinks"
    );

    db.teardown().await;
}

// ---- Frontmatter -----------------------------------------------------------

#[tokio::test]
async fn frontmatter_extracted_from_yaml_block() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-fm-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-fm-1".to_string(),
                task_prefix: "PFM".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "FM Doc".to_string(),
                folder_id: None,
                content: Some("---\nauthor: alice\ntags: [a, b]\n---\nBody text.".to_string()),
            },
        )
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let fm = client
        .get_frontmatter(&ws.slug, slug)
        .await
        .expect("get frontmatter");

    assert_eq!(
        fm.data["author"],
        serde_json::json!("alice"),
        "frontmatter author must be extracted"
    );

    db.teardown().await;
}

// ---- Attachments -----------------------------------------------------------

#[tokio::test]
async fn attach_image_and_download_roundtrip() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-att-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-att-1".to_string(),
                task_prefix: "PA1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Attach Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let payload = b"fake-png-bytes-1234".to_vec();

    let att = client
        .upload_attachment(&ws.slug, slug, "image.png", "image/png", payload.clone())
        .await
        .expect("upload attachment");

    assert_eq!(att.file_name, "image.png");
    assert_eq!(att.content_type, "image/png");
    assert_eq!(att.size_bytes, payload.len() as i64);

    let downloaded = client
        .download_attachment(&ws.slug, att.id)
        .await
        .expect("download attachment");

    assert_eq!(downloaded, payload);

    db.teardown().await;
}

#[tokio::test]
async fn list_attachments_returns_uploaded_attachment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-att-list").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-att-list".to_string(),
                task_prefix: "PAL".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Attach List Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    client
        .upload_attachment(&ws.slug, slug, "file.txt", "text/plain", b"hello".to_vec())
        .await
        .expect("upload");

    let page = client
        .list_attachments(&ws.slug, slug, None, None)
        .await
        .expect("list attachments");

    assert!(
        page.items.iter().any(|a| a.file_name == "file.txt"),
        "uploaded attachment must appear in list"
    );

    db.teardown().await;
}

#[tokio::test]
async fn delete_attachment_removes_it_from_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-att-del").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-att-del".to_string(),
                task_prefix: "PAD".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Del Attach Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let att = client
        .upload_attachment(&ws.slug, slug, "del.txt", "text/plain", b"bye".to_vec())
        .await
        .expect("upload");

    client
        .delete_attachment(&ws.slug, att.id)
        .await
        .expect("delete attachment");

    let result = client.download_attachment(&ws.slug, att.id).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "deleted attachment must return 404 on download, got: {result:?}"
    );

    db.teardown().await;
}

// ---- Move document ---------------------------------------------------------

#[tokio::test]
async fn move_document_changes_folder() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-move-1").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-move-1".to_string(),
                task_prefix: "PM1".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Move Me"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let moved = client
        .move_document(&ws.slug, slug, MoveDocumentRequest { folder_id: None })
        .await
        .expect("move document");

    assert_eq!(moved.id, doc.id);
    assert_eq!(moved.folder_id, None);

    db.teardown().await;
}

// ---- Permissions -----------------------------------------------------------

#[tokio::test]
async fn viewer_cannot_create_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "doc-perm-owner").await;

    let project = owner
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-perm-v".to_string(),
                task_prefix: "PPV".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let hash = atlas_server::auth::password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    let viewer_user = db
        .user_repo()
        .create(NewUser {
            username: "doc-perm-viewer".to_string(),
            display_name: "Viewer".to_string(),
            password_hash: hash,
            is_root: false,
        })
        .await
        .expect("create viewer");

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(viewer_user.id));
    db.membership_repo()
        .add(&ctx, viewer_user.id, MemberRole::Member)
        .await
        .expect("add viewer membership");

    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_domain::ids::ProjectId;
    use atlas_domain::permissions::ResourceRole;
    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(viewer_user.id),
            api_key_id: None,
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Viewer,
            created_by_user_id: None,
            created_by_api_key_id: None,
        })
        .await
        .expect("grant viewer role");

    let mut viewer_client = atlas_client::AtlasClient::new(server.base_url().to_string());
    viewer_client
        .login(atlas_api::dtos::LoginRequest {
            username: "doc-perm-viewer".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("viewer login");

    let result = viewer_client
        .create_document(&ws.slug, &project.slug, doc_req("Forbidden"))
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "viewer must not create documents (expected 403), got: {result:?}"
    );

    db.teardown().await;
}

// ---- API-key actor ---------------------------------------------------------

#[tokio::test]
async fn api_key_actor_write_sets_actor_type_api_key() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "doc-ak-write").await;

    let project = owner
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-ak-write".to_string(),
                task_prefix: "PAK".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = owner
        .create_document(&ws.slug, &project.slug, doc_req("API Key Doc"))
        .await
        .expect("create document");

    let key_created = owner
        .create_api_key(
            &ws.slug,
            CreateApiKeyRequest {
                name: "test-key".to_string(),
                expires_at: None,
            },
        )
        .await
        .expect("create api key");

    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_domain::ids::{ApiKeyId, ProjectId};
    use atlas_domain::permissions::ResourceRole;
    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(ApiKeyId(key_created.id)),
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("grant api key editor role");

    let agent_client =
        atlas_client::AtlasClient::new(server.base_url()).with_token(key_created.secret.clone());

    let slug = doc.slug.as_deref().expect("slug");
    let updated = agent_client
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "api key wrote this".to_string(),
                base_revision_id: doc.head_revision_id,
            },
        )
        .await
        .expect("api key update content must succeed");

    assert_eq!(updated.content, "api key wrote this");

    let history = agent_client
        .list_document_history(&ws.slug, slug, None, None)
        .await
        .expect("list history");

    let head_rev = history.items.first().expect("at least one revision");
    let actor = head_rev.actor.as_ref().expect("revision must carry actor");
    assert_eq!(actor.r#type, "api_key", "actor type must be api_key");

    db.teardown().await;
}

// ---- Cross-tenant isolation ------------------------------------------------

#[tokio::test]
async fn cross_tenant_get_document_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (alice, _ws_a, _) = support::login_user_with_workspace(&server, &db, "doc-ct-alice").await;
    let (bob, ws_b, _) = support::login_user_with_workspace(&server, &db, "doc-ct-bob").await;

    let proj_b = bob
        .create_project(
            &ws_b.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Bob Proj".to_string(),
                slug: "proj-ct-bob".to_string(),
                task_prefix: "CTB".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("bob creates project");

    let doc_b = bob
        .create_document(&ws_b.slug, &proj_b.slug, doc_req("Bob's Secret"))
        .await
        .expect("bob creates document");

    let slug = doc_b.slug.as_deref().expect("slug");

    let result = alice.get_document(&ws_b.slug, slug).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant document read must return 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn cross_tenant_download_attachment_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (alice, _ws_a, _) =
        support::login_user_with_workspace(&server, &db, "doc-ct-att-alice").await;
    let (bob, ws_b, _) = support::login_user_with_workspace(&server, &db, "doc-ct-att-bob").await;

    let proj_b = bob
        .create_project(
            &ws_b.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Bob Proj".to_string(),
                slug: "proj-ct-att-bob".to_string(),
                task_prefix: "CTA".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("bob creates project");

    let doc_b = bob
        .create_document(&ws_b.slug, &proj_b.slug, doc_req("Bob's Attach Doc"))
        .await
        .expect("bob creates document");

    let slug = doc_b.slug.as_deref().expect("slug");
    let att = bob
        .upload_attachment(
            &ws_b.slug,
            slug,
            "secret.txt",
            "text/plain",
            b"secret".to_vec(),
        )
        .await
        .expect("bob uploads attachment");

    let result = alice.download_attachment(&ws_b.slug, att.id).await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "cross-tenant attachment download must return 404, got: {result:?}"
    );

    db.teardown().await;
}

// ---- Oversized attachment returns 413 --------------------------------------

#[tokio::test]
async fn oversized_attachment_returns_413() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let base_state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let state = base_state.with_max_attachment_bytes(16);
    let server = support::TestServer::spawn_with_state(state).await;

    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-att-413").await;

    let project = client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-att-413".to_string(),
                task_prefix: "P413".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .create_document(&ws.slug, &project.slug, doc_req("Oversized Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");

    let result = client
        .upload_attachment(
            &ws.slug,
            slug,
            "big.bin",
            "application/octet-stream",
            vec![0u8; 32],
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 413),
        "oversized attachment must return 413, got: {result:?}"
    );

    db.teardown().await;
}
