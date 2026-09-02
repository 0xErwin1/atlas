#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! `v2-e3-s3` PR4 (T4.1–T4.6, T4.21/T4.22, plus the two orchestrator-added
//! gates): the `Idempotency-Key` middleware, proven two ways.
//!
//! `mock_handler` tests build a tiny axum router around
//! `idempotency_middleware_release`/`idempotency_middleware_store_briefly`
//! and a stub handler — exactly the shape T4.1 describes ("given a mock
//! handler and the store from PR3") — so the
//! replay/mismatch/in-flight/`Superseded`/`set-cookie`/5xx-policy branches
//! are each isolated from any particular business route's request/response
//! shape.
//!
//! `live_server` tests drive the real `atlas_server::app()` router against
//! `create_tag` (one declared-`idempotent: true` route, chosen for its
//! minimal request body), proving the middleware is reachable end to end
//! through the production layer stack, including T4.21/T4.22's
//! layer-ordering proof: a request that fails `require_authn` never reaches
//! this middleware, so it leaves no row in the store.
//!
//! Every test here is container-backed (Postgres via `TestDb`) and therefore
//! not runnable in this sandbox (INV-CONTAINER-UNVERIFIABLE, rootless podman
//! blocks it); `cargo check --workspace --all-targets` is the local proof,
//! CI shards are the actual gate.

mod support;

use atlas_server::middleware::idempotency::{
    idempotency_middleware_release, idempotency_middleware_store_briefly,
};
use atlas_server::persistence::repos::{
    CompleteOutcome, IN_FLIGHT_TTL, IdempotencyScope, InsertOutcome, PgIdempotencyRepo,
};
use atlas_server::state::AppState;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router};
use chrono::{Duration, Utc};
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use support::TestDb;
use tower::ServiceExt;

const RETENTION: Duration = Duration::hours(24);

/// A stand-in for `require_authn`'s `Principal` insertion, mirroring exactly
/// what production wiring guarantees the idempotency layer sees: the layer
/// is mounted innermost, so a `Principal` extension is always present by the
/// time it runs (or absent, for the routes D8 classifies `false` for
/// exactly that reason — see `routes::custos::public`).
#[derive(Clone, Copy)]
struct StubPrincipal(uuid::Uuid);

async fn inject_principal(
    Extension(principal): Extension<StubPrincipal>,
    mut request: Request,
    next: axum::middleware::Next,
) -> Response {
    request
        .extensions_mut()
        .insert(atlas_server::auth::middleware::Principal::User(
            atlas_core::principal::UserId(principal.0),
        ));
    next.run(request).await
}

async fn mock_create_handler(Json(body): Json<serde_json::Value>) -> Response {
    (StatusCode::CREATED, Json(json!({"echo": body}))).into_response()
}

/// A handler whose response carries a `set-cookie` header — proving the
/// allowlist filter runs inside the real middleware call path, not just as
/// an isolated unit (the orchestrator's second added PR4 gate).
async fn mock_handler_with_cookie() -> Response {
    let mut response = (StatusCode::OK, Json(json!({"ok": true}))).into_response();
    response.headers_mut().insert(
        axum::http::header::SET_COOKIE,
        HeaderValue::from_static("atlas_session=leaked; HttpOnly"),
    );
    response.headers_mut().insert(
        axum::http::header::LOCATION,
        HeaderValue::from_static("/mock/1"),
    );
    response
}

fn mock_router(state: AppState) -> Router {
    Router::new()
        .route("/mock", post(mock_create_handler))
        .route("/mock-cookie", post(mock_handler_with_cookie))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            idempotency_middleware_release,
        ))
        .layer(axum::middleware::from_fn(inject_principal))
        .with_state(state)
}

/// R1 fix proof: a route carrying its own `DefaultBodyLimit`, wired in the
/// exact `.layer(idempotency).layer(DefaultBodyLimit::max(N))` order
/// `routes::acta::layered` uses in production (the later `.layer()` call
/// becomes the OUTER layer, so `DefaultBodyLimit` runs — and inserts its
/// extension — before this middleware does).
fn mock_router_with_body_limit(state: AppState) -> Router {
    const ROUTE_BODY_LIMIT: usize = 1024;

    Router::new()
        .route(
            "/mock-limited",
            post(mock_create_handler)
                .layer::<_, std::convert::Infallible>(axum::middleware::from_fn_with_state(
                    state.clone(),
                    idempotency_middleware_release,
                ))
                .layer(axum::extract::DefaultBodyLimit::max(ROUTE_BODY_LIMIT)),
        )
        .layer(axum::middleware::from_fn(inject_principal))
        .with_state(state)
}

/// R4 fix proof: a handler whose completion installs a trigger that blocks
/// only the middleware's own `complete()` UPDATE (`state = 'completed'`) —
/// the cheapest available way to inject a genuine store error on that
/// specific call, since [`atlas_server::persistence::repos::PgIdempotencyRepo`]
/// is a concrete type (no fake-repo seam to substitute instead). Unlike
/// dropping the whole table, `insert_in_flight`'s own INSERT/SELECT queries
/// stay fully functional, so a follow-up request against the same row still
/// observes the real degraded mode (`InFlight`, not a fresh DB error).
fn mock_router_with_dropping_handler(state: AppState, conn: sea_orm::DatabaseConnection) -> Router {
    let handler = move || {
        let conn = conn.clone();
        async move {
            sea_orm::ConnectionTrait::execute_unprepared(
                &conn,
                "CREATE OR REPLACE FUNCTION platform.block_idempotency_complete() \
                 RETURNS trigger AS $$ \
                 BEGIN RAISE EXCEPTION 'blocked for test'; END; \
                 $$ LANGUAGE plpgsql; \
                 CREATE TRIGGER block_complete \
                 BEFORE UPDATE ON platform.idempotency_keys \
                 FOR EACH ROW WHEN (NEW.state = 'completed') \
                 EXECUTE FUNCTION platform.block_idempotency_complete();",
            )
            .await
            .expect("installing the blocking trigger for this test must not itself fail");
            (StatusCode::CREATED, Json(json!({"ok": true}))).into_response()
        }
    };

    Router::new()
        .route("/mock-drop", post(handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            idempotency_middleware_release,
        ))
        .layer(axum::middleware::from_fn(inject_principal))
        .with_state(state)
}

