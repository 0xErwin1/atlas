#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::search::{SearchHitDto, SearchKindDto};
use atlas_api::pagination::{Page, SearchCursor, SortKey};
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{
        boards_tasks::{NewBoard, NewTask, PositionBetween},
        documents::NewDocument,
        workspace_core::NewProject,
    },
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::persistence::repos::{
    BoardRepo, DocumentRepo, PgBoardRepo, PgDocumentRepo, PgProjectRepo, PgTaskRepo, ProjectRepo,
    TaskRepo,
};
use serde_json::Value;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helper: make a raw HTTP GET to the search endpoint with Bearer auth.
// ---------------------------------------------------------------------------

fn search_url(base: &str, ws: &str, qs: &str) -> String {
    if qs.is_empty() {
        format!("{base}/api/workspaces/{ws}/search")
    } else {
        format!("{base}/api/workspaces/{ws}/search?{qs}")
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

// ---------------------------------------------------------------------------
// Helpers shared by T10/T11 integration tests
// ---------------------------------------------------------------------------

async fn seed_doc_and_task(db: &support::TestDb, ctx: &WorkspaceCtx, unique: &str) -> (Uuid, Uuid) {
    let doc_repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let doc = doc_repo
        .create(
            ctx,
            NewDocument {
                title: format!("Document {unique}"),
                slug: Some(format!("doc-{unique}")),
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");

    let project_repo = PgProjectRepo {
        conn: db.conn().clone(),
    };
    let board_repo = PgBoardRepo::new(db.conn().clone());
    let task_repo = PgTaskRepo::new(db.conn().clone());

    let project = project_repo
        .create(
            ctx,
            NewProject {
                name: format!("Project {unique}"),
                slug: format!("proj-{unique}"),
                task_prefix: format!("T{}", &unique[..4].to_uppercase()),
                visibility: Visibility::Workspace(VisibilityRole::Editor),
            },
        )
        .await
        .expect("seed project");

    let board = board_repo
        .create_board(
            ctx,
            NewBoard {
                folder_id: None,
                project_id: project.id,
                name: "Board".to_string(),
            },
        )
        .await
        .expect("seed board");

    let col = board_repo
        .add_column(
            ctx,
            board.id,
            "Backlog".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("seed column");

    let task = task_repo
        .create(
            ctx,
            NewTask {
                column_id: col.id,
                board_id: board.id,
                project_id: project.id,
                title: format!("Task {unique}"),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("seed task");

    (doc.id.0, task.id.0)
}

// ---------------------------------------------------------------------------
// T10 — REQ-MT2/MT3/MT7/MT8 integration tests
// ---------------------------------------------------------------------------

/// `type=note,task` returns both the seeded document and the seeded task (REQ-MT2).
#[tokio::test]
async fn type_note_task_returns_both_kinds() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "search-mt2-both").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let unique = "xmt2bothkinds9z";
    let (doc_id, task_id) = seed_doc_and_task(&db, &ctx, unique).await;

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&type=note,task"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200);
    let page: Page<SearchHitDto> = resp.json().await.expect("json");

    let ids: Vec<Uuid> = page.items.iter().map(|h| h.id).collect();
    assert!(ids.contains(&doc_id), "document must appear; got: {ids:?}");
    assert!(ids.contains(&task_id), "task must appear; got: {ids:?}");

    let has_doc_kind = page.items.iter().any(|h| h.kind == SearchKindDto::Document);
    let has_task_kind = page.items.iter().any(|h| h.kind == SearchKindDto::Task);
    assert!(has_doc_kind, "must have a document-kind hit");
    assert!(has_task_kind, "must have a task-kind hit");

    db.teardown().await;
}

/// `type=note,task` today-only equivalence to absent `type` (REQ-MT2 caveat).
///
/// Today the searchable universe is exactly {note, task}, so an explicit
/// `type=note,task` returns the same set as an absent `type`. This assertion
/// is labeled "today only" — once a third kind exists `{note,task}` will
/// correctly EXCLUDE it while `all` includes it.
#[tokio::test]
async fn type_note_task_today_same_as_absent_type() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "search-mt2-equiv").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let unique = "xmt2equivterm8q";
    seed_doc_and_task(&db, &ctx, unique).await;

    let resp_explicit = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&type=note,task"),
    )
    .await;
    let resp_absent = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}"),
    )
    .await;

    let page_explicit: Page<SearchHitDto> = resp_explicit.json().await.expect("json explicit");
    let page_absent: Page<SearchHitDto> = resp_absent.json().await.expect("json absent");

    let mut ids_explicit: Vec<Uuid> = page_explicit.items.iter().map(|h| h.id).collect();
    let mut ids_absent: Vec<Uuid> = page_absent.items.iter().map(|h| h.id).collect();
    ids_explicit.sort();
    ids_absent.sort();

    // today only, not an invariant: {note,task}==all while universe is {note,task}
    assert_eq!(
        ids_explicit, ids_absent,
        "today only: type=note,task must match absent type (universe={{note,task}})"
    );

    db.teardown().await;
}

/// `type=note` returns only the document (backward compat, REQ-MT1).
#[tokio::test]
async fn type_note_returns_only_documents() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "search-mt1-note").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let unique = "xmt1noteonlyterm";
    let (doc_id, _task_id) = seed_doc_and_task(&db, &ctx, unique).await;

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&type=note"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200);
    let page: Page<SearchHitDto> = resp.json().await.expect("json");

    let ids: Vec<Uuid> = page.items.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&doc_id),
        "document must appear with type=note; got: {ids:?}"
    );
    assert!(
        page.items.iter().all(|h| h.kind == SearchKindDto::Document),
        "type=note must return only documents; got kinds: {:?}",
        page.items.iter().map(|h| &h.kind).collect::<Vec<_>>()
    );

    db.teardown().await;
}

