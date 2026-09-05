#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use sea_orm::{ConnectionTrait, Statement};

use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::identity::MemberRole;
use atlas_acta_postgres::repos::identity::MembershipRepo;
use atlas_api::dtos::{
    ApiKeyScope, CreateUserApiKeyRequest,
    documents::{
        CreateDocumentRequest, DocumentCompactDto, DocumentContentEditRequest,
        DocumentContentRangeDto, DocumentContentSearchDto, DocumentContentSearchRequest,
        DocumentLineEditRequest, DocumentSearchMode, MoveDocumentRequest, UpdateContentRequest,
        UpdateDocumentRequest,
    },
};
use atlas_client::ClientError;
use atlas_server::persistence::repos::{FolderRepo, NewUser, PermissionGrantRepo, UserRepo};

fn doc_req(title: &str) -> CreateDocumentRequest {
    CreateDocumentRequest {
        title: title.to_string(),
        folder_id: None,
        content: None,
    }
}

fn tamper_continuation(mut continuation: String) -> String {
    let replacement = if continuation.starts_with('A') {
        "B"
    } else {
        "A"
    };
    continuation.replace_range(..1, replacement);
    continuation
}

async fn post_document_moves_batch(
    server: &support::TestServer,
    client: &atlas_client::AtlasClient,
    workspace: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    reqwest::Client::new()
        .post(support::path::api_url(
            server.base_url(),
            "acta",
            &format!("/workspaces/{workspace}/documents/moves/batch"),
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&body)
        .send()
        .await
        .expect("document move batch request")
}
macro_rules! edit_content_range {
    ($server:expr, $client:expr, $workspace:expr, $slug:expr, $base:expr, $edit:expr $(,)?) => {
        reqwest::Client::new()
            .patch(support::path::api_url(
                $server.base_url(),
                "acta",
                &format!(
                    "/workspaces/{}/documents/{}/content/range",
                    $workspace, $slug
                ),
            ))
            .bearer_auth($client.token().expect("authenticated token"))
            .json(&DocumentContentEditRequest {
                base_revision_id: $base,
                edit: $edit,
            })
            .send()
            .await
            .expect("partial edit request")
    };
}
// ---- CRUD ------------------------------------------------------------------

#[tokio::test]
async fn create_document_returns_201_with_generated_slug() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-crud-1").await;

    let project = client
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Fetch Me"))
        .await
        .expect("create document");

    let slug = created.slug.as_deref().expect("slug");
    let fetched = client
        .acta()
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

    let result = client.acta().get_document(&ws.slug, "does-not-exist").await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "unknown slug must return 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn compact_document_read_omits_content_and_does_not_leak_across_workspaces() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-compact-read").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-compact-read".to_string(),
                task_prefix: "PCR".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let doc = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Compact read".to_string(),
                folder_id: None,
                content: Some("content that must not be transferred".repeat(1024)),
            },
        )
        .await
        .expect("create document");
    let slug = doc.slug.as_deref().expect("slug");
    let token = client.token().expect("authenticated token");
    let http = reqwest::Client::new();

    let compact = http
        .get(support::path::api_url(
            server.base_url(),
            "acta",
            &format!("/workspaces/{}/documents/{slug}/compact", ws.slug),
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("compact request");
    assert_eq!(compact.status(), reqwest::StatusCode::OK);
    let compact = compact
        .json::<serde_json::Value>()
        .await
        .expect("compact response");
    assert_eq!(compact["title"], "Compact read");
    assert!(compact.get("content").is_none());

    let (_other_client, other_ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-compact-other").await;
    let hidden = http
        .get(support::path::api_url(
            server.base_url(),
            "acta",
            &format!("/workspaces/{}/documents/{slug}/compact", other_ws.slug),
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("cross-workspace compact request");
    assert_eq!(hidden.status(), reqwest::StatusCode::NOT_FOUND);

    db.teardown().await;
}

#[tokio::test]
async fn compact_document_read_previews_the_body_without_frontmatter() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-compact-preview").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-compact-preview".to_string(),
                task_prefix: "PCP".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let long = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Long".to_string(),
                folder_id: None,
                content: Some(format!(
                    "---\ntitle: Long\nowner: platform\n---\n\n{}",
                    "a".repeat(500)
                )),
            },
        )
        .await
        .expect("create long document");
    let multi_byte = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Multi byte".to_string(),
                folder_id: None,
                content: Some("héllo wörld ☃️".repeat(60)),
            },
        )
        .await
        .expect("create multi-byte document");
    let empty = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Empty"))
        .await
        .expect("create empty document");

    let long_compact = client
        .acta()
        .get_document_compact(&ws.slug, long.slug.as_deref().expect("slug"))
        .await
        .expect("compact long document");
    let long_preview = long_compact.preview.expect("preview for long document");
    assert_eq!(long_preview, "a".repeat(200));

    let multi_byte_compact = client
        .acta()
        .get_document_compact(&ws.slug, multi_byte.slug.as_deref().expect("slug"))
        .await
        .expect("compact multi-byte document");
    let multi_byte_preview = multi_byte_compact
        .preview
        .expect("preview for multi-byte document");
    assert_eq!(multi_byte_preview.chars().count(), 200);
    assert!(multi_byte_preview.len() > 200, "preview must be multi-byte");
    assert!(multi_byte.content.starts_with(&multi_byte_preview));

    let empty_slug = empty.slug.as_deref().expect("slug");
    let empty_compact = reqwest::Client::new()
        .get(support::path::api_url(
            server.base_url(),
            "acta",
            &format!("/workspaces/{}/documents/{empty_slug}/compact", ws.slug),
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("compact empty document request")
        .json::<serde_json::Value>()
        .await
        .expect("compact empty document response");
    assert!(
        empty_compact.get("preview").is_none(),
        "an empty body must not produce a preview"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_documents_returns_previews_only_when_requested() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-list-preview").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-list-preview".to_string(),
                task_prefix: "PLP".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Listed".to_string(),
                folder_id: None,
                content: Some("---\nowner: platform\n---\n\nBody signal".to_string()),
            },
        )
        .await
        .expect("create document");

    let without_preview = client
        .acta()
        .list_documents(&ws.slug, &project.slug, None, None)
        .await
        .expect("list documents");
    assert!(
        without_preview
            .items
            .iter()
            .all(|document| document.preview.is_none()),
        "listings must stay cheap unless preview is requested"
    );

    let with_preview = client
        .acta()
        .list_documents_with_options(&ws.slug, &project.slug, None, None, None, true)
        .await
        .expect("list documents with preview");
    let listed = with_preview
        .items
        .iter()
        .find(|document| document.title == "Listed")
        .expect("listed document");
    assert_eq!(listed.preview.as_deref(), Some("Body signal"));

    db.teardown().await;
}

#[tokio::test]
async fn range_read_pages_losslessly_and_rejects_too_small_or_stale_continuations() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-range-read").await;
    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj-range-read".into(),
                task_prefix: "PRR".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let doc = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Bounded read".into(),
                folder_id: None,
                content: Some(format!("one\r\ntwo\n{}\n😀", "é".repeat(40))),
            },
        )
        .await
        .expect("create document");
    let slug = doc.slug.as_deref().expect("slug");
    let token = client.token().expect("authenticated token");
    let url = support::path::api_url(
        server.base_url(),
        "acta",
        &format!("/workspaces/{}/documents/{slug}/content/range", ws.slug),
    );
    let http = reqwest::Client::new();

    for (line, byte_limit) in [(3, 1), (4, 3)] {
        let response = http
            .get(format!(
                "{url}?start_line={line}&end_line={line}&byte_limit={byte_limit}"
            ))
            .bearer_auth(token)
            .send()
            .await
            .expect("too-small UTF-8 range request");
        assert_eq!(response.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    }

    let first = http
        .get(format!(
            "{url}?start_line=1&end_line=4&line_limit=2&byte_limit=64"
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("first range request");
    let first: DocumentContentRangeDto = first.json().await.expect("first range response");
    assert_eq!(
        first
            .lines
            .iter()
            .map(|line| (line.line_number, line.text.as_str()))
            .collect::<Vec<_>>(),
        [(1, "one"), (2, "two")]
    );
    let continuation = first.continuation.expect("continuation after line limit");

    let stale = continuation.clone();
    let mut continuation = Some(continuation);
    let mut long_line = String::new();
    while let Some(cursor) = continuation.take() {
        let response = http
            .get(format!("{url}?continuation={cursor}"))
            .bearer_auth(client.token().expect("authenticated token"))
            .send()
            .await
            .expect("continued range request");
        let page: DocumentContentRangeDto = response.json().await.expect("range response");
        for line in page.lines.iter().filter(|line| line.line_number == 3) {
            long_line.push_str(&line.text);
        }
        continuation = page.continuation;
    }
    assert_eq!(long_line, "é".repeat(40));

    client
        .acta()
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "changed".into(),
                base_revision_id: doc.head_revision_id,
            },
        )
        .await
        .expect("change document head");
    let stale = http
        .get(format!("{url}?continuation={stale}"))
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("stale continuation request");
    assert_eq!(stale.status(), reqwest::StatusCode::CONFLICT);

    db.teardown().await;
}