/// D6 scoped correction (`R4-5xx-release-duplicates-one-shot-jobs`) proof: a
/// one-shot scratch route, wired to `idempotency_middleware_store_briefly`
/// exactly like `purge_trash`/`semantic_reindex_start`. The handler returns
/// 500 on its first call and 201 on every call after, tracked via a shared
/// counter — proving a 5xx is stored/replayed for the short
/// `FAILURE_RETENTION` window, and re-executes the handler only once that
/// window has lapsed.
fn mock_router_with_flaky_handler(state: AppState, calls: Arc<AtomicUsize>) -> Router {
    let handler = move || {
        let calls = calls.clone();
        async move {
            let call_number = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call_number == 1 {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok": false})),
                )
                    .into_response();
            }
            (StatusCode::CREATED, Json(json!({"ok": true}))).into_response()
        }
    };

    Router::new()
        .route("/mock-flaky", post(handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            idempotency_middleware_store_briefly,
        ))
        .layer(axum::middleware::from_fn(inject_principal))
        .with_state(state)
}

/// D6 scoped correction counterpart to [`mock_router_with_flaky_handler`]: a
/// create scratch route, wired to `idempotency_middleware_release` exactly
/// like every ordinary `idempotent: true` create. Same flaky handler shape
/// (500 on the first call, 201 after), proving a 5xx instead RELEASES the
/// row, so an immediate retry re-executes as `Fresh` rather than replaying
/// the 500.
fn mock_router_with_flaky_create_handler(state: AppState, calls: Arc<AtomicUsize>) -> Router {
    let handler = move || {
        let calls = calls.clone();
        async move {
            let call_number = calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call_number == 1 {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"ok": false})),
                )
                    .into_response();
            }
            (StatusCode::CREATED, Json(json!({"ok": true}))).into_response()
        }
    };

    Router::new()
        .route("/mock-flaky-create", post(handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            idempotency_middleware_release,
        ))
        .layer(axum::middleware::from_fn(inject_principal))
        .with_state(state)
}

/// R2 fix proof (`R2-response-buffer-silently-emptied`, ORCHESTRATOR RULING
/// 2026-09-02): a create-like route, wired to `idempotency_middleware_release`
/// exactly like `mock_router_with_flaky_create_handler`, whose handler
/// always returns a 200 with a body larger than this middleware's own
/// buffering ceiling (32 MiB, kept as a private constant in
/// `middleware::idempotency` rather than exposed through
/// [`atlas_server::middleware::idempotency::IdempotencyPolicy`] — there is
/// no product reason for a route author to ever tune it, so this test just
/// hardcodes a value one byte over it instead of widening that public
/// surface for a test-only need). Tracks how many times the handler
/// actually ran, to prove a released claim re-executes it.
fn mock_router_with_oversized_response_handler(state: AppState, calls: Arc<AtomicUsize>) -> Router {
    const OVER_BUFFERING_CEILING_BYTES: usize = 33 * 1024 * 1024;

    let handler = move || {
        let calls = calls.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            let oversized_body = vec![b'a'; OVER_BUFFERING_CEILING_BYTES];
            (StatusCode::OK, Body::from(oversized_body)).into_response()
        }
    };

    Router::new()
        .route("/mock-oversized-response", post(handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            idempotency_middleware_release,
        ))
        .layer(axum::middleware::from_fn(inject_principal))
        .with_state(state)
}

/// A simple counting handler used for the store-unavailable degraded-mode
/// proof: always 201s, tracks how many times it actually ran.
fn mock_router_with_counting_handler(state: AppState, calls: Arc<AtomicUsize>) -> Router {
    let handler = move || {
        let calls = calls.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            (StatusCode::CREATED, Json(json!({"ok": true}))).into_response()
        }
    };

    Router::new()
        .route("/mock-count", post(handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            idempotency_middleware_release,
        ))
        .layer(axum::middleware::from_fn(inject_principal))
        .with_state(state)
}

/// `inject_principal`'s `Extension`-based approach only works via
/// `tower::ServiceExt::oneshot` (an in-process `Service` call, where a
/// caller can set request extensions directly). A real client-disconnect
/// proof needs an actual TCP round trip, so there is no in-process
/// extension to set — this reads a stand-in `x-stub-principal` header
/// instead, for that one test only.
async fn inject_principal_from_header(
    mut request: Request,
    next: axum::middleware::Next,
) -> Response {
    if let Some(principal) = request
        .headers()
        .get("x-stub-principal")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
    {
        request
            .extensions_mut()
            .insert(atlas_server::auth::middleware::Principal::User(
                atlas_core::principal::UserId(principal),
            ));
    }
    next.run(request).await
}

/// R4 fix proof (`R4-inflight-residue-blocks-then-duplicates`, ORCHESTRATOR
/// RULING 2026-09-02): a handler that sleeps long enough for a client's own
/// timeout to fire first, tracked via a shared counter — proving the
/// handler still runs to completion and its response still gets recorded
/// even once nothing is left on the other end of the connection to receive
/// it.
fn mock_router_with_slow_handler(state: AppState, calls: Arc<AtomicUsize>) -> Router {
    let handler = move || {
        let calls = calls.clone();
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            calls.fetch_add(1, Ordering::SeqCst);
            (StatusCode::CREATED, Json(json!({"ok": true}))).into_response()
        }
    };

    Router::new()
        .route("/mock-slow", post(handler))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            idempotency_middleware_release,
        ))
        .layer(axum::middleware::from_fn(inject_principal_from_header))
        .with_state(state)
}

async fn state_for(db: &TestDb) -> AppState {
    AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
}

