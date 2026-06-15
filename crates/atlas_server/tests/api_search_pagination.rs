//! Pagination determinism matrix for `GET /v1/workspaces/{ws}/search`.
//!
//! Guards the invariant that when two hits share an identical primary sort key
//! (score or updated_at), the secondary sort key (id DESC, UUIDv7 time-ordering)
//! fully orders them so that page-throughs produce no duplicates and no gaps.
//!
//! All tests verify:
//! - The full result set is exactly covered with no duplicates and no gaps.
//! - `has_more` and `next_cursor` are correct at every page boundary.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::{
    dtos::search::SearchHitDto,
    pagination::{Page, SearchCursor, SortKey},
};
use atlas_domain::{Actor, WorkspaceCtx, entities::documents::NewDocument};
use atlas_server::persistence::repos::{DocumentRepo, PgDocumentRepo};
use sea_orm::{ConnectionTrait, Statement};
use std::collections::HashSet;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn search_url(base: &str, ws: &str, qs: &str) -> String {
    format!("{base}/v1/workspaces/{ws}/search?{qs}")
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
        .expect("HTTP request")
}

async fn get_page(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    ws: &str,
    qs: &str,
) -> Page<SearchHitDto> {
    let resp = get_search(http, token, base, ws, qs).await;
    assert_eq!(
        resp.status().as_u16(),
        200,
        "expected 200, got {:?}",
        resp.status()
    );
    resp.json().await.expect("decode Page<SearchHitDto>")
}

