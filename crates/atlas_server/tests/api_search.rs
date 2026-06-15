#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::search::SearchHitDto;
use atlas_api::pagination::{Page, SearchCursor, SortKey};
use atlas_domain::{Actor, WorkspaceCtx, entities::documents::NewDocument};
use atlas_server::persistence::repos::{DocumentRepo, PgDocumentRepo};
use serde_json::Value;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helper: make a raw HTTP GET to the search endpoint with Bearer auth.
// ---------------------------------------------------------------------------

fn search_url(base: &str, ws: &str, qs: &str) -> String {
    if qs.is_empty() {
        format!("{base}/v1/workspaces/{ws}/search")
    } else {
        format!("{base}/v1/workspaces/{ws}/search?{qs}")
    }
}

async fn get_search(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    ws: &str,
    qs: &str,
) -> reqwest::Response {
    http.get(search_url(base, ws, qs))
        .bearer_auth(token)
        .send()
        .await
        .expect("HTTP request must succeed")
}

// ---------------------------------------------------------------------------
// T8 tests: route-level status codes and response shapes
// ---------------------------------------------------------------------------

/// Absent `q` param -> 422 with `urn:atlas:error:invalid-input`.
#[tokio::test]
async fn absent_q_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "search-abq").await;
    let token = client.token().expect("must be logged in");
    let http = reqwest::Client::new();

    let resp = get_search(&http, token, server.base_url(), &ws.slug, "").await;

    assert_eq!(resp.status().as_u16(), 422, "absent q must return 422");
    let body: Value = resp.json().await.expect("json body");
    assert_eq!(
        body["type"], "urn:atlas:error:invalid-input",
        "must be invalid-input problem type"
    );

    db.teardown().await;
}

/// Present-but-empty `q` -> 200 with empty page.
#[tokio::test]
async fn empty_q_returns_200_empty_page() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "search-emq").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let resp = get_search(&http, token, server.base_url(), &ws.slug, "q=").await;

    assert_eq!(resp.status().as_u16(), 200, "empty q must return 200");
    let page: Page<SearchHitDto> = resp.json().await.expect("json");
    assert!(page.items.is_empty());
    assert!(!page.has_more);
    assert!(page.next_cursor.is_none());

    db.teardown().await;
}

/// Whitespace-only `q` -> 200 empty page.
#[tokio::test]
async fn whitespace_q_returns_200_empty_page() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "search-wsq").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    // %20%20%20 = "   " (three spaces)
    let resp = get_search(&http, token, server.base_url(), &ws.slug, "q=%20%20%20").await;

    assert_eq!(resp.status().as_u16(), 200, "whitespace q must return 200");
    let page: Page<SearchHitDto> = resp.json().await.expect("json");
    assert!(page.items.is_empty());

    db.teardown().await;
}

/// Malformed cursor -> 422.
#[tokio::test]
async fn malformed_cursor_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "search-badc").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        "q=hello&cursor=thisisnotavalidcursor",
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        422,
        "malformed cursor must return 422"
    );
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["type"], "urn:atlas:error:invalid-input");

    db.teardown().await;
}

/// Relevance cursor sent with `sort=updated` -> 422 (sort tag mismatch).
#[tokio::test]
async fn cursor_sort_tag_mismatch_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "search-smm").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let cursor = SearchCursor {
        key: SortKey::Relevance(0.5),
        id: Uuid::now_v7(),
    };
    let encoded = cursor.encode();

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q=hello&sort=updated&cursor={encoded}"),
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        422,
        "cursor/sort tag mismatch must return 422"
    );
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["type"], "urn:atlas:error:invalid-input");

    db.teardown().await;
}