#[tokio::test]
async fn content_search_enforces_scan_and_utf8_page_boundaries() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-search").await;
    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj-search".into(),
                task_prefix: "PSR".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let doc = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Search".into(),
                folder_id: None,
                content: Some(format!("{}match", "skip\n".repeat(209_716))),
            },
        )
        .await
        .expect("create document");
    let slug = doc.slug.as_deref().expect("slug");
    let url = support::path::api_url(
        server.base_url(),
        "acta",
        &format!("/workspaces/{}/documents/{slug}/content/search", ws.slug),
    );
    let http = reqwest::Client::new();
    let first = http
        .post(&url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&DocumentContentSearchRequest {
            query: "match".into(),
            ..Default::default()
        })
        .send()
        .await
        .expect("scan-capped search");
    let first: DocumentContentSearchDto = first.json().await.expect("scan-cap page");
    assert!(first.matches.is_empty());
    let continuation = first.continuation.expect("scan-cap continuation");
    let resumed = http
        .post(&url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&DocumentContentSearchRequest {
            query: "match".into(),
            continuation: Some(continuation),
            ..Default::default()
        })
        .send()
        .await
        .expect("resumed scan-capped search");
    let resumed: DocumentContentSearchDto = resumed.json().await.expect("resumed scan-cap page");
    assert_eq!(resumed.matches.len(), 1);
    assert_eq!(resumed.matches[0].line_number, 209_717);
    assert_eq!(resumed.matches[0].preview, "match");
    assert!(resumed.continuation.is_none());
    client
        .acta()
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "a\né".into(),
                base_revision_id: doc.head_revision_id,
            },
        )
        .await
        .expect("replace content for UTF-8 boundary");
    let first = http
        .post(&url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&DocumentContentSearchRequest {
            query: ".".into(),
            mode: Some(DocumentSearchMode::Pattern),
            byte_limit: Some(1),
            ..Default::default()
        })
        .send()
        .await
        .expect("UTF-8 boundary search");
    let first: DocumentContentSearchDto = first.json().await.expect("UTF-8 boundary response");
    assert_eq!(first.matches.len(), 1);
    assert_eq!(first.matches[0].preview, "a");
    let continuation = first.continuation.expect("continuation at UTF-8 boundary");
    let blocked = http
        .post(&url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&DocumentContentSearchRequest {
            query: ".".into(),
            continuation: Some(continuation),
            ..Default::default()
        })
        .send()
        .await
        .expect("non-progress guard search");
    assert_eq!(blocked.status(), reqwest::StatusCode::UNPROCESSABLE_ENTITY);
    db.teardown().await;
}

#[tokio::test]
async fn tampered_document_continuations_are_rejected_without_leaking_or_mutating_state() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-tamper").await;
    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj-tamper".into(),
                task_prefix: "PTM".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let content = "range first\nrange second\nneedle first\nneedle second";
    let doc = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Tamper continuations".into(),
                folder_id: None,
                content: Some(content.into()),
            },
        )
        .await
        .expect("create document");
    let slug = doc.slug.as_deref().expect("slug");
    let token = client.token().expect("authenticated token");
    let http = reqwest::Client::new();
    let range_url = support::path::api_url(
        server.base_url(),
        "acta",
        &format!("/workspaces/{}/documents/{slug}/content/range", ws.slug),
    );
    let search_url = support::path::api_url(
        server.base_url(),
        "acta",
        &format!("/workspaces/{}/documents/{slug}/content/search", ws.slug),
    );

    let range = http
        .get(format!(
            "{range_url}?start_line=1&end_line=2&line_limit=1&byte_limit=64"
        ))
        .bearer_auth(token)
        .send()
        .await
        .expect("issue range continuation");
    assert_eq!(range.status(), reqwest::StatusCode::OK);
    let range: DocumentContentRangeDto = range.json().await.expect("range response");
    let range_continuation = range.continuation.expect("range continuation");

    let rejected_range = http
        .get(format!(
            "{range_url}?continuation={}",
            tamper_continuation(range_continuation.clone())
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("tampered range continuation request");
    assert_eq!(rejected_range.status(), reqwest::StatusCode::BAD_REQUEST);
    let rejected_range: serde_json::Value =
        rejected_range.json().await.expect("range problem details");
    assert_eq!(rejected_range["type"], "urn:atlas:error:bad-request");
    assert_eq!(rejected_range["title"], "Bad Request");
    assert!(rejected_range.get("lines").is_none());
    assert!(rejected_range.get("content").is_none());

    let resumed_range = http
        .get(format!("{range_url}?continuation={range_continuation}"))
        .bearer_auth(client.token().expect("authenticated token"))
        .send()
        .await
        .expect("resume untampered range continuation");
    assert_eq!(resumed_range.status(), reqwest::StatusCode::OK);
    let resumed_range: DocumentContentRangeDto =
        resumed_range.json().await.expect("resumed range response");
    assert_eq!(resumed_range.lines[0].text, "range second");

    let search = http
        .post(&search_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&DocumentContentSearchRequest {
            query: "needle".into(),
            match_limit: Some(1),
            ..Default::default()
        })
        .send()
        .await
        .expect("issue search continuation");
    assert_eq!(search.status(), reqwest::StatusCode::OK);
    let search: DocumentContentSearchDto = search.json().await.expect("search response");
    let search_continuation = search.continuation.expect("search continuation");

    let rejected_search = http
        .post(&search_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&DocumentContentSearchRequest {
            query: "needle".into(),
            continuation: Some(tamper_continuation(search_continuation.clone())),
            ..Default::default()
        })
        .send()
        .await
        .expect("tampered search continuation request");
    assert_eq!(rejected_search.status(), reqwest::StatusCode::BAD_REQUEST);
    let rejected_search: serde_json::Value = rejected_search
        .json()
        .await
        .expect("search problem details");
    assert_eq!(rejected_search["type"], "urn:atlas:error:bad-request");
    assert_eq!(rejected_search["title"], "Bad Request");
    assert!(rejected_search.get("matches").is_none());
    assert!(rejected_search.get("content").is_none());

    let resumed_search = http
        .post(&search_url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&DocumentContentSearchRequest {
            query: "needle".into(),
            continuation: Some(search_continuation),
            ..Default::default()
        })
        .send()
        .await
        .expect("resume untampered search continuation");
    assert_eq!(resumed_search.status(), reqwest::StatusCode::OK);
    let resumed_search: DocumentContentSearchDto = resumed_search
        .json()
        .await
        .expect("resumed search response");
    assert_eq!(resumed_search.matches[0].preview, "needle second");

    let current = client
        .acta()
        .get_document(&ws.slug, slug)
        .await
        .expect("read document after rejected continuations");
    assert_eq!(current.content, content);

    db.teardown().await;
}

#[tokio::test]
async fn invalid_pattern_search_returns_problem_details_without_partial_results() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-invalid-pattern").await;
    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj-invalid-pattern".into(),
                task_prefix: "PIP".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let content = "needle one\nneedle two";
    let doc = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Invalid pattern".into(),
                folder_id: None,
                content: Some(content.into()),
            },
        )
        .await
        .expect("create document");
    let slug = doc.slug.as_deref().expect("slug");
    let url = support::path::api_url(
        server.base_url(),
        "acta",
        &format!("/workspaces/{}/documents/{slug}/content/search", ws.slug),
    );

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(client.token().expect("authenticated token"))
        .json(&DocumentContentSearchRequest {
            query: "[".into(),
            mode: Some(DocumentSearchMode::Pattern),
            ..Default::default()
        })
        .send()
        .await
        .expect("invalid pattern search request");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let problem: serde_json::Value = response.json().await.expect("problem details");
    assert_eq!(problem["type"], "urn:atlas:error:bad-request");
    assert_eq!(problem["title"], "Bad Request");
    assert_eq!(problem["status"], 400);
    assert!(problem.get("matches").is_none());
    assert!(problem.get("continuation").is_none());

    let current = client
        .acta()
        .get_document(&ws.slug, slug)
        .await
        .expect("read document after invalid pattern");
    assert_eq!(current.content, content);

    db.teardown().await;
}