async fn seed_document(
    db: &support::TestDb,
    ctx: &WorkspaceCtx,
    title: &str,
    content: &str,
) -> Uuid {
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let doc = repo
        .create(
            ctx,
            NewDocument {
                title: title.to_string(),
                slug: None,
                content: content.to_string(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document");
    doc.id.0
}

/// Pages through all results using the provided query string prefix (e.g. `q=foo`).
/// Returns all collected items in page order.
async fn page_through_all(
    http: &reqwest::Client,
    token: &str,
    base: &str,
    ws: &str,
    q_prefix: &str,
    sort: &str,
    page_size: u32,
) -> Vec<SearchHitDto> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut qs = format!("{q_prefix}&sort={sort}&limit={page_size}");
        if let Some(ref c) = cursor {
            qs.push_str(&format!("&cursor={c}"));
        }

        let page = get_page(http, token, base, ws, &qs).await;

        let returned = page.items.len();
        all.extend(page.items);

        if page.has_more {
            let next = page
                .next_cursor
                .expect("has_more=true must have next_cursor");
            cursor = Some(next);
        } else {
            assert!(
                page.next_cursor.is_none(),
                "has_more=false must not have next_cursor"
            );
            break;
        }

        // Guard against infinite loops in test-harness bugs.
        if all.len() > 10_000 {
            panic!("page_through_all collected >10000 items — probable infinite loop");
        }

        if returned == 0 && page.has_more {
            panic!("empty page with has_more=true — broken pagination");
        }
    }

    all
}

// ---------------------------------------------------------------------------
// T15a: Tie in relevance score → id tie-break prevents duplicates / gaps
//
// Two documents with IDENTICAL content produce identical ts_rank_cd scores.
// The secondary sort (id DESC) must fully order them so that a page-through
// with page_size=1 yields each document exactly once.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn relevance_tie_no_duplicate_no_gap() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "pag-rel-tie").await;
    let token = client.token().expect("token");
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    // Same title AND content → identical ts_rank_cd score for the same query.
    let unique = "tiescoreuniq17";
    let mut expected_ids: HashSet<Uuid> = HashSet::new();
    for i in 0..4 {
        let id = seed_document(&db, &ctx, &format!("Tie Score {i} {unique}"), unique).await;
        expected_ids.insert(id);
    }

    let http = reqwest::Client::new();
    let all = page_through_all(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}"),
        "relevance",
        1,
    )
    .await;

    let collected_ids: HashSet<Uuid> = all.iter().map(|h| h.id).collect();

    assert_eq!(
        all.len(),
        expected_ids.len(),
        "page-through must return exactly {n} items with no duplicates; got {got}",
        n = expected_ids.len(),
        got = all.len()
    );

    assert_eq!(
        collected_ids, expected_ids,
        "page-through must cover all seeded docs with no gaps"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T15b: Tie in updated_at → id tie-break prevents duplicates / gaps
//
// Two documents inserted in rapid succession can share the same updated_at
// microsecond (SeaORM's `Utc::now()` precision). The secondary sort (id DESC)
// must fully order them.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn updated_tie_no_duplicate_no_gap() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "pag-upd-tie").await;
    let token = client.token().expect("token");
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    let unique = "tieupduniqs18";
    let mut expected_ids: HashSet<Uuid> = HashSet::new();

    // Seed 4 docs in quick succession; some may share the same updated_at microsecond.
    for i in 0..4 {
        let id = seed_document(&db, &ctx, &format!("Updated Tie {i} {unique}"), unique).await;
        expected_ids.insert(id);
    }

    let http = reqwest::Client::new();
    let all = page_through_all(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}"),
        "updated",
        1,
    )
    .await;

    let collected_ids: HashSet<Uuid> = all.iter().map(|h| h.id).collect();

    assert_eq!(
        all.len(),
        expected_ids.len(),
        "page-through with updated sort must return exactly {n} items; got {got}",
        n = expected_ids.len(),
        got = all.len()
    );

    assert_eq!(
        collected_ids, expected_ids,
        "page-through with updated sort must cover all seeded docs with no gaps"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T15b-det: updated_at DETERMINISTIC tie — id DESC tie-break, no duplicate/gap
//
// Seeds 3 docs then forces all three to the SAME updated_at via a direct SQL
// UPDATE, guaranteeing a real tie regardless of wall-clock resolution. Pages
// across the tie boundary with limit=1 and asserts the exact ordered id sequence
// (id DESC tiebreak) with no duplicate and no gap.
//
// This test would fail if the id tiebreak were absent or reversed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn updated_tie_deterministic_no_duplicate_no_gap() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "pag-upd-det").await;
    let token = client.token().expect("token");
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    let unique = "tieupddetu19x";

    // Seed 3 documents that all share the same query term.
    let repo = PgDocumentRepo::new(db.conn().clone(), 50);
    let mut seeded_ids: Vec<Uuid> = Vec::new();
    for i in 0..3 {
        let doc = repo
            .create(
                &ctx,
                NewDocument {
                    title: format!("Det Tie {i} {unique}"),
                    slug: None,
                    content: unique.to_string(),
                    folder_id: None,
                    project_id: None,
                    frontmatter: None,
                },
            )
            .await
            .expect("seed document");
        seeded_ids.push(doc.id.0);
    }

    // Force all three documents to the same updated_at timestamp. Using a fixed
    // value in the past ensures no other insert can accidentally share it, and
    // the identical value guarantees a real id-tiebreak is needed.
    let placeholders = seeded_ids
        .iter()
        .enumerate()
        .map(|(i, _)| format!("${}", i + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let update_sql = format!(
        "UPDATE documents SET updated_at = '2020-01-01 00:00:00+00' WHERE id IN ({placeholders})"
    );
    let values: Vec<sea_orm::Value> = seeded_ids
        .iter()
        .map(|id| sea_orm::Value::Uuid(Some(*id)))
        .collect();
    db.conn()
        .execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            &update_sql,
            values,
        ))
        .await
        .expect("force identical updated_at");

    // Page through with limit=1. Every page crosses a tie boundary.
    // The expected order is id DESC (UUIDv7 — descending by creation time).
    let http = reqwest::Client::new();
    let all = page_through_all(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}"),
        "updated",
        1,
    )
    .await;

    let collected_ids: Vec<Uuid> = all.iter().map(|h| h.id).collect();
    let expected_set: HashSet<Uuid> = seeded_ids.iter().copied().collect();
    let collected_set: HashSet<Uuid> = collected_ids.iter().copied().collect();

    assert_eq!(
        all.len(),
        seeded_ids.len(),
        "page-through must return exactly {n} items with no duplicates; got {got}",
        n = seeded_ids.len(),
        got = all.len()
    );

    assert_eq!(
        collected_set, expected_set,
        "page-through must cover all seeded docs with no gaps"
    );

    // The id tiebreak must be DESC: higher UUIDv7 (later insertion) comes first.
    let mut expected_ordered = seeded_ids.clone();
    expected_ordered.sort_by(|a, b| b.cmp(a));

    assert_eq!(
        collected_ids, expected_ordered,
        "tied updated_at rows must be ordered by id DESC; \
         got {collected_ids:?}, expected {expected_ordered:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T15c: Multi-page relevance sort — ordered sequence, no duplicate, no gap
//
// Seeds N docs with the same query term (different relevance via title weight)
// and pages through them 2 at a time. Verifies the collected sequence
// contains each id exactly once.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multipage_relevance_sort_no_duplicate_no_gap() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "pag-rel-mp").await;
    let token = client.token().expect("token");
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    let unique = "multipgreluniq19";
    let mut expected_ids: HashSet<Uuid> = HashSet::new();

    for i in 0..7 {
        let id = seed_document(
            &db,
            &ctx,
            &format!("MultiPage Relevance {i} {unique}"),
            &format!("{unique} extra content for item {i}"),
        )
        .await;
        expected_ids.insert(id);
    }

    let http = reqwest::Client::new();
    let all = page_through_all(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}"),
        "relevance",
        2,
    )
    .await;

    let collected_ids: HashSet<Uuid> = all.iter().map(|h| h.id).collect();

    assert_eq!(
        all.len(),
        expected_ids.len(),
        "relevance multi-page must cover all {n} docs exactly once; got {got}",
        n = expected_ids.len(),
        got = all.len()
    );

    assert_eq!(
        collected_ids, expected_ids,
        "relevance multi-page must not miss any seeded doc"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T15d: Multi-page updated sort — no duplicate, no gap
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multipage_updated_sort_no_duplicate_no_gap() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "pag-upd-mp").await;
    let token = client.token().expect("token");
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(user.id));

    let unique = "multipgupduniq20";
    let mut expected_ids: HashSet<Uuid> = HashSet::new();

    for i in 0..7 {
        let id = seed_document(
            &db,
            &ctx,
            &format!("MultiPage Updated {i} {unique}"),
            unique,
        )
        .await;
        expected_ids.insert(id);
    }

    let http = reqwest::Client::new();
    let all = page_through_all(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        &format!("q={unique}"),
        "updated",
        2,
    )
    .await;

    let collected_ids: HashSet<Uuid> = all.iter().map(|h| h.id).collect();

    assert_eq!(
        all.len(),
        expected_ids.len(),
        "updated multi-page must cover all {n} docs exactly once; got {got}",
        n = expected_ids.len(),
        got = all.len()
    );

    assert_eq!(
        collected_ids, expected_ids,
        "updated multi-page must not miss any seeded doc"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T15e: Malformed cursor → 422 with problem type (cursor validation)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn malformed_cursor_pagination_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "pag-badcur").await;
    let token = client.token().expect("token");
    let http = reqwest::Client::new();

    let resp = get_search(
        &http,
        token,
        server.base_url(),
        &ws.slug,
        "q=hello&cursor=not-a-valid-cursor-at-all",
    )
    .await;

    assert_eq!(
        resp.status().as_u16(),
        422,
        "malformed cursor must return 422"
    );
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(
        body["type"], "urn:atlas:error:invalid-input",
        "problem type must be invalid-input"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T15f: Cursor/sort tag mismatch → 422 (relevance cursor with updated sort)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cursor_sort_mismatch_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "pag-sortmm").await;
    let token = client.token().expect("token");
    let http = reqwest::Client::new();

    // Build a relevance-sort cursor.
    let cursor = SearchCursor {
        key: SortKey::Relevance(0.5),
        id: Uuid::now_v7(),
    };
    let encoded = cursor.encode();

    // Send it with sort=updated — the server must reject this.
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
        "relevance cursor with updated sort must return 422"
    );
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(body["type"], "urn:atlas:error:invalid-input");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T15g: Limit clamping — out-of-range values succeed (silently clamped)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn limit_out_of_range_is_silently_clamped() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, _) = support::login_user_with_workspace(&server, &db, "pag-limclamp").await;
    let token = client.token().expect("token");
    let http = reqwest::Client::new();

    for limit_str in ["0", "201", "99999"] {
        let resp = get_search(
            &http,
            token,
            server.base_url(),
            &ws.slug,
            &format!("q=anythingwilldo&limit={limit_str}"),
        )
        .await;

        assert_eq!(
            resp.status().as_u16(),
            200,
            "limit={limit_str} must return 200 (clamped, not an error)"
        );
    }

    db.teardown().await;
}