/// Updated cursor sent with default relevance sort -> 422 (reverse mismatch).
#[tokio::test]
async fn cursor_updated_with_relevance_sort_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "search-smm2").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let cursor = SearchCursor {
        key: SortKey::Updated(1_718_000_000_000_000_i64),
        id: Uuid::now_v7(),
    };
    let encoded = cursor.encode();

    // No sort param -> defaults to relevance; cursor has Updated tag -> mismatch
    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q=hello&cursor={encoded}"),
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        422,
        "updated cursor with implicit-relevance sort must return 422"
    );

    db.teardown().await;
}

/// Contradictory filter (type=note + status: task-only token) -> 200 empty page.
#[tokio::test]
async fn contradictory_filter_returns_200_empty_page() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "search-contr").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    // q contains status:open (task-only filter); type=note restricts to docs -> TaskFilterOnNotes
    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        "q=status%3Aopen&type=note",
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        200,
        "contradictory filter must return 200"
    );
    let page: Page<SearchHitDto> = resp.json().await.expect("json");
    assert!(
        page.items.is_empty(),
        "contradictory filter must yield empty items"
    );

    db.teardown().await;
}

/// `limit=5000` and `limit=0` are clamped silently; no error returned.
#[tokio::test]
async fn limit_clamping_does_not_error() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "search-lim").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    for limit_str in ["5000", "0"] {
        let resp = get_search(
            &http,
            token,
            server.base_url(),
            &ws.slug,
            &format!("q=hello&limit={limit_str}"),
        )
        .await;

        assert_eq!(
            resp.status().as_u16(),
            200,
            "limit={limit_str} must return 200 after clamping"
        );
        let _page: Page<SearchHitDto> = resp.json().await.expect("json");
    }

    db.teardown().await;
}

/// Happy path: a seeded document matching the query appears in the response.
#[tokio::test]
async fn happy_path_returns_matching_document() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "search-happy").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 50);

    let unique_word = "xyzzy9uniqueterm";
    doc_repo
        .create(
            &ctx,
            NewDocument {
                title: format!("Document about {unique_word}"),
                slug: Some(format!("doc-{unique_word}")),
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique_word}"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200, "happy path must return 200");
    let page: Page<SearchHitDto> = resp.json().await.expect("json");
    assert!(!page.items.is_empty(), "must find the seeded document");

    let hit = &page.items[0];
    assert!(
        hit.title.to_lowercase().contains(unique_word),
        "hit title must contain the search term; got: {}",
        hit.title
    );
    assert!(
        hit.readable_id.is_none(),
        "document hit must not have readable_id"
    );

    db.teardown().await;
}

/// Two pages with `sort=updated` and `limit=1` do not repeat items and the cursor round-trips.
#[tokio::test]
async fn updated_sort_cursor_paginates_without_duplicates() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "search-currt").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 50);

    let unique = "zzzpaginationtestdoc";
    for i in 0..2_u32 {
        doc_repo
            .create(
                &ctx,
                NewDocument {
                    title: format!("{unique} {i}"),
                    slug: Some(format!("{unique}-{i}-{}", Uuid::now_v7().as_simple())),
                    content: String::new(),
                    folder_id: None,
                    project_id: None,
                    frontmatter: None,
                },
            )
            .await
            .expect("create doc");
    }

    let resp1 = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&sort=updated&limit=1"),
    )
    .await;

    assert_eq!(resp1.status().as_u16(), 200);
    let page1: Page<SearchHitDto> = resp1.json().await.expect("json p1");
    assert_eq!(page1.items.len(), 1, "limit=1 must return 1 item");
    assert!(page1.has_more, "must have more with 2 docs and limit=1");

    let cursor = page1.next_cursor.expect("must have next_cursor");

    let resp2 = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&sort=updated&limit=1&cursor={cursor}"),
    )
    .await;

    assert_eq!(resp2.status().as_u16(), 200, "valid cursor must return 200");
    let page2: Page<SearchHitDto> = resp2.json().await.expect("json p2");
    assert_eq!(page2.items.len(), 1, "second page must have 1 item");

    let id1 = page1.items[0].id;
    let id2 = page2.items[0].id;
    assert_ne!(id1, id2, "pages must not repeat items");

    db.teardown().await;
}