#[tokio::test]
async fn list_documents_returns_created_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-list-1").await;

    let project = client
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Listed Doc"))
        .await
        .expect("create document");

    let page = client
        .acta()
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
async fn list_documents_distinguishes_unfiled_from_filed_documents() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-list-folder-filter").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Project".to_string(),
                slug: "doc-list-folder-filter".to_string(),
                task_prefix: "DLF".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Unfiled"))
        .await
        .expect("create unfiled document");

    let folder = client
        .acta()
        .create_folder(
            &ws.slug,
            &project.slug,
            atlas_api::dtos::folders::CreateFolderRequest {
                name: "Folder".to_string(),
                parent_folder_id: None,
            },
        )
        .await
        .expect("create folder");

    client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Filed".to_string(),
                folder_id: Some(folder.id),
                content: None,
            },
        )
        .await
        .expect("create filed document");

    let base_url = server.base_url().to_string();
    let workspace_slug = ws.slug.clone();
    let project_slug = project.slug.clone();
    let token = client.token().expect("authenticated token").to_string();
    let list = |unfiled: Option<bool>| {
        let base_url = base_url.clone();
        let workspace_slug = workspace_slug.clone();
        let project_slug = project_slug.clone();
        let token = token.clone();

        async move {
            let suffix = match unfiled {
                Some(value) => format!("?unfiled={value}"),
                None => String::new(),
            };
            let response = reqwest::Client::new()
                .get(support::path::api_url(
                    &base_url,
                    "acta",
                    &format!(
                        "/workspaces/{workspace_slug}/projects/{project_slug}/documents{suffix}"
                    ),
                ))
                .bearer_auth(token)
                .send()
                .await
                .expect("list documents");

            assert_eq!(response.status(), reqwest::StatusCode::OK);
            response
                .json::<serde_json::Value>()
                .await
                .expect("document list response")
        }
    };

    let any_titles = list(None).await["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|document| document["title"].as_str().expect("title").to_string())
        .collect::<Vec<_>>();
    assert!(any_titles.contains(&"Unfiled".to_string()));
    assert!(any_titles.contains(&"Filed".to_string()));

    let unfiled_titles = list(Some(true)).await["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|document| document["title"].as_str().expect("title").to_string())
        .collect::<Vec<_>>();
    assert_eq!(unfiled_titles, vec!["Unfiled"]);

    let filed_titles = list(Some(false)).await["items"]
        .as_array()
        .expect("items array")
        .iter()
        .map(|document| document["title"].as_str().expect("title").to_string())
        .collect::<Vec<_>>();
    assert_eq!(filed_titles, vec!["Filed"]);

    db.teardown().await;
}

#[tokio::test]
async fn root_user_lists_documents_in_workspace_without_membership() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner_client, ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-list-root").await;
    let root_client = support::login_root_user(&server, &db).await;

    let project = owner_client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Shared Project".to_string(),
                slug: "shared-project".to_string(),
                task_prefix: "SPR".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    owner_client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Visible to Root"))
        .await
        .expect("create document");

    let page = root_client
        .acta()
        .list_documents(&ws.slug, &project.slug, None, None)
        .await
        .expect("root list documents");

    assert!(
        page.items.iter().any(|d| d.title == "Visible to Root"),
        "root/system admin users must see workspace documents without a membership row"
    );

    db.teardown().await;
}

#[tokio::test]
async fn update_document_changes_title() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-upd-1").await;

    let project = client
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Old Title"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let updated = client
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("To Delete"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    client
        .acta()
        .delete_document(&ws.slug, slug)
        .await
        .expect("delete document");

    let result = client.acta().get_document(&ws.slug, slug).await;
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Collision"))
        .await
        .expect("create first document");

    let doc2 = client
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Original Title"))
        .await
        .expect("create document");

    let original_slug = doc.slug.clone();

    let updated = client
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("CAS Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let updated = client
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Stale CAS Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let stale_revision_id = doc.head_revision_id;

    client
        .acta()
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
        .acta()
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "concurrent update".to_string(),
                base_revision_id: stale_revision_id,
            },
        )
        .await;

    let conflict = match result {
        Err(ClientError::Conflict(c)) => c,
        other => panic!("stale base revision must surface ClientError::Conflict, got: {other:?}"),
    };

    assert_eq!(conflict.problem.status, 409, "conflict status must be 409");
    assert_eq!(
        conflict.current_seq, 2,
        "conflict must carry the current head seq"
    );
    assert!(
        !conflict.base_to_current_patch.is_empty(),
        "conflict must carry the base-to-current patch so the caller can rebase"
    );

    db.teardown().await;
}

#[tokio::test]
async fn update_content_unknown_base_revision_is_rejected_not_conflict() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-cas-bogus").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-cas-bogus".to_string(),
                task_prefix: "PCG".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Bogus Base Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let result = client
        .acta()
        .update_content(
            &ws.slug,
            slug,
            UpdateContentRequest {
                content: "new content".to_string(),
                base_revision_id: uuid::Uuid::now_v7(),
            },
        )
        .await;

    match result {
        Err(ClientError::Api(p)) => assert!(
            p.status == 422 || p.status == 404,
            "unknown base revision must be rejected as 422/404, got: {}",
            p.status
        ),
        other => panic!(
            "unknown base revision must be a client/validation error, not a conflict, got: {other:?}"
        ),
    }

    db.teardown().await;
}

#[tokio::test]
async fn update_content_empty_string_is_valid() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-cas-empty").await;

    let project = client
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Empty Content Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let updated = client
        .acta()
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

#[tokio::test]
async fn edit_content_range_updates_content_and_returns_a_new_revision() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-edit-range").await;
    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".into(),
                slug: "proj-edit-range".into(),
                task_prefix: "PER".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let target = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Edit target"))
        .await
        .expect("create target");
    let doc = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Partial edit".into(),
                folder_id: None,
                content: Some("---\nkind: guide\n---\nfirst\r\nsecond [[Edit target]]".into()),
            },
        )
        .await
        .expect("create document");
    let slug = doc.slug.as_deref().expect("slug");
    let response = edit_content_range!(
        &server,
        &client,
        &ws.slug,
        slug,
        doc.head_revision_id,
        DocumentLineEditRequest::Insert {
            position: 5,
            content: "inserted".into()
        },
    );
    let updated: serde_json::Value = response.json().await.expect("partial edit response");
    assert!(updated.get("content").is_none());
    assert_eq!(updated["frontmatter"], serde_json::json!({"kind": "guide"}));
    let updated = client
        .acta()
        .get_document(&ws.slug, slug)
        .await
        .expect("read edited document");
    assert_eq!(
        updated.content,
        "---\nkind: guide\n---\nfirst\r\ninserted\nsecond [[Edit target]]"
    );
    assert_eq!(updated.frontmatter, serde_json::json!({"kind": "guide"}));
    assert_ne!(updated.head_revision_id, doc.head_revision_id);
    let backlinks = client
        .acta()
        .list_backlinks(
            &ws.slug,
            target.slug.as_deref().expect("target slug"),
            None,
            None,
        )
        .await
        .expect("list backlinks");
    assert!(
        backlinks
            .items
            .iter()
            .any(|backlink| backlink.source_document_id == doc.id)
    );
    let guard_doc = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Edit guards".into(),
                folder_id: None,
                content: Some("first\nsecond [[Edit target]]".into()),
            },
        )
        .await
        .expect("create document");
    let slug = guard_doc.slug.as_deref().expect("slug");
    let links_before = document_source_link_identity_rows(&db, guard_doc.id).await;
    let no_op = edit_content_range!(
        &server,
        &client,
        &ws.slug,
        slug,
        guard_doc.head_revision_id,
        DocumentLineEditRequest::Delete { start: 3, end: 2 },
    );
    let no_op: serde_json::Value = no_op.json().await.expect("no-op response");
    assert!(no_op.get("content").is_none());
    let no_op = client
        .acta()
        .get_document(&ws.slug, slug)
        .await
        .expect("read no-op document");
    assert_eq!(no_op.content, "first\nsecond [[Edit target]]");
    assert_eq!(no_op.head_revision_id, guard_doc.head_revision_id);
    assert_eq!(no_op.head_seq, guard_doc.head_seq);
    assert_eq!(no_op.updated_at, guard_doc.updated_at);
    assert_eq!(
        document_source_link_identity_rows(&db, guard_doc.id).await,
        links_before
    );
    let changed = edit_content_range!(
        &server,
        &client,
        &ws.slug,
        slug,
        guard_doc.head_revision_id,
        DocumentLineEditRequest::Replace {
            start: 2,
            end: 2,
            content: "changed".into()
        },
    );
    let changed: serde_json::Value = changed.json().await.expect("replace response");
    assert!(changed.get("content").is_none());
    let changed = client
        .acta()
        .get_document(&ws.slug, slug)
        .await
        .expect("read replaced document");
    assert_eq!(changed.content, "first\nchanged");
    let stale = edit_content_range!(
        &server,
        &client,
        &ws.slug,
        slug,
        guard_doc.head_revision_id,
        DocumentLineEditRequest::Insert {
            position: 0,
            content: "invalid".into()
        },
    );
    let stale: serde_json::Value = stale.json().await.expect("stale response");
    assert_eq!(stale["status"], 409);
    assert_eq!(
        stale["current_revision_id"],
        changed.head_revision_id.to_string()
    );
    assert!(stale["base_to_current_patch"].is_string());
    let deleted = edit_content_range!(
        &server,
        &client,
        &ws.slug,
        slug,
        changed.head_revision_id,
        DocumentLineEditRequest::Delete { start: 2, end: 2 },
    );
    let deleted: serde_json::Value = deleted.json().await.expect("delete response");
    assert!(deleted.get("content").is_none());
    let deleted = client
        .acta()
        .get_document(&ws.slug, slug)
        .await
        .expect("read deleted document");
    assert_eq!(deleted.content, "first\n");
    db.teardown().await;
}