fn request_with(
    path: &str,
    key: &str,
    body: serde_json::Value,
    principal: StubPrincipal,
) -> Request {
    request_with_headers(path, key, body, principal, &[])
}

fn request_with_headers(
    path: &str,
    key: &str,
    body: serde_json::Value,
    principal: StubPrincipal,
    extra_headers: &[(&str, &str)],
) -> Request {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("idempotency-key", key)
        .extension(principal);
    for (name, value) in extra_headers {
        builder = builder.header(*name, *value);
    }
    builder
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

/// T4.1/T4.2: a matching retry short-circuits to the stored response without
/// invoking the handler again, carrying `Idempotent-Replayed: true`.
#[tokio::test]
async fn matching_retry_replays_without_rerunning_the_handler() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});

    let first = router
        .clone()
        .oneshot(request_with("/mock", "key-1", body.clone(), principal))
        .await
        .expect("first response");
    assert_eq!(first.status(), StatusCode::CREATED);
    assert!(first.headers().get("idempotent-replayed").is_none());

    let second = router
        .clone()
        .oneshot(request_with("/mock", "key-1", body, principal))
        .await
        .expect("second response");

    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(
        second
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "T4.1: the replay must carry Idempotent-Replayed: true"
    );
}

/// T4.3/T4.4: same key, different body — 409 with the D1 conflict problem
/// type, rendered through the shared `render_problem` path.
#[tokio::test]
async fn mismatched_retry_returns_409_conflict() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());

    router
        .clone()
        .oneshot(request_with(
            "/mock",
            "key-2",
            json!({"name": "a"}),
            principal,
        ))
        .await
        .expect("first response");

    let second = router
        .clone()
        .oneshot(request_with(
            "/mock",
            "key-2",
            json!({"name": "b"}),
            principal,
        ))
        .await
        .expect("second response");

    assert_eq!(second.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(second.into_body(), 64 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["type"], "urn:atlas:error:idempotency-key-conflict",
        "T4.3: must use the D1 conflict type, never revision-conflict"
    );
}

/// T4.5/T4.6: a second request while the first is still in flight gets 409
/// with a DISTINCT problem type from the mismatch case, even though both are
/// the same status code.
#[tokio::test]
async fn concurrent_in_flight_returns_409_with_a_distinct_problem_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };
    let principal_id = uuid::Uuid::now_v7();
    let scope = IdempotencyScope {
        principal_id,
        method: "POST".to_string(),
        path: "/mock".to_string(),
        key: "key-3".to_string(),
    };
    let now = Utc::now();

    // Simulate an in-flight first request directly against the store (no
    // handler ever completes it), then drive a second request through the
    // real middleware for the same scope.
    let outcome = repo
        .insert_in_flight(&scope, b"fingerprint", now, RETENTION)
        .await
        .expect("insert_in_flight");
    assert!(matches!(outcome, InsertOutcome::Fresh { .. }));

    let router = mock_router(state);
    let principal = StubPrincipal(principal_id);
    let second = router
        .oneshot(request_with(
            "/mock",
            "key-3",
            json!({"name": "a"}),
            principal,
        ))
        .await
        .expect("second response");

    assert_eq!(second.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(second.into_body(), 64 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["type"], "urn:atlas:error:idempotency-key-in-flight");
    assert_ne!(
        value["type"], "urn:atlas:error:idempotency-key-conflict",
        "T4.5: in-flight and mismatch must render distinct problem types"
    );
}

/// Orchestrator gate 1 (PR4 verify): on `CompleteOutcome::Superseded`, the
/// middleware must NOT replay — it returns the handler's own response to
/// this caller only, never a stale/foreign stored response.
#[tokio::test]
async fn superseded_complete_never_replays_and_leaves_the_reclaimed_row_in_flight() {
    let db = TestDb::create().await.expect("TestDb::create");
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };
    let principal_id = uuid::Uuid::now_v7();
    let scope = IdempotencyScope {
        principal_id,
        method: "POST".to_string(),
        path: "/mock".to_string(),
        key: "key-4".to_string(),
    };

    let t0 = Utc::now();
    let first = repo
        .insert_in_flight(&scope, b"fp", t0, RETENTION)
        .await
        .expect("first insert");
    let InsertOutcome::Fresh {
        id: first_id,
        generation: first_generation,
    } = first
    else {
        panic!("expected Fresh, got {first:?}");
    };

    // The first handler is now "slow": time advances past IN_FLIGHT_TTL
    // before it completes, so a second caller reclaims the row.
    let t1 = t0 + IN_FLIGHT_TTL + Duration::seconds(1);
    let second = repo
        .insert_in_flight(&scope, b"fp", t1, RETENTION)
        .await
        .expect("reclaim insert");
    let InsertOutcome::Fresh {
        id: second_id,
        generation: second_generation,
    } = second
    else {
        panic!("expected the stale row to be reclaimed as Fresh, got {second:?}");
    };
    assert_eq!(second_id, first_id, "reclaim keeps the same row id");
    assert_ne!(
        second_generation, first_generation,
        "reclaim mints a fresh generation (F2)"
    );

    // The (slow) first handler now finishes and calls complete() with its
    // now-stale generation.
    let first_complete = repo
        .complete(
            first_id,
            first_generation,
            201,
            b"{\"from\":\"first\"}",
            None,
            Utc::now(),
            RETENTION,
        )
        .await
        .expect("first complete must not error");
    assert_eq!(
        first_complete,
        CompleteOutcome::Superseded,
        "the first handler's completion must be Superseded, not Stored"
    );

    // The row must still belong to the second (reclaiming) caller: a lookup
    // with the ORIGINAL fingerprint/scope must not show the first handler's
    // body, since the row is still in_flight under the second generation.
    let lookup = repo
        .lookup(&scope, Utc::now())
        .await
        .expect("lookup must not error");
    assert!(
        lookup.is_none(),
        "the row is still in_flight (owned by the second caller), so lookup must see nothing \
         to replay — the first handler's Superseded completion must not have overwritten it"
    );
}