/// Unauthenticated request -> 401.
#[tokio::test]
async fn unauthenticated_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (_, ws, _) = support::login_user_with_workspace(&server, &db, "search-unauth").await;

    let anon = reqwest::Client::new();
    let resp = anon
        .get(search_url(server.base_url(), &ws.slug, "q=hello"))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status().as_u16(), 401);

    db.teardown().await;
}

/// Unknown workspace -> 404 (cross-tenant concealment).
#[tokio::test]
async fn unknown_workspace_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, _, _) = support::login_user_with_workspace(&server, &db, "search-unkws").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        "nonexistent-workspace-xyz",
        "q=hello",
    )
    .await;

    assert_eq!(resp.status().as_u16(), 404);

    db.teardown().await;
}

/// REQ-2: under `sort=relevance`, a document whose title contains the search
/// term outranks a document whose title does NOT contain it (body-only match).
///
/// The `search_vector` column is weighted A (title) and B (body). `ts_rank_cd`
/// applied to a vector weighted A>B must produce a strictly higher score for
/// the title hit. This guards the A/B weight assignment in the E02 GEN column.
#[tokio::test]
async fn title_hit_outranks_body_hit_under_relevance_sort() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "search-rank").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 50);

    // A term that appears in no other doc in this workspace.
    let unique = "xtitlerank7qzx";

    // Doc A: term only in title, empty body.
    let doc_a = doc_repo
        .create(
            &ctx,
            NewDocument {
                title: unique.to_string(),
                slug: Some(format!("doc-a-{unique}")),
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create doc A");

    // Doc B: term only in body, neutral title that does NOT contain the term.
    doc_repo
        .create(
            &ctx,
            NewDocument {
                title: "Neutral document title".to_string(),
                slug: Some(format!("doc-b-{unique}")),
                content: format!("This body contains {unique} only here."),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create doc B");

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&sort=relevance"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200);
    let page: Page<SearchHitDto> = resp.json().await.expect("json");

    assert_eq!(
        page.items.len(),
        2,
        "both docs must match; got: {:?}",
        page.items.len()
    );

    let first = &page.items[0];
    assert_eq!(
        first.id,
        doc_a.id.0,
        "doc A (title match) must rank first; got: {:?}",
        page.items.iter().map(|h| h.id).collect::<Vec<_>>()
    );
    assert!(
        page.items[0].score > page.items[1].score,
        "title-match score ({}) must exceed body-match score ({})",
        page.items[0].score,
        page.items[1].score
    );

    db.teardown().await;
}

/// REQ-7: a body-match hit's `snippet` contains `<mark>` highlighting.
///
/// The outer page query runs `ts_headline` only for the returned rows, which
/// wraps matched lexemes in `<mark>…</mark>`. An absent or un-highlighted
/// snippet indicates the headline SQL is not running or is not targeting the
/// right column.
#[tokio::test]
async fn body_match_snippet_contains_mark_highlight() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "search-hl").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 50);

    let unique = "xmarkhlterm9mzq";

    doc_repo
        .create(
            &ctx,
            NewDocument {
                title: "Plain title without the term".to_string(),
                slug: Some(format!("doc-hl-{unique}")),
                content: format!("The body contains {unique} surrounded by other words."),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create doc");

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200);
    let page: Page<SearchHitDto> = resp.json().await.expect("json");

    assert!(!page.items.is_empty(), "doc must match the query");

    let hit = &page.items[0];
    let snippet = hit
        .snippet
        .as_deref()
        .expect("snippet must be present for a body match");
    assert!(
        snippet.contains("<mark>") && snippet.contains("</mark>"),
        "snippet must carry <mark> highlighting; got: {snippet:?}"
    );

    db.teardown().await;
}