#[tokio::test]
async fn edit_content_range_requires_editor_access_without_leaking_other_workspaces() {
    use atlas_acta::ids::ProjectId;
    use atlas_server::authz::ResourceRole;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "doc-edit-auth").await;
    let project = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Private project".into(),
                slug: "doc-edit-auth-project".into(),
                task_prefix: "DEA".into(),
                visibility: Some("private".into()),
                visibility_role: None,
            },
        )
        .await
        .expect("create private project");
    let document = owner
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Restricted edit".into(),
                folder_id: None,
                content: Some("first".into()),
            },
        )
        .await
        .expect("create restricted document");
    let slug = document.slug.as_deref().expect("slug");
    let viewer = member_client_with_optional_project_grant(
        &server,
        &db,
        &ws,
        "doc-edit-viewer",
        Some(ProjectId(project.id)),
        Some(ResourceRole::Viewer),
    )
    .await;
    let editor = member_client_with_optional_project_grant(
        &server,
        &db,
        &ws,
        "doc-edit-editor",
        Some(ProjectId(project.id)),
        Some(ResourceRole::Editor),
    )
    .await;
    let outsider = member_client_with_optional_project_grant(
        &server,
        &db,
        &ws,
        "doc-edit-outsider",
        None,
        None,
    )
    .await;
    let edit = DocumentLineEditRequest::Replace {
        start: 1,
        end: 1,
        content: "changed".into(),
    };

    let viewer_response = edit_content_range!(
        &server,
        &viewer,
        &ws.slug,
        slug,
        document.head_revision_id,
        edit.clone(),
    );
    assert_eq!(viewer_response.status(), reqwest::StatusCode::FORBIDDEN);

    let editor_response = edit_content_range!(
        &server,
        &editor,
        &ws.slug,
        slug,
        document.head_revision_id,
        edit,
    );
    assert_eq!(editor_response.status(), reqwest::StatusCode::OK);
    let edited: DocumentCompactDto = editor_response.json().await.expect("editor response");
    let editor_document = editor
        .acta()
        .get_document(&ws.slug, slug)
        .await
        .expect("editor reads changed document");
    assert_eq!(editor_document.content, "changed");

    let inaccessible = edit_content_range!(
        &server,
        &outsider,
        &ws.slug,
        slug,
        edited.head_revision_id,
        DocumentLineEditRequest::Delete { start: 1, end: 1 },
    );
    assert_eq!(
        inaccessible.status(),
        reqwest::StatusCode::NOT_FOUND,
        "inaccessible private document must not reveal whether it exists"
    );

    let (_other_client, other_ws, _) =
        support::login_user_with_workspace(&server, &db, "doc-edit-other-workspace").await;
    let cross_workspace = edit_content_range!(
        &server,
        &owner,
        &other_ws.slug,
        slug,
        edited.head_revision_id,
        DocumentLineEditRequest::Delete { start: 1, end: 1 },
    );
    assert_eq!(cross_workspace.status(), reqwest::StatusCode::NOT_FOUND);

    db.teardown().await;
}

// ---- History & revisions ---------------------------------------------------

#[tokio::test]
async fn history_shows_actor_and_is_newest_first() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-hist-1").await;

    let project = client
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("History Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    client
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Target Doc"))
        .await
        .expect("create target");

    let source = client
        .acta()
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
        .acta()
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
        .acta()
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
        .acta()
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

/// Reads `(target_title, target_document_id)` rows for the links sourced from a
/// document, ordered by title — mirrors the task-link helper.
async fn document_source_links(
    db: &support::TestDb,
    source_document_id: uuid::Uuid,
) -> Vec<(String, Option<uuid::Uuid>)> {
    use sea_orm::{ConnectionTrait, Statement};

    let rows = db
        .conn()
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "SELECT target_title, target_document_id FROM acta.document_links \
                 WHERE source_document_id = '{source_document_id}' ORDER BY target_title"
            ),
        ))
        .await
        .expect("query document_links");

    rows.into_iter()
        .map(|r| {
            let title: String = r.try_get("", "target_title").expect("target_title");
            let doc: Option<uuid::Uuid> = r
                .try_get("", "target_document_id")
                .expect("target_document_id");
            (title, doc)
        })
        .collect()
}

async fn document_source_link_identity_rows(
    db: &support::TestDb,
    source_document_id: uuid::Uuid,
) -> Vec<(uuid::Uuid, String, Option<uuid::Uuid>)> {
    let sql = format!(
        "SELECT id, target_title, target_document_id FROM acta.document_links WHERE source_document_id = '{source_document_id}' ORDER BY id"
    );
    let rows = db
        .conn()
        .query_all_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            sql,
        ))
        .await
        .expect("query document_links");

    rows.into_iter()
        .map(|row| {
            (
                row.try_get("", "id").expect("id"),
                row.try_get("", "target_title").expect("target_title"),
                row.try_get("", "target_document_id")
                    .expect("target_document_id"),
            )
        })
        .collect()
}

/// An id-bound wikilink `[[<uuid>|Title]]` resolves to the target by its stable
/// id, independent of the display title text.
#[tokio::test]
async fn id_bound_wikilink_resolves_by_id_independent_of_title() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-idlink-1").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-idlink-1".to_string(),
                task_prefix: "PIL".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let target = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Target Doc"))
        .await
        .expect("create target");

    let source = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Source Doc".to_string(),
                folder_id: None,
                content: Some(format!(
                    "See [[{}|Totally Different Label]] now.",
                    target.id
                )),
            },
        )
        .await
        .expect("create source");

    let links = document_source_links(&db, source.id).await;
    assert_eq!(
        links,
        vec![("Totally Different Label".to_string(), Some(target.id))],
        "id-bound link must resolve to the target id and keep the display title"
    );

    db.teardown().await;
}

/// An id-bound wikilink whose UUID does not exist in the workspace is stored as
/// a pending link (target_document_id NULL), not dropped, not an error.
#[tokio::test]
async fn id_bound_wikilink_to_missing_doc_is_pending() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-idlink-2").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-idlink-2".to_string(),
                task_prefix: "PIM".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let missing_id = uuid::Uuid::now_v7();

    let source = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Source Doc".to_string(),
                folder_id: None,
                content: Some(format!("See [[{missing_id}|Ghost Doc]] here.")),
            },
        )
        .await
        .expect("create source");

    let links = document_source_links(&db, source.id).await;
    assert_eq!(
        links,
        vec![("Ghost Doc".to_string(), None)],
        "id-bound link to an unknown doc must persist as pending"
    );

    db.teardown().await;
}