/// D5 / orchestrator gate 2 (PR4 verify): a `set-cookie` header from the
/// handler's real response must be neither stored nor replayed, proven
/// through the actual middleware call path (not just the isolated filter
/// unit test in `middleware::idempotency::tests`).
#[tokio::test]
async fn set_cookie_is_never_stored_or_replayed() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());

    let first = router
        .clone()
        .oneshot(request_with("/mock-cookie", "key-5", json!({}), principal))
        .await
        .expect("first response");
    assert!(
        first.headers().get("set-cookie").is_some(),
        "sanity: the handler itself does set a cookie"
    );

    let second = router
        .oneshot(request_with("/mock-cookie", "key-5", json!({}), principal))
        .await
        .expect("second (replayed) response");

    assert_eq!(
        second
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );
    assert!(
        second.headers().get("set-cookie").is_none(),
        "a replayed response must never carry a set-cookie header"
    );
    assert_eq!(
        second
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/mock/1"),
        "an allowlisted header (location) must still replay correctly"
    );
}

/// T4.21/T4.22: a request that fails `require_authn` (401, no valid
/// session/API key) while carrying an `Idempotency-Key` header, driven
/// through the REAL production layer stack, must leave no row in the store
/// — the middleware is mounted innermost (D6), so a pre-execution 401 never
/// reaches it.
#[tokio::test]
async fn a_request_that_fails_authn_leaves_no_row_in_the_store() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) = support::login_user_with_workspace(&server, &db, "idem-authn").await;

    let http = client.http_client();
    let response = http
        .post(format!(
            "{}/api/workspaces/{}/tags",
            server.base_url(),
            ws.slug
        ))
        .header("authorization", "Bearer atlas_this-token-does-not-exist")
        .header("idempotency-key", "authn-fail-key")
        .header("x-atlas-csrf", "1")
        .json(&json!({"name": "should-not-be-created"}))
        .send()
        .await
        .expect("request must complete");

    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);

    // No principal exists for a failed-authn request, so there is no
    // meaningful (principal_id, ...) scope to look up; the authoritative
    // proof is a direct count query for this key across all principals.
    let count: i64 = sea_orm::ConnectionTrait::query_one_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*) AS c FROM platform.idempotency_keys WHERE key = $1",
            ["authn-fail-key".into()],
        ),
    )
    .await
    .expect("count query must not error")
    .and_then(|row| row.try_get::<i64>("", "c").ok())
    .unwrap_or(0);

    assert_eq!(
        count, 0,
        "a request rejected by require_authn must leave zero rows for its Idempotency-Key"
    );
}

/// Untested spec scenario (verify report finding): "A response MUST be
/// stored once handler execution has begun, including 4xx/5xx responses
/// produced during or after execution", replayed "byte-identical in status
/// and body" on retry.
///
/// `add_member` (`/api/workspaces/{ws}/members`, declared `idempotent:
/// true`) is chosen because its 404 comes from a real domain check INSIDE
/// the handler body (`user_repo.find_by_id(target_user_id)`,
/// `routes/members.rs`), well after `WorkspaceOwnerOrAdmin`'s pre-execution
/// role check and the `Json<AddMemberRequest>` extractor have both already
/// succeeded — a valid-shaped `user_id` that simply does not exist fails
/// ONLY this post-execution check, unlike a 401/403/429 or a malformed-body
/// rejection, both of which never reach `next.run(request)` at all.
///
/// `problem_stamp` runs OUTSIDE this middleware (it wraps the whole router,
/// while the idempotency layer wraps only the route), so it stores the RAW,
/// unstamped problem body and stamps `request_id` fresh on every response —
/// first response and replay alike, each from its own `x-request-id`. That
/// is correct: `request_id` identifies the request, not the stored outcome.
/// The two problem bodies are therefore compared field-by-field, excluding
/// `request_id`.
#[tokio::test]
async fn a_post_execution_404_is_stored_and_replayed_like_any_completed_response() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "idem-post-exec-404").await;

    let nonexistent_user_id = uuid::Uuid::now_v7();
    let key = "post-exec-404-key";
    let path = format!("/api/workspaces/{}/members", ws.slug);
    let token = client.token().expect("session token").to_string();

    let send_request = || {
        client
            .http_client()
            .post(format!("{}{}", server.base_url(), path))
            .bearer_auth(&token)
            .header("x-atlas-csrf", "1")
            .header("idempotency-key", key)
            .json(&json!({ "user_id": nonexistent_user_id, "role": "member" }))
            .send()
    };

    let first = send_request().await.expect("first request must complete");
    assert_eq!(
        first.status(),
        reqwest::StatusCode::NOT_FOUND,
        "a nonexistent user_id must fail members::add_member's domain check with 404"
    );
    assert!(
        first.headers().get("idempotent-replayed").is_none(),
        "the ORIGINAL response must never carry Idempotent-Replayed"
    );
    let request_id1 = first
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .expect("first response must carry x-request-id");
    let body1 = first.bytes().await.expect("read first response body");

    let second = send_request().await.expect("second request must complete");
    let status2 = second.status();
    let replayed = second
        .headers()
        .get("idempotent-replayed")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let request_id2 = second
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .expect("second response must carry x-request-id");
    let body2 = second.bytes().await.expect("read second response body");

    assert_eq!(
        status2,
        reqwest::StatusCode::NOT_FOUND,
        "the replay must carry the SAME 404 status as the first response"
    );

    let mut problem1: serde_json::Value =
        serde_json::from_slice(&body1).expect("first response body must be JSON");
    let mut problem2: serde_json::Value =
        serde_json::from_slice(&body2).expect("replay body must be JSON");
    let stamped_request_id1 = problem1
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let stamped_request_id2 = problem2
        .get("request_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    assert_eq!(
        stamped_request_id1.as_deref(),
        Some(request_id1.as_str()),
        "problem_stamp must stamp the first response's OWN x-request-id"
    );
    assert_eq!(
        stamped_request_id2.as_deref(),
        Some(request_id2.as_str()),
        "problem_stamp must stamp the replay's OWN x-request-id, not the first response's"
    );
    assert_ne!(
        stamped_request_id1, stamped_request_id2,
        "each response's request_id identifies THAT request, never the stored outcome — \
         a replay is still stamped with ITS OWN request_id, so a replayed problem body \
         is never byte-identical to the first response's"
    );
    if let Some(map1) = problem1.as_object_mut() {
        map1.remove("request_id");
    }
    if let Some(map2) = problem2.as_object_mut() {
        map2.remove("request_id");
    }
    assert_eq!(
        problem2, problem1,
        "the replay's problem body must be identical to the first response's, \
         aside from each response's own stamped request_id"
    );
    assert_eq!(
        replayed.as_deref(),
        Some("true"),
        "a stored post-execution 4xx must replay with Idempotent-Replayed: true"
    );

    let row = sea_orm::ConnectionTrait::query_one_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT state, response_status FROM platform.idempotency_keys \
             WHERE principal_id = $1 AND method = $2 AND path = $3 AND key = $4",
            [
                user.id.0.into(),
                "POST".into(),
                path.clone().into(),
                key.into(),
            ],
        ),
    )
    .await
    .expect("row lookup must not error")
    .expect("a completed row must exist for this principal/method/path/key");

    let state: String = row.try_get("", "state").expect("state column");
    let response_status: i16 = row
        .try_get("", "response_status")
        .expect("response_status column");
    assert_eq!(
        state, "completed",
        "a fully-handled 404 must be stored as completed, never left in_flight"
    );
    assert_eq!(
        response_status, 404,
        "the stored response_status must match the handler's actual 404"
    );
}