/// `type=task` returns only the task (backward compat, REQ-MT1).
#[tokio::test]
async fn type_task_returns_only_tasks() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "search-mt1-task").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let unique = "xmt1taskonlyterm";
    let (_doc_id, task_id) = seed_doc_and_task(&db, &ctx, unique).await;

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&type=task"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200);
    let page: Page<SearchHitDto> = resp.json().await.expect("json");

    let ids: Vec<Uuid> = page.items.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&task_id),
        "task must appear with type=task; got: {ids:?}"
    );
    assert!(
        page.items.iter().all(|h| h.kind == SearchKindDto::Task),
        "type=task must return only tasks; got kinds: {:?}",
        page.items.iter().map(|h| &h.kind).collect::<Vec<_>>()
    );

    db.teardown().await;
}

/// `type=note,task` + status filter in q does NOT short-circuit (REQ-MT7 NO row).
///
/// With both note and task selected, the task arm covers the status filter,
/// so no TaskFilterOnNotes warning fires and the route does not return empty.
#[tokio::test]
async fn type_note_task_with_status_filter_does_not_short_circuit() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "search-mt7-notask").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let unique = "xmt7nowarningterm";
    let (_doc_id, task_id) = seed_doc_and_task(&db, &ctx, unique).await;

    // q contains status:backlog (task-only filter); type=note,task includes task arm
    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}%20status:backlog&type=note,task"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200);
    let page: Page<SearchHitDto> = resp.json().await.expect("json");
    // The route must NOT short-circuit to empty
    let ids: Vec<Uuid> = page.items.iter().map(|h| h.id).collect();
    assert!(
        ids.contains(&task_id),
        "type=note,task with status filter must find the task (no short-circuit); got: {ids:?}"
    );

    db.teardown().await;
}

/// `type=note` + status filter in q → empty page (warning short-circuit, REQ-MT7 YES row).
#[tokio::test]
async fn type_note_with_status_filter_returns_empty() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "search-mt7-warn").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let unique = "xmt7warnterm9p";
    seed_doc_and_task(&db, &ctx, unique).await;

    // q contains status:open (task-only filter); type=note is notes-only -> TaskFilterOnNotes
    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}%20status:open&type=note"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200);
    let page: Page<SearchHitDto> = resp.json().await.expect("json");
    assert!(
        page.items.is_empty(),
        "type=note + status filter must return empty page (TaskFilterOnNotes); got: {:?}",
        page.items.len()
    );

    db.teardown().await;
}

/// Permission filtering preserved under `type=note,task` (REQ-MT8).
///
/// Seeding two users' data in the same workspace; a plain member who can see
/// their own document via workspace visibility must not see a private-project
/// resource. The per-arm predicates must still apply independently.
///
/// This test verifies the owner can see both kinds and the task carries readable_id.
#[tokio::test]
async fn type_note_task_owner_sees_both_kinds_with_readable_id() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "search-mt8-perm").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let unique = "xmt8permterm7r";
    let (doc_id, task_id) = seed_doc_and_task(&db, &ctx, unique).await;

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&type=note,task"),
    )
    .await;

    assert_eq!(resp.status().as_u16(), 200);
    let page: Page<SearchHitDto> = resp.json().await.expect("json");
    let ids: Vec<Uuid> = page.items.iter().map(|h| h.id).collect();

    assert!(
        ids.contains(&doc_id),
        "owner must see document with type=note,task; got: {ids:?}"
    );
    assert!(
        ids.contains(&task_id),
        "owner must see task with type=note,task; got: {ids:?}"
    );

    let task_hit = page
        .items
        .iter()
        .find(|h| h.id == task_id)
        .expect("task hit");
    assert!(
        task_hit.readable_id.is_some(),
        "task hit must carry readable_id; got: {:?}",
        task_hit.readable_id
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T11 — REQ-MT4/D1: unknown type token returns all results (not empty)
// ---------------------------------------------------------------------------

/// `type=comment` (unknown token) collapses to all — returns same as absent type (REQ-MT4/D1).
#[tokio::test]
async fn type_unknown_token_collapses_to_all() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "search-d1-unknown").await;
    let token = client.token().expect("logged in");
    let http = reqwest::Client::new();

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let unique = "xd1unknownterm5s";
    let (doc_id, task_id) = seed_doc_and_task(&db, &ctx, unique).await;

    let resp_unknown = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}&type=comment"),
    )
    .await;

    let resp_absent = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}"),
    )
    .await;

    assert_eq!(resp_unknown.status().as_u16(), 200);
    let page_unknown: Page<SearchHitDto> = resp_unknown.json().await.expect("json unknown");
    let page_absent: Page<SearchHitDto> = resp_absent.json().await.expect("json absent");

    let ids_unknown: Vec<Uuid> = page_unknown.items.iter().map(|h| h.id).collect();
    assert!(
        ids_unknown.contains(&doc_id),
        "type=comment must not filter documents (D1 collapse to all); got: {ids_unknown:?}"
    );
    assert!(
        ids_unknown.contains(&task_id),
        "type=comment must not filter tasks (D1 collapse to all); got: {ids_unknown:?}"
    );

    let mut ids_u = ids_unknown.clone();
    let mut ids_a: Vec<Uuid> = page_absent.items.iter().map(|h| h.id).collect();
    ids_u.sort();
    ids_a.sort();
    assert_eq!(
        ids_u, ids_a,
        "type=comment must return same result set as absent type"
    );

    db.teardown().await;
}