/// A legacy `[[Title]]` wikilink (no `|`) still resolves by title-slug.
#[tokio::test]
async fn legacy_wikilink_still_resolves_by_slug() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-idlink-3").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-idlink-3".to_string(),
                task_prefix: "PIS".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let target = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Target Doc"))
        .await
        .expect("create target");

    let source = client
        .acta()
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

    let links = document_source_links(&db, source.id).await;
    assert_eq!(
        links,
        vec![("Target Doc".to_string(), Some(target.id))],
        "legacy link must resolve by slug to the target"
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
        .acta()
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
        .acta()
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

    assert_eq!(
        doc.updated_at, doc.created_at,
        "a freshly created document must not have updated_at skewed past created_at"
    );
    assert_eq!(
        doc.frontmatter["author"],
        serde_json::json!("alice"),
        "create response must already carry parsed frontmatter (single persist)"
    );

    let slug = doc.slug.as_deref().expect("slug");
    let fm = client
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Attach Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let payload = b"fake-png-bytes-1234".to_vec();

    let att = client
        .acta()
        .upload_attachment(&ws.slug, slug, "image.png", "image/png", payload.clone())
        .await
        .expect("upload attachment");

    assert_eq!(att.file_name, "image.png");
    assert_eq!(att.content_type, "image/png");
    assert_eq!(att.size_bytes, payload.len() as i64);

    let intent_count: i64 = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT COUNT(*) AS count FROM acta.attachment_write_intents",
        ))
        .await
        .expect("query write intents")
        .expect("write intent count row")
        .try_get("", "count")
        .expect("read write intent count");
    assert_eq!(
        intent_count, 0,
        "raw upload must atomically finalize its attachment row and remove the intent"
    );

    let downloaded = client
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Attach List Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    client
        .acta()
        .upload_attachment(&ws.slug, slug, "file.txt", "text/plain", b"hello".to_vec())
        .await
        .expect("upload");

    let page = client
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Del Attach Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let att = client
        .acta()
        .upload_attachment(&ws.slug, slug, "del.txt", "text/plain", b"bye".to_vec())
        .await
        .expect("upload");

    client
        .acta()
        .delete_attachment(&ws.slug, att.id)
        .await
        .expect("delete attachment");

    let deleted_at: Option<chrono::DateTime<chrono::Utc>> = db
        .conn()
        .query_one_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT deleted_at FROM acta.attachments WHERE id = $1",
            [att.id.into()],
        ))
        .await
        .expect("load deleted attachment")
        .expect("deleted attachment row")
        .try_get("", "deleted_at")
        .expect("attachment tombstone");
    assert!(
        deleted_at.is_some(),
        "delete must retain the attachment row"
    );

    let cleanup_intents: i64 = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT COUNT(*) AS count FROM acta.attachment_write_intents",
        ))
        .await
        .expect("count cleanup intents")
        .expect("cleanup intent count row")
        .try_get("", "count")
        .expect("cleanup intent count");
    assert_eq!(
        cleanup_intents, 0,
        "ordinary delete must not schedule blob cleanup"
    );

    let result = client.acta().download_attachment(&ws.slug, att.id).await;
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Move Me"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let moved = client
        .acta()
        .move_document(&ws.slug, slug, MoveDocumentRequest { folder_id: None })
        .await
        .expect("move document");

    assert_eq!(moved.id, doc.id);
    assert_eq!(moved.folder_id, None);

    db.teardown().await;
}

#[tokio::test]
async fn move_to_nonexistent_folder_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-move-bad").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-move-bad".to_string(),
                task_prefix: "PMB".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Move Bad"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let result = client
        .acta()
        .move_document(
            &ws.slug,
            slug,
            MoveDocumentRequest {
                folder_id: Some(uuid::Uuid::now_v7()),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422 || p.status == 404),
        "moving to a nonexistent folder must be rejected, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn move_to_foreign_workspace_folder_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (alice, ws_a, alice_user) =
        support::login_user_with_workspace(&server, &db, "doc-move-fa").await;
    let (_bob, ws_b, bob_user) =
        support::login_user_with_workspace(&server, &db, "doc-move-fb").await;

    let project = alice
        .acta()
        .create_project(
            &ws_a.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-move-fa".to_string(),
                task_prefix: "PMF".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = alice
        .acta()
        .create_document(&ws_a.slug, &project.slug, doc_req("Move Foreign"))
        .await
        .expect("create document");

    // A folder owned by Bob's workspace must not be a valid move target for Alice's doc.
    let foreign_folder = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws_b.id,
                Actor::User(atlas_acta::actor::UserAttributionId(bob_user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: None,
                parent_folder_id: None,
                name: "bob-folder".to_string(),
            },
        )
        .await
        .expect("create foreign folder");

    let _ = alice_user;

    let slug = doc.slug.as_deref().expect("slug");
    let result = alice
        .acta()
        .move_document(
            &ws_a.slug,
            slug,
            MoveDocumentRequest {
                folder_id: Some(foreign_folder.id.0),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422 || p.status == 404),
        "moving into a folder from another workspace must be rejected, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn move_into_folder_adopts_folder_project() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "doc-move-adopt").await;

    let project_a = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj A".to_string(),
                slug: "proj-move-a".to_string(),
                task_prefix: "PMA".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project a");

    let project_b = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj B".to_string(),
                slug: "proj-move-b".to_string(),
                task_prefix: "PMB2".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project b");

    let doc = client
        .acta()
        .create_document(&ws.slug, &project_a.slug, doc_req("Adopt Project"))
        .await
        .expect("create document");
    assert_eq!(doc.project_id, Some(project_a.id));

    // A folder belonging to project B; moving the doc into it must adopt project B.
    let folder_b = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: Some(atlas_acta::ids::ProjectId(project_b.id)),
                parent_folder_id: None,
                name: "folder-b".to_string(),
            },
        )
        .await
        .expect("create folder b");

    let slug = doc.slug.as_deref().expect("slug");
    let moved = client
        .acta()
        .move_document(
            &ws.slug,
            slug,
            MoveDocumentRequest {
                folder_id: Some(folder_b.id.0),
            },
        )
        .await
        .expect("move document into folder b");

    assert_eq!(moved.folder_id, Some(folder_b.id.0));
    assert_eq!(
        moved.project_id,
        Some(project_b.id),
        "document must adopt the destination folder's project"
    );

    db.teardown().await;
}

#[tokio::test]
async fn move_into_other_project_folder_without_access_is_rejected() {
    use atlas_acta::ids::ProjectId;
    use atlas_server::authz::ResourceRole;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "doc-mv-idor-owner").await;

    let project_a = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj A".to_string(),
                slug: "mv-idor-a".to_string(),
                task_prefix: "MIA".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create project a");

    let project_b = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj B".to_string(),
                slug: "mv-idor-b".to_string(),
                task_prefix: "MIB".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create project b");

    let doc = owner
        .acta()
        .create_document(&ws.slug, &project_a.slug, doc_req("Doc In A"))
        .await
        .expect("create document in a");

    let folder_b = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(owner_user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: Some(ProjectId(project_b.id)),
                parent_folder_id: None,
                name: "folder-in-b".to_string(),
            },
        )
        .await
        .expect("create folder in b");

    let bob = member_client_with_optional_project_grant(
        &server,
        &db,
        &ws,
        "doc-mv-idor-bob",
        Some(ProjectId(project_a.id)),
        Some(ResourceRole::Editor),
    )
    .await;

    let slug = doc.slug.as_deref().expect("slug");
    let result = bob
        .acta()
        .move_document(
            &ws.slug,
            slug,
            MoveDocumentRequest {
                folder_id: Some(folder_b.id.0),
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403 || p.status == 404),
        "editor on project A must not move the doc into project B's folder, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn move_within_authorized_project_folder_succeeds() {
    use atlas_acta::ids::ProjectId;
    use atlas_server::authz::ResourceRole;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "doc-mv-ok-owner").await;

    let project_a = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj A".to_string(),
                slug: "mv-ok-a".to_string(),
                task_prefix: "MOA".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create project a");

    let doc = owner
        .acta()
        .create_document(&ws.slug, &project_a.slug, doc_req("Doc In A OK"))
        .await
        .expect("create document in a");

    let folder_a = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(owner_user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: Some(ProjectId(project_a.id)),
                parent_folder_id: None,
                name: "folder-in-a".to_string(),
            },
        )
        .await
        .expect("create folder in a");

    let bob = member_client_with_optional_project_grant(
        &server,
        &db,
        &ws,
        "doc-mv-ok-bob",
        Some(ProjectId(project_a.id)),
        Some(ResourceRole::Editor),
    )
    .await;

    let slug = doc.slug.as_deref().expect("slug");
    let moved = bob
        .acta()
        .move_document(
            &ws.slug,
            slug,
            MoveDocumentRequest {
                folder_id: Some(folder_a.id.0),
            },
        )
        .await
        .expect("move within authorized project must succeed");

    assert_eq!(moved.folder_id, Some(folder_a.id.0));
    assert_eq!(moved.project_id, Some(project_a.id));

    db.teardown().await;
}