/// R1 fix: a body over the route's OWN `DefaultBodyLimit` (1 KiB here, well
/// under this middleware's 32 MiB buffering ceiling) must be rejected with
/// 413 by this middleware itself, and must leave no row behind — proving
/// the fix reads the route's limit via `RequestExt::with_limited_body()`
/// instead of only enforcing its own much larger ceiling.
#[tokio::test]
async fn body_over_the_routes_own_default_body_limit_returns_413_and_leaves_no_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router_with_body_limit(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());

    let oversized_value = "x".repeat(2 * 1024);
    let body = json!({"data": oversized_value});

    let response = router
        .oneshot(request_with(
            "/mock-limited",
            "big-body-key",
            body,
            principal,
        ))
        .await
        .expect("response");

    assert_eq!(
        response.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "a body over the route's own DefaultBodyLimit must be rejected by this middleware, \
         not silently buffered up to its own 32 MiB ceiling"
    );

    let count: i64 = sea_orm::ConnectionTrait::query_one_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*) AS c FROM platform.idempotency_keys WHERE key = $1",
            ["big-body-key".into()],
        ),
    )
    .await
    .expect("count query must not error")
    .and_then(|row| row.try_get::<i64>("", "c").ok())
    .unwrap_or(0);

    assert_eq!(
        count, 0,
        "a request rejected for exceeding the route's own body limit must leave no row"
    );
}

/// R3 fix: the fingerprint now covers the query string. Same key, same
/// body, different query string on the same concrete path — 409 conflict,
/// not a silent cross-target replay.
#[tokio::test]
async fn same_key_and_body_but_different_query_string_returns_409_conflict() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});

    router
        .clone()
        .oneshot(request_with(
            "/mock?scope=a",
            "key-query",
            body.clone(),
            principal,
        ))
        .await
        .expect("first response");

    let second = router
        .oneshot(request_with("/mock?scope=b", "key-query", body, principal))
        .await
        .expect("second response");

    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "a different query string under the same key/body must conflict, never replay"
    );
}

/// R4 fix (`R4-fingerprint-denylist-breaks-real-retries`): `x-file-name` is
/// read only by `idempotent: false` attachment-upload handlers, so it is not
/// on `FINGERPRINT_HEADER_ALLOWLIST` — a differing `x-file-name` must replay,
/// not conflict.
#[tokio::test]
async fn differing_x_file_name_header_alone_still_replays() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});

    router
        .clone()
        .oneshot(request_with_headers(
            "/mock",
            "key-file-name",
            body.clone(),
            principal,
            &[("x-file-name", "a.pdf")],
        ))
        .await
        .expect("first response");

    let second = router
        .oneshot(request_with_headers(
            "/mock",
            "key-file-name",
            body,
            principal,
            &[("x-file-name", "b.pdf")],
        ))
        .await
        .expect("second response");

    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(
        second
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "a differing x-file-name alone must replay, since it is not on the fingerprint allowlist"
    );
}

/// R4 fix: `x-create-token` IS on `FINGERPRINT_HEADER_ALLOWLIST`, because
/// `tasks::create_comment_draft` and `documents::create_comment_draft` (both
/// `idempotent: true`) read it as the comment-draft's own replay token —
/// same key, same body, different `x-create-token` — 409 conflict.
#[tokio::test]
async fn same_key_and_body_but_different_x_create_token_header_returns_409_conflict() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});

    router
        .clone()
        .oneshot(request_with_headers(
            "/mock",
            "key-create-token",
            body.clone(),
            principal,
            &[("x-create-token", "3f3a6a9e-8f0e-4b8a-9f0e-4b8a9f0e4b8a")],
        ))
        .await
        .expect("first response");

    let second = router
        .oneshot(request_with_headers(
            "/mock",
            "key-create-token",
            body,
            principal,
            &[("x-create-token", "7c1d9b2a-2f0e-4b8a-9f0e-4b8a9f0e7c1d")],
        ))
        .await
        .expect("second response");

    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "a different x-create-token under the same key/body must conflict, never replay"
    );
}