#[tokio::test]
async fn document_moves_batch_preserves_order_and_prior_successes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "doc-moves-batch").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Project".to_string(),
                slug: "doc-moves-batch-project".to_string(),
                task_prefix: "DMB".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let destination = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: Some(atlas_acta::ids::ProjectId(project.id)),
                parent_folder_id: None,
                name: "destination".to_string(),
            },
        )
        .await
        .expect("create destination folder");

    let (foreign_client, foreign_workspace, _) =
        support::login_user_with_workspace(&server, &db, "doc-moves-batch-foreign").await;
    let foreign_project = foreign_client
        .acta()
        .create_project(
            &foreign_workspace.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Foreign Project".to_string(),
                slug: "doc-moves-batch-foreign-project".to_string(),
                task_prefix: "DMF".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create foreign project");
    let foreign_document = foreign_client
        .acta()
        .create_document(
            &foreign_workspace.slug,
            &foreign_project.slug,
            doc_req("Foreign"),
        )
        .await
        .expect("create foreign document");

    let first = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("First"))
        .await
        .expect("create first document");
    let second = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Second"))
        .await
        .expect("create second document");

    let first_slug = first.slug.as_deref().expect("first slug");
    let second_slug = second.slug.as_deref().expect("second slug");
    let response = post_document_moves_batch(
        &server,
        &client,
        &ws.slug,
        serde_json::json!({
            "moves": [
                { "source_document": first_slug, "folder_id": destination.id.0 },
                { "source_document": foreign_document.id, "folder_id": null },
                { "source_document": second_slug, "folder_id": null }
            ]
        }),
    )
    .await;

    assert_eq!(response.status(), 200, "mixed batch must return outcomes");
    let body: serde_json::Value = response.json().await.expect("batch response body");
    let outcomes = body.as_array().expect("ordered result array");
    assert_eq!(outcomes.len(), 3, "every input must have one outcome");
    assert_eq!(outcomes[0]["outcome"], "success");
    assert_eq!(outcomes[0]["index"], 0);
    assert_eq!(outcomes[0]["document"]["id"], first.id.to_string());
    assert!(
        outcomes[0]["document"]["content"].is_null(),
        "batch successes must use compact document values"
    );
    assert_eq!(outcomes[1]["outcome"], "problem");
    assert_eq!(outcomes[1]["index"], 1);
    assert_eq!(outcomes[1]["status"], 404);
    assert!(
        outcomes[1]["document"].is_null(),
        "problems must not disclose source or destination data"
    );
    assert_eq!(outcomes[2]["outcome"], "success");
    assert_eq!(outcomes[2]["index"], 2);

    let first_after = client
        .acta()
        .get_document(&ws.slug, first_slug)
        .await
        .expect("read first after batch");
    let second_after = client
        .acta()
        .get_document(&ws.slug, second_slug)
        .await
        .expect("read second after batch");
    assert_eq!(first_after.folder_id, Some(destination.id.0));
    assert_eq!(second_after.folder_id, None);

    db.teardown().await;
}

#[tokio::test]
async fn document_moves_batch_hides_inaccessible_sources_and_destinations() {
    use atlas_acta::entities::workspace_core::NewFolder;
    use atlas_acta::ids::ProjectId;
    use atlas_server::authz::ResourceRole;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "doc-moves-batch-auth").await;

    let source_project = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Source".into(),
                slug: "doc-moves-batch-auth-source".into(),
                task_prefix: "DMS".into(),
                visibility: Some("private".into()),
                visibility_role: None,
            },
        )
        .await
        .expect("create source project");
    let destination_project = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Destination".into(),
                slug: "doc-moves-batch-auth-destination".into(),
                task_prefix: "DMD".into(),
                visibility: Some("private".into()),
                visibility_role: None,
            },
        )
        .await
        .expect("create destination project");
    let source_folder = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(owner_user.id.0)),
            ),
            NewFolder {
                project_id: Some(ProjectId(source_project.id)),
                parent_folder_id: None,
                name: "source folder".into(),
            },
        )
        .await
        .expect("create source folder");
    let hidden_destination = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(owner_user.id.0)),
            ),
            NewFolder {
                project_id: Some(ProjectId(destination_project.id)),
                parent_folder_id: None,
                name: "hidden destination".into(),
            },
        )
        .await
        .expect("create hidden destination");
    let allowed_source = owner
        .acta()
        .create_document(
            &ws.slug,
            &source_project.slug,
            CreateDocumentRequest {
                title: "Allowed source".into(),
                folder_id: Some(source_folder.id.0),
                content: None,
            },
        )
        .await
        .expect("create allowed source");
    let hidden_source = owner
        .acta()
        .create_document(&ws.slug, &source_project.slug, doc_req("Hidden source"))
        .await
        .expect("create hidden source");
    let allowed_slug = allowed_source.slug.as_deref().expect("allowed source slug");

    let viewer = member_client_with_document_grant(
        &server,
        &db,
        &ws,
        "doc-moves-batch-viewer",
        allowed_source.id,
        ResourceRole::Viewer,
    )
    .await;
    let viewer_response = post_document_moves_batch(
        &server,
        &viewer,
        &ws.slug,
        serde_json::json!({
            "moves": [{ "source_document": allowed_slug, "folder_id": null }]
        }),
    )
    .await;
    assert_eq!(viewer_response.status(), reqwest::StatusCode::OK);
    let viewer_outcomes: serde_json::Value =
        viewer_response.json().await.expect("viewer batch response");
    assert_eq!(viewer_outcomes[0]["outcome"], "problem");
    assert_eq!(viewer_outcomes[0]["status"], 404);

    let attacker = member_client_with_document_grant(
        &server,
        &db,
        &ws,
        "doc-moves-batch-attacker",
        allowed_source.id,
        ResourceRole::Editor,
    )
    .await;

    let (foreign_owner, foreign_ws, foreign_user) =
        support::login_user_with_workspace(&server, &db, "doc-moves-batch-auth-foreign").await;
    let foreign_project = foreign_owner
        .acta()
        .create_project(
            &foreign_ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Foreign".into(),
                slug: "doc-moves-batch-auth-foreign-project".into(),
                task_prefix: "DMF".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create foreign project");
    let foreign_folder = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                foreign_ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(foreign_user.id.0)),
            ),
            NewFolder {
                project_id: Some(ProjectId(foreign_project.id)),
                parent_folder_id: None,
                name: "foreign destination".into(),
            },
        )
        .await
        .expect("create foreign destination");

    let hidden_slug = hidden_source.slug.as_deref().expect("hidden source slug");
    let response = post_document_moves_batch(
        &server,
        &attacker,
        &ws.slug,
        serde_json::json!({
            "moves": [
                { "source_document": hidden_slug, "folder_id": null },
                { "source_document": allowed_slug, "folder_id": hidden_destination.id.0 },
                { "source_document": allowed_slug, "folder_id": foreign_folder.id.0 },
                { "source_document": allowed_slug, "folder_id": null }
            ]
        }),
    )
    .await;

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let outcomes: serde_json::Value = response.json().await.expect("batch response");
    let outcomes = outcomes.as_array().expect("ordered outcomes");
    assert_eq!(outcomes.len(), 4);
    for (index, outcome) in outcomes.iter().enumerate() {
        assert_eq!(outcome["outcome"], "problem");
        assert_eq!(outcome["index"], index);
        assert_eq!(outcome["status"], 404);
        assert_eq!(outcome["type"], "urn:atlas:error:not-found");
        assert_eq!(outcome["title"], "Not Found");
        assert_eq!(
            outcome["hint"],
            "Check the identifier — it may not exist or you may not have access."
        );
    }

    let source_after = owner
        .acta()
        .get_document(&ws.slug, allowed_slug)
        .await
        .expect("read source after denied moves");
    assert_eq!(source_after.folder_id, Some(source_folder.id.0));

    db.teardown().await;
}