/// R4 fix: `user-agent` was never on the allowlist — a differing `user-agent`
/// alone must still replay, not conflict.
#[tokio::test]
async fn differing_user_agent_alone_still_replays() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});

    router
        .clone()
        .oneshot(request_with_headers(
            "/mock",
            "key-ua",
            body.clone(),
            principal,
            &[("user-agent", "agent-a/1.0")],
        ))
        .await
        .expect("first response");

    let second = router
        .oneshot(request_with_headers(
            "/mock",
            "key-ua",
            body,
            principal,
            &[("user-agent", "agent-b/2.0")],
        ))
        .await
        .expect("second response");

    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(
        second
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "a differing user-agent alone must still replay, since it is not on the fingerprint allowlist"
    );
}

/// R4 fix (`R4-fingerprint-denylist-breaks-real-retries`, the corroborated
/// finding this correction addresses): a byte-identical retry that merely
/// picked up a different B3/Zipkin trace header per hop must still replay,
/// not 409 — `x-b3-traceid` is not on `FINGERPRINT_HEADER_ALLOWLIST`.
#[tokio::test]
async fn byte_identical_retry_with_differing_b3_trace_header_replays() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router(state);
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});

    router
        .clone()
        .oneshot(request_with_headers(
            "/mock",
            "key-b3-trace",
            body.clone(),
            principal,
            &[("x-b3-traceid", "80f198ee56343ba864fe8b2a57d3eff7")],
        ))
        .await
        .expect("first response");

    let second = router
        .oneshot(request_with_headers(
            "/mock",
            "key-b3-trace",
            body,
            principal,
            &[("x-b3-traceid", "9a1f2c3d4e5f60718293a4b5c6d7e8f9")],
        ))
        .await
        .expect("second response");

    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(
        second
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "a retry that only differs in x-b3-traceid must replay, not 409, \
         since it is not on the fingerprint allowlist"
    );
}

/// R4 fix: when `complete()` itself fails after the handler already ran,
/// the middleware must fix-forward — return the handler's real response to
/// THIS caller (never a 500 masking a committed write), rather than
/// propagating the store error. Extended (`R4-inflight-residue-blocks-then-
/// duplicates`, ORCHESTRATOR RULING 2026-09-02) with the documented degraded
/// mode's other half: the row stays `in_flight`, so an immediate identical
/// retry answers 409 (never silently re-executes the handler within the
/// staleness window).
#[tokio::test]
async fn a_complete_store_error_returns_the_handlers_real_response_instead_of_500() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let router = mock_router_with_dropping_handler(state, db.conn().clone());
    let principal = StubPrincipal(uuid::Uuid::now_v7());

    let response = router
        .clone()
        .oneshot(request_with(
            "/mock-drop",
            "key-drop",
            json!({"name": "first"}),
            principal,
        ))
        .await
        .expect("response");

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a complete() store failure must never mask the handler's own committed response \
         behind a 500 — it must fix-forward to the handler's real response"
    );
    assert!(
        response.headers().get("idempotent-replayed").is_none(),
        "a fix-forwarded response is this caller's own handler response, never a replay"
    );

    let retry = router
        .oneshot(request_with(
            "/mock-drop",
            "key-drop",
            json!({"name": "first"}),
            principal,
        ))
        .await
        .expect("retry response");

    assert_eq!(
        retry.status(),
        StatusCode::CONFLICT,
        "the row stays in_flight after a swallowed complete() failure, so an immediate \
         retry within the staleness window must answer 409, not silently re-execute the \
         handler — the documented degraded mode, made explicit rather than left hidden"
    );
}

/// R4 fix proof (`R4-inflight-residue-blocks-then-duplicates`, ORCHESTRATOR
/// RULING 2026-09-02): the client disconnects mid-request (its own
/// timeout) — the canonical reason a caller sends an `Idempotency-Key` at
/// all — while the handler is still running. The handler must still run to
/// completion and `complete()` must still record it, so a retry once the
/// staleness window for the (now-orphaned) in-flight claim would otherwise
/// matter instead sees the real replay, never a re-execution.
///
/// This needs a genuine TCP round trip (a `tower::ServiceExt::oneshot` call
/// never has a connection for the client to drop), so it binds
/// `mock_router_with_slow_handler` on a real port via `axum::serve` and
/// drives it with a `reqwest` client carrying a 50ms timeout.
#[tokio::test]
async fn a_client_disconnect_mid_handler_still_records_the_completed_response() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let router = mock_router_with_slow_handler(state, calls.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service())
            .await
            .expect("serve");
    });

    let principal = uuid::Uuid::now_v7();
    let key = "disconnect-key";
    let body = json!({"name": "first"});

    let impatient = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(50))
        .build()
        .expect("impatient client");

    let first = impatient
        .post(format!("http://{addr}/mock-slow"))
        .header("content-type", "application/json")
        .header("idempotency-key", key)
        .header("x-stub-principal", principal.to_string())
        .json(&body)
        .send()
        .await;

    assert!(
        first.is_err(),
        "the 50ms client timeout must fire before the handler's 300ms sleep completes"
    );

    tokio::time::sleep(std::time::Duration::from_millis(600)).await;

    let patient = reqwest::Client::new();
    let second = patient
        .post(format!("http://{addr}/mock-slow"))
        .header("content-type", "application/json")
        .header("idempotency-key", key)
        .header("x-stub-principal", principal.to_string())
        .json(&body)
        .send()
        .await
        .expect("second request");

    assert_eq!(
        second.status(),
        reqwest::StatusCode::CREATED,
        "the handler's real response must still be recorded after the disconnect"
    );
    assert_eq!(
        second
            .headers()
            .get("idempotent-replayed")
            .and_then(|value| value.to_str().ok()),
        Some("true"),
        "a replay proves complete() ran and recorded the response despite the disconnect"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the handler must have executed exactly once despite the client's disconnect"
    );
}