#[tokio::test]
async fn document_moves_batch_rejects_an_oversized_envelope_before_processing() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "doc-moves-batch-limit").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Project".into(),
                slug: "doc-moves-batch-limit-project".into(),
                task_prefix: "DML".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let folder = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: Some(atlas_acta::ids::ProjectId(project.id)),
                parent_folder_id: None,
                name: "source folder".into(),
            },
        )
        .await
        .expect("create source folder");
    let document = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Oversized source".into(),
                folder_id: Some(folder.id.0),
                content: None,
            },
        )
        .await
        .expect("create source document");
    let body = serde_json::to_vec(&serde_json::json!({
        "moves": [{
            "source_document": document.slug,
            "folder_id": null
        }],
        "padding": "x".repeat(1024 * 1024)
    }))
    .expect("serialize oversized body");

    let response = reqwest::Client::new()
        .post(support::path::api_url(
            server.base_url(),
            "acta",
            &format!("/workspaces/{}/documents/moves/batch", ws.slug),
        ))
        .bearer_auth(client.token().expect("authenticated token"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("oversized document move batch request");

    assert_eq!(
        response.status(),
        413,
        "oversized envelope must be rejected"
    );
    let document_after = client
        .acta()
        .get_document(&ws.slug, document.slug.as_deref().expect("document slug"))
        .await
        .expect("read source after oversized request");
    assert_eq!(document_after.folder_id, Some(folder.id.0));

    db.teardown().await;
}

#[tokio::test]
async fn document_moves_batch_rejects_more_than_one_hundred_items_before_processing() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "doc-moves-batch-count").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Project".into(),
                slug: "doc-moves-batch-count-project".into(),
                task_prefix: "DMC".into(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let folder = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: Some(atlas_acta::ids::ProjectId(project.id)),
                parent_folder_id: None,
                name: "source folder".into(),
            },
        )
        .await
        .expect("create source folder");
    let document = client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Count source".into(),
                folder_id: Some(folder.id.0),
                content: None,
            },
        )
        .await
        .expect("create source document");

    let moves: Vec<serde_json::Value> = (0..101)
        .map(|index| {
            if index == 0 {
                serde_json::json!({
                    "source_document": document.slug,
                    "folder_id": null
                })
            } else {
                serde_json::json!({
                    "source_document": format!("unreachable-{index}"),
                    "folder_id": null
                })
            }
        })
        .collect();
    let response = post_document_moves_batch(
        &server,
        &client,
        &ws.slug,
        serde_json::json!({ "moves": moves }),
    )
    .await;

    assert_eq!(
        response.status(),
        422,
        "more than 100 moves must be rejected"
    );
    let document_after = client
        .acta()
        .get_document(&ws.slug, document.slug.as_deref().expect("document slug"))
        .await
        .expect("read source after oversized item count");
    assert_eq!(document_after.folder_id, Some(folder.id.0));

    db.teardown().await;
}

async fn member_client_with_document_grant(
    server: &support::TestServer,
    db: &support::TestDb,
    ws: &atlas_acta_postgres::repos::identity::Workspace,
    username: &str,
    document_id: uuid::Uuid,
    role: atlas_server::authz::ResourceRole,
) -> atlas_client::AtlasClient {
    use atlas_server::authz::policy::NewPermissionGrant;

    let hash = atlas_server::auth::password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");
    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member");
    support::activate_user_in_db(db, user.id.0).await;

    let ctx = WorkspaceCtx::new(
        ws.id,
        Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
    );
    db.membership_repo()
        .add(&ctx, user.id, MemberRole::Member)
        .await
        .expect("add membership");
    let grant_repo = atlas_custos_postgres::repos::permissions::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: atlas_custos::WorkspaceScope((ws.id).0),
            user_id: Some(user.id),
            api_key_id: None,
            group_id: None,
            resource_ref: atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Workspace,
                ws.id,
            ),
            role: atlas_server::authz::ResourceRole::Viewer,
            created_by_user_id: None,
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert workspace viewer grant");
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: atlas_custos::WorkspaceScope((ws.id).0),
            user_id: Some(user.id),
            api_key_id: None,
            group_id: None,
            resource_ref: atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Document(atlas_acta::ids::DocumentId(
                    document_id,
                )),
                ws.id,
            ),
            role,
            created_by_user_id: None,
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert document grant");

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(atlas_api::dtos::LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("member login");
    client
}

#[tokio::test]
async fn create_into_foreign_project_folder_is_rejected() {
    use atlas_acta::ids::ProjectId;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "doc-cr-idor-owner").await;

    let project_a = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj A".to_string(),
                slug: "cr-idor-a".to_string(),
                task_prefix: "CIA".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project a");

    let project_b = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj B".to_string(),
                slug: "cr-idor-b".to_string(),
                task_prefix: "CIB".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project b");

    let folder_b = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(owner_user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: Some(ProjectId(project_b.id)),
                parent_folder_id: None,
                name: "cr-folder-in-b".to_string(),
            },
        )
        .await
        .expect("create folder in b");

    let req = CreateDocumentRequest {
        title: "Cross Project Plant".to_string(),
        folder_id: Some(folder_b.id.0),
        content: None,
    };

    let result = owner
        .acta()
        .create_document(&ws.slug, &project_a.slug, req)
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422 || p.status == 404),
        "creating a doc into project A with a project B folder must be rejected, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn create_into_same_project_folder_succeeds() {
    use atlas_acta::ids::ProjectId;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "doc-cr-ok-owner").await;

    let project_a = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj A".to_string(),
                slug: "cr-ok-a".to_string(),
                task_prefix: "COA".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project a");

    let folder_a = db
        .folder_repo()
        .create(
            &WorkspaceCtx::new(
                ws.id,
                Actor::User(atlas_acta::actor::UserAttributionId(owner_user.id.0)),
            ),
            atlas_acta::entities::workspace_core::NewFolder {
                project_id: Some(ProjectId(project_a.id)),
                parent_folder_id: None,
                name: "cr-folder-in-a".to_string(),
            },
        )
        .await
        .expect("create folder in a");

    let req = CreateDocumentRequest {
        title: "Same Project Doc".to_string(),
        folder_id: Some(folder_a.id.0),
        content: None,
    };

    let created = owner
        .acta()
        .create_document(&ws.slug, &project_a.slug, req)
        .await
        .expect("create into same-project folder must succeed");

    assert_eq!(created.project_id, Some(project_a.id));
    assert_eq!(created.folder_id, Some(folder_a.id.0));

    db.teardown().await;
}

// ---- Permissions -----------------------------------------------------------

#[tokio::test]
async fn viewer_cannot_create_document() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "doc-perm-owner").await;

    let project = owner
        .acta()
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
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create viewer");

    support::activate_user_in_db(&db, viewer_user.id.0).await;

    let ctx = WorkspaceCtx::new(
        ws.id,
        Actor::User(atlas_acta::actor::UserAttributionId(viewer_user.id.0)),
    );
    db.membership_repo()
        .add(&ctx, viewer_user.id, MemberRole::Member)
        .await
        .expect("add viewer membership");

    use atlas_acta::ids::ProjectId;
    use atlas_server::authz::ResourceRole;
    use atlas_server::authz::policy::NewPermissionGrant;
    let grant_repo = atlas_custos_postgres::repos::permissions::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: atlas_custos::WorkspaceScope((ws.id).0),
            user_id: Some(viewer_user.id),
            api_key_id: None,
            group_id: None,
            resource_ref: atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Project(ProjectId(project.id)),
                ws.id,
            ),
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
        .acta()
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("API Key Doc"))
        .await
        .expect("create document");

    let key_created = owner
        .custos()
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "test-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
            scopes: Some(vec![ApiKeyScope::DocsRead, ApiKeyScope::DocsUpdate]),
        })
        .await
        .expect("create api key");

    use atlas_acta::ids::ProjectId;
    use atlas_core::principal::ApiKeyId;
    use atlas_server::authz::ResourceRole;
    use atlas_server::authz::policy::NewPermissionGrant;
    let grant_repo = atlas_custos_postgres::repos::permissions::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: atlas_custos::WorkspaceScope((ws.id).0),
            user_id: None,
            api_key_id: Some(ApiKeyId(key_created.id)),
            group_id: None,
            resource_ref: atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Project(ProjectId(project.id)),
                ws.id,
            ),
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
        .acta()
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
        .acta()
        .list_document_history(&ws.slug, slug, None, None)
        .await
        .expect("list history");

    let head_rev = history.items.first().expect("at least one revision");
    let actor = head_rev.actor.as_ref().expect("revision must carry actor");
    assert_eq!(actor.r#type, "api_key", "actor type must be api_key");

    db.teardown().await;
}

// ---- Attachment authorization ----------------------------------------------