/// `R4-5xx-release-duplicates-one-shot-jobs` fix (ORCHESTRATOR DECISION,
/// 2026-09-02, D6 scoped correction), `StoreBriefly` policy: a 5xx from the
/// handler IS stored and replayed like a 2xx/4xx (the Stripe posture), but
/// only for the short `FAILURE_RETENTION` window — an immediate retry gets
/// the same 500 back without re-running the handler, while a retry after
/// the window has lapsed re-executes it, so a poisoned key clears itself in
/// minutes instead of duplicating a one-shot job (`purge_trash`,
/// `semantic_reindex_start`, ...) forever.
#[tokio::test]
async fn a_5xx_is_stored_for_the_failure_retention_and_then_re_executes() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let router = mock_router_with_flaky_handler(state, calls.clone());
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});
    let before_first = Utc::now();

    let first = router
        .clone()
        .oneshot(request_with(
            "/mock-flaky",
            "key-flaky",
            body.clone(),
            principal,
        ))
        .await
        .expect("first response");
    assert_eq!(first.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        first.headers().get("idempotent-replayed").is_none(),
        "the original 5xx response is never itself marked as a replay"
    );

    let row = sea_orm::ConnectionTrait::query_one_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT state, response_status, expires_at FROM platform.idempotency_keys \
             WHERE principal_id = $1 AND method = $2 AND path = $3 AND key = $4",
            [
                principal.0.into(),
                "POST".into(),
                "/mock-flaky".into(),
                "key-flaky".into(),
            ],
        ),
    )
    .await
    .expect("row lookup must not error")
    .expect("a 5xx must be stored, not released");

    let state: String = row.try_get("", "state").expect("state column");
    let response_status: i16 = row.try_get("", "response_status").expect("status column");
    let expires_at: chrono::DateTime<Utc> =
        row.try_get("", "expires_at").expect("expires_at column");
    assert_eq!(
        state, "completed",
        "a 5xx row must be completed, not left in_flight"
    );
    assert_eq!(response_status, 500);
    let expected_expiry = before_first + Duration::minutes(5);
    assert!(
        (expires_at - expected_expiry).num_seconds().abs() < 30,
        "a 5xx must be retained for FAILURE_RETENTION (5 minutes), not the normal retention; \
         got expires_at={expires_at}, expected close to {expected_expiry}"
    );

    let second = router
        .clone()
        .oneshot(request_with(
            "/mock-flaky",
            "key-flaky",
            body.clone(),
            principal,
        ))
        .await
        .expect("second response");
    assert_eq!(
        second.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "an immediate retry within the failure-retention window must replay the stored 500"
    );
    assert_eq!(
        second
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "the replayed 500 must carry Idempotent-Replayed: true"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a replay must never invoke the handler again"
    );

    sea_orm::ConnectionTrait::execute_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "UPDATE platform.idempotency_keys SET expires_at = $1 \
             WHERE principal_id = $2 AND method = $3 AND path = $4 AND key = $5",
            [
                (Utc::now() - Duration::minutes(1)).into(),
                principal.0.into(),
                "POST".into(),
                "/mock-flaky".into(),
                "key-flaky".into(),
            ],
        ),
    )
    .await
    .expect("forcing expiry must not error");

    let third = router
        .oneshot(request_with("/mock-flaky", "key-flaky", body, principal))
        .await
        .expect("third response");
    assert_eq!(
        third.status(),
        StatusCode::CREATED,
        "once the failure-retention window has lapsed, a retry must re-execute the handler"
    );
    assert!(
        third.headers().get("idempotent-replayed").is_none(),
        "a re-executed Fresh response is never a replay"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the handler must have run exactly twice: once for the 5xx, once for the re-execution"
    );
}

/// D6 scoped correction, `Release` policy counterpart to the `StoreBriefly`
/// test above: for an ordinary create, a 5xx RELEASES the row instead of
/// storing it, so the very next attempt re-executes as `Fresh` (not just
/// after `FAILURE_RETENTION` lapses) — the row is gone, not merely expired.
#[tokio::test]
async fn a_5xx_release_deletes_the_row_and_the_next_attempt_re_executes() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let router = mock_router_with_flaky_create_handler(state, calls.clone());
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});

    let first = router
        .clone()
        .oneshot(request_with(
            "/mock-flaky-create",
            "key-flaky-create",
            body.clone(),
            principal,
        ))
        .await
        .expect("first response");
    assert_eq!(first.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        first.headers().get("idempotent-replayed").is_none(),
        "the original 5xx response is never itself marked as a replay"
    );

    let row_after_first = sea_orm::ConnectionTrait::query_one_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT state FROM platform.idempotency_keys \
             WHERE principal_id = $1 AND method = $2 AND path = $3 AND key = $4",
            [
                principal.0.into(),
                "POST".into(),
                "/mock-flaky-create".into(),
                "key-flaky-create".into(),
            ],
        ),
    )
    .await
    .expect("row lookup must not error");
    assert!(
        row_after_first.is_none(),
        "a 5xx on the Release policy must delete the row outright, not leave it \
         completed or in_flight"
    );

    let second = router
        .clone()
        .oneshot(request_with(
            "/mock-flaky-create",
            "key-flaky-create",
            body.clone(),
            principal,
        ))
        .await
        .expect("second response");
    assert_eq!(
        second.status(),
        StatusCode::CREATED,
        "with the row released, the very next attempt must re-execute the handler, \
         not replay the 500"
    );
    assert!(
        second.headers().get("idempotent-replayed").is_none(),
        "a re-executed Fresh response is never a replay"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "the handler must have run exactly twice so far: once for the 5xx, once for \
         the re-execution"
    );

    let third = router
        .oneshot(request_with(
            "/mock-flaky-create",
            "key-flaky-create",
            body,
            principal,
        ))
        .await
        .expect("third response");
    assert_eq!(
        third.status(),
        StatusCode::CREATED,
        "the second call's 201 was completed normally and must now replay"
    );
    assert_eq!(
        third
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true"),
        "an identical retry against the completed 201 must carry Idempotent-Replayed: true"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "a replay must never invoke the handler again"
    );
}