/// Creates a member user with a password login, optionally granting a role on a
/// project, and returns an authenticated client for that user.
async fn member_client_with_optional_project_grant(
    server: &support::TestServer,
    db: &support::TestDb,
    ws: &atlas_acta_postgres::repos::identity::Workspace,
    username: &str,
    project_id: Option<atlas_acta::ids::ProjectId>,
    role: Option<atlas_server::authz::ResourceRole>,
) -> atlas_client::AtlasClient {
    use atlas_server::authz::policy::NewPermissionGrant;

    let hash = atlas_server::auth::password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member");

    support::activate_user_in_db(db, user.id.0).await;

    let ctx = WorkspaceCtx::new(
        ws.id,
        Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
    );
    db.membership_repo()
        .add(&ctx, user.id, MemberRole::Member)
        .await
        .expect("add membership");

    if let (Some(pid), Some(role)) = (project_id, role) {
        let grant_repo = atlas_custos_postgres::repos::permissions::PgPermissionGrantRepo {
            conn: db.conn().clone(),
        };
        grant_repo
            .upsert(NewPermissionGrant {
                workspace_id: atlas_custos::WorkspaceScope((ws.id).0),
                user_id: Some(user.id),
                api_key_id: None,
                group_id: None,
                resource_ref: atlas_acta::permissions::resource_ref_codec::to_core(
                    &atlas_acta::permissions::ResourceRef::Project(pid),
                    ws.id,
                ),
                role,
                created_by_user_id: None,
                created_by_api_key_id: None,
            })
            .await
            .expect("upsert grant");
    }

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(atlas_api::dtos::LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("member login");

    client
}

async fn private_project_with_attachment(
    server: &support::TestServer,
    db: &support::TestDb,
    owner: &atlas_client::AtlasClient,
    ws: &atlas_acta_postgres::repos::identity::Workspace,
    slug_prefix: &str,
) -> (atlas_api::dtos::ProjectDto, uuid::Uuid) {
    let _ = (server, db);
    let project = owner
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Restricted".to_string(),
                slug: format!("{slug_prefix}-proj"),
                task_prefix: "RST".to_string(),
                visibility: Some("private".to_string()),
                visibility_role: None,
            },
        )
        .await
        .expect("create private project");

    let doc = owner
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Restricted Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let att = owner
        .acta()
        .upload_attachment(
            &ws.slug,
            slug,
            "secret.txt",
            "text/plain",
            b"top secret".to_vec(),
        )
        .await
        .expect("upload attachment");

    (project, att.id)
}

#[tokio::test]
async fn member_without_grant_cannot_download_or_delete_restricted_attachment() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "att-az-owner-1").await;

    let (_project, attachment_id) =
        private_project_with_attachment(&server, &db, &owner, &ws, "att-az-nogrant").await;

    let outsider =
        member_client_with_optional_project_grant(&server, &db, &ws, "att-az-outsider", None, None)
            .await;

    let download = outsider
        .acta()
        .download_attachment(&ws.slug, attachment_id)
        .await;
    assert!(
        matches!(download, Err(ClientError::Api(ref p)) if p.status == 403 || p.status == 404),
        "member without a grant must not download a restricted attachment, got: {download:?}"
    );

    let delete = outsider
        .acta()
        .delete_attachment(&ws.slug, attachment_id)
        .await;
    assert!(
        matches!(delete, Err(ClientError::Api(ref p)) if p.status == 403 || p.status == 404),
        "member without a grant must not delete a restricted attachment, got: {delete:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn viewer_can_download_but_not_delete_attachment() {
    use atlas_server::authz::ResourceRole;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "att-az-owner-2").await;

    let (project, attachment_id) =
        private_project_with_attachment(&server, &db, &owner, &ws, "att-az-viewer").await;

    let viewer = member_client_with_optional_project_grant(
        &server,
        &db,
        &ws,
        "att-az-viewer-user",
        Some(atlas_acta::ids::ProjectId(project.id)),
        Some(ResourceRole::Viewer),
    )
    .await;

    let bytes = viewer
        .acta()
        .download_attachment(&ws.slug, attachment_id)
        .await
        .expect("viewer must be able to download");
    assert_eq!(bytes, b"top secret");

    let delete = viewer
        .acta()
        .delete_attachment(&ws.slug, attachment_id)
        .await;
    assert!(
        matches!(delete, Err(ClientError::Api(ref p)) if p.status == 403 || p.status == 404),
        "viewer must not delete an attachment, got: {delete:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn editor_can_delete_attachment() {
    use atlas_server::authz::ResourceRole;

    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _) = support::login_user_with_workspace(&server, &db, "att-az-owner-3").await;

    let (project, attachment_id) =
        private_project_with_attachment(&server, &db, &owner, &ws, "att-az-editor").await;

    let editor = member_client_with_optional_project_grant(
        &server,
        &db,
        &ws,
        "att-az-editor-user",
        Some(atlas_acta::ids::ProjectId(project.id)),
        Some(ResourceRole::Editor),
    )
    .await;

    editor
        .acta()
        .delete_attachment(&ws.slug, attachment_id)
        .await
        .expect("editor must be able to delete");

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
        .acta()
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
        .acta()
        .create_document(&ws_b.slug, &proj_b.slug, doc_req("Bob's Secret"))
        .await
        .expect("bob creates document");

    let slug = doc_b.slug.as_deref().expect("slug");

    let result = alice.acta().get_document(&ws_b.slug, slug).await;
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
        .acta()
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
        .acta()
        .create_document(&ws_b.slug, &proj_b.slug, doc_req("Bob's Attach Doc"))
        .await
        .expect("bob creates document");

    let slug = doc_b.slug.as_deref().expect("slug");
    let att = bob
        .acta()
        .upload_attachment(
            &ws_b.slug,
            slug,
            "secret.txt",
            "text/plain",
            b"secret".to_vec(),
        )
        .await
        .expect("bob uploads attachment");

    let result = alice.acta().download_attachment(&ws_b.slug, att.id).await;
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
        .acta()
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
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Oversized Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");

    let result = client
        .acta()
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

// ---- 409 conflict body contains required fields ----------------------------

#[tokio::test]
async fn conflict_409_body_carries_conflict_fields() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-cas-body").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-cas-body".to_string(),
                task_prefix: "PCB".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("CAS Body Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let stale_revision_id = doc.head_revision_id;

    client
        .acta()
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

    let token = client.token().expect("session token").to_string();
    let http = reqwest::Client::new();
    let url = support::path::api_url(
        server.base_url(),
        "acta",
        &format!("/workspaces/{}/documents/{}/content", ws.slug, slug),
    );
    let body = serde_json::json!({
        "content": "concurrent update",
        "base_revision_id": stale_revision_id
    });

    let response = http
        .put(&url)
        .bearer_auth(&token)
        .header("x-atlas-csrf", "1")
        .json(&body)
        .send()
        .await
        .expect("send request");

    assert_eq!(response.status().as_u16(), 409, "stale CAS must return 409");
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "application/problem+json",
        "conflict response must be application/problem+json"
    );

    let body: serde_json::Value = response.json().await.expect("parse conflict body");

    assert!(
        body["current_revision_id"].is_string(),
        "conflict body must carry current_revision_id, got: {body}"
    );
    assert!(
        body["current_seq"].is_number(),
        "conflict body must carry current_seq, got: {body}"
    );
    assert!(
        body["base_to_current_patch"].is_string(),
        "conflict body must carry base_to_current_patch, got: {body}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn download_attachment_sets_nosniff_header() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-att-nosniff").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-att-nosniff".to_string(),
                task_prefix: "PAN".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Nosniff Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");
    let att = client
        .acta()
        .upload_attachment(&ws.slug, slug, "test.txt", "text/plain", b"data".to_vec())
        .await
        .expect("upload attachment");

    let token = client.token().expect("session token").to_string();
    let http = reqwest::Client::new();
    let url = support::path::api_url(
        server.base_url(),
        "acta",
        &format!("/workspaces/{}/attachments/{}", ws.slug, att.id),
    );

    let response = http
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .expect("send request");

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok())
            .unwrap_or(""),
        "nosniff",
        "download response must carry x-content-type-options: nosniff"
    );

    db.teardown().await;
}

#[tokio::test]
async fn get_revision_content_nonexistent_seq_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "doc-rev-404").await;

    let project = client
        .acta()
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Proj".to_string(),
                slug: "proj-rev-404".to_string(),
                task_prefix: "PRV".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let doc = client
        .acta()
        .create_document(&ws.slug, &project.slug, doc_req("Rev 404 Doc"))
        .await
        .expect("create document");

    let slug = doc.slug.as_deref().expect("slug");

    let result = client
        .acta()
        .get_revision_content(&ws.slug, slug, 9999)
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "non-existent revision seq must return 404, got: {result:?}"
    );

    let result_zero = client.acta().get_revision_content(&ws.slug, slug, 0).await;

    assert!(
        matches!(result_zero, Err(ClientError::Api(ref p)) if p.status == 404),
        "revision seq 0 must return 404, got: {result_zero:?}"
    );

    db.teardown().await;
}