/// R4 fix proof (`R4-store-write-is-a-hard-precondition`, ORCHESTRATOR
/// RULING 2026-09-02): when `insert_in_flight` itself fails (the store is
/// unavailable BEFORE the handler ever runs), the middleware must degrade to
/// no-dedup rather than turn the store into a hard availability dependency —
/// the handler still runs, gets its normal response, and the response
/// carries `Idempotency-Degraded: store-unavailable` instead of a stored
/// row. A `BEFORE INSERT` trigger on `platform.idempotency_keys` (mirroring
/// the existing `BEFORE UPDATE` trigger used for the complete()-failure
/// proof above) is the cheapest way to force that specific failure.
#[tokio::test]
async fn an_insert_in_flight_store_failure_degrades_to_no_dedup_instead_of_blocking() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let router = mock_router_with_counting_handler(state, calls.clone());
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});
    let key = "key-insert-blocked";

    sea_orm::ConnectionTrait::execute_unprepared(
        db.conn(),
        "CREATE OR REPLACE FUNCTION platform.block_idempotency_insert() \
         RETURNS trigger AS $$ \
         BEGIN RAISE EXCEPTION 'blocked for test'; END; \
         $$ LANGUAGE plpgsql; \
         CREATE TRIGGER block_insert \
         BEFORE INSERT ON platform.idempotency_keys \
         FOR EACH ROW EXECUTE FUNCTION platform.block_idempotency_insert();",
    )
    .await
    .expect("installing the blocking trigger for this test must not itself fail");

    let first = router
        .clone()
        .oneshot(request_with("/mock-count", key, body.clone(), principal))
        .await
        .expect("first response");

    assert_eq!(
        first.status(),
        StatusCode::CREATED,
        "insert_in_flight failing must never turn the store into a hard availability \
         dependency for a route the client did nothing wrong to reach"
    );
    assert_eq!(
        first
            .headers()
            .get("idempotency-degraded")
            .and_then(|v| v.to_str().ok()),
        Some("store-unavailable"),
        "a degraded response must carry Idempotency-Degraded: store-unavailable"
    );
    assert!(
        first.headers().get("idempotent-replayed").is_none(),
        "a degraded response was never stored, so it can never be a replay"
    );

    let count_after_first: i64 = sea_orm::ConnectionTrait::query_one_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*) AS c FROM platform.idempotency_keys WHERE key = $1",
            [key.into()],
        ),
    )
    .await
    .expect("count query must not error")
    .and_then(|row| row.try_get::<i64>("", "c").ok())
    .unwrap_or(-1);
    assert_eq!(
        count_after_first, 0,
        "a store outage before the handler runs must leave no row behind"
    );

    let second = router
        .clone()
        .oneshot(request_with("/mock-count", key, body.clone(), principal))
        .await
        .expect("second response");

    assert_eq!(
        second.status(),
        StatusCode::CREATED,
        "the second request must also degrade to no-dedup while the store stays down"
    );
    assert_eq!(
        second
            .headers()
            .get("idempotency-degraded")
            .and_then(|v| v.to_str().ok()),
        Some("store-unavailable"),
        "the second degraded request must also carry the header"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "with the store down, dedup cannot happen: an identical retry must re-execute \
         the handler, not be blocked or replayed"
    );

    sea_orm::ConnectionTrait::execute_unprepared(
        db.conn(),
        "DROP TRIGGER block_insert ON platform.idempotency_keys",
    )
    .await
    .expect("dropping the blocking trigger must not error");

    let third = router
        .oneshot(request_with("/mock-count", key, body, principal))
        .await
        .expect("third response");

    assert_eq!(
        third.status(),
        StatusCode::CREATED,
        "once the store recovers, the handler must still run normally"
    );
    assert!(
        third.headers().get("idempotency-degraded").is_none(),
        "once the store recovers, the response must no longer be marked degraded"
    );
    assert!(
        third.headers().get("idempotent-replayed").is_none(),
        "with no prior row (every earlier attempt was degraded, none stored), \
         the store-recovered request follows the normal Fresh path, not a replay"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "the handler must have run exactly three times: twice degraded, once Fresh"
    );
}

/// R2 fix (`R2-response-buffer-silently-emptied`, ORCHESTRATOR RULING
/// 2026-09-02): a handler's response body over this middleware's own
/// buffering ceiling must never be silently swapped for an empty one and
/// returned/stored as if it were the handler's real output. This caller
/// gets an honest 500, the claim is released (no row survives), and an
/// immediate retry re-executes the handler rather than replaying a
/// fabricated empty response.
#[tokio::test]
async fn an_oversized_response_body_returns_500_and_releases_the_claim() {
    let db = TestDb::create().await.expect("TestDb::create");
    let state = state_for(&db).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let router = mock_router_with_oversized_response_handler(state, calls.clone());
    let principal = StubPrincipal(uuid::Uuid::now_v7());
    let body = json!({"name": "first"});

    let first = router
        .clone()
        .oneshot(request_with(
            "/mock-oversized-response",
            "key-oversized",
            body.clone(),
            principal,
        ))
        .await
        .expect("first response");

    assert_eq!(first.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        first.headers().get("idempotent-replayed").is_none(),
        "a fabricated response must never be marked as a replay"
    );
    let bytes = axum::body::to_bytes(first.into_body(), 64 * 1024)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["type"], "urn:atlas:error:internal",
        "a buffering failure must answer the generic internal-error problem type, \
         never a fabricated success"
    );

    let row_after_first = sea_orm::ConnectionTrait::query_one_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT state FROM platform.idempotency_keys \
             WHERE principal_id = $1 AND method = $2 AND path = $3 AND key = $4",
            [
                principal.0.into(),
                "POST".into(),
                "/mock-oversized-response".into(),
                "key-oversized".into(),
            ],
        ),
    )
    .await
    .expect("row lookup must not error");
    assert!(
        row_after_first.is_none(),
        "a buffering failure must release the claim outright, leaving no row behind"
    );

    let second = router
        .oneshot(request_with(
            "/mock-oversized-response",
            "key-oversized",
            body,
            principal,
        ))
        .await
        .expect("second response");
    assert_eq!(
        second.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "the same oversized handler still fails buffering on the re-execution"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "with the claim released, an immediate identical retry must re-execute \
         the handler rather than replay anything"
    );
}
