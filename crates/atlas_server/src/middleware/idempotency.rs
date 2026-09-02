//! `Idempotency-Key` middleware (design D1/D2/D3/D4/D5/D6, `v2-e3-s3` PR4,
//! plus a scoped correction to `R4-5xx-release-duplicates-one-shot-jobs`).
//!
//! Sits innermost on a declared-idempotent route's `MethodRouter`, applied by
//! `component_routes!`'s `idempotent`/`one_shot` modifiers (or hand-wired at
//! the same `.route()` call site for the ten routes outside the macro), so
//! the layer order is `authn -> rate_limit -> csrf -> idempotency ->
//! handler` (D6): a pre-execution 401/403/429 never reaches this middleware
//! at all, so nothing is ever stored for it (T4.21/T4.22).
//!
//! The mechanism reads [`PgIdempotencyRepo`] (PR3) and implements D2's
//! branch table:
//! - `Fresh { id, generation }` — run the handler, filter its response
//!   headers to the allowlist, then branch on the handler's status. A
//!   2xx/4xx response is always stored for the normal retention (the Stripe
//!   posture: replaying a known outcome is safe). A 5xx's handling is
//!   [`IdempotencyPolicy`]-dependent, carried structurally by which of the
//!   two entry points below wired the layer, never read from the registry at
//!   request time:
//!   - [`OnServerError::StoreBriefly`] ([`idempotency_middleware_store_briefly`],
//!     wired by `component_routes!`'s `one_shot` modifier for the routes
//!     named in `router_audit::ONE_SHOT_IDEMPOTENT_ROUTES`) stores the 5xx like a
//!     2xx/4xx, but only for [`FAILURE_RETENTION`], a short window: these
//!     routes enqueue a one-shot side effect with no domain uniqueness check
//!     to catch a duplicate, so replaying the unknown outcome briefly is
//!     safer than silently re-running the handler; an immediate retry gets
//!     the same response back, and a retry after the window lapses
//!     re-executes.
//!   - [`OnServerError::Release`] ([`idempotency_middleware_release`], the
//!     default for every other `idempotent: true` route) instead calls
//!     [`PgIdempotencyRepo::release`], deleting the `in_flight` row so an
//!     immediate retry re-executes the handler as `Fresh` — correct for an
//!     ordinary `create_*` route, whose duplicate a domain check (a
//!     duplicate-slug 409, a unique-constraint violation) already catches.
//!
//!   On [`CompleteOutcome::Superseded`] (a stale-reclaim race resolved in a
//!   later caller's favor while this handler was still running), this
//!   caller still receives its own handler's real response — it is simply
//!   not the response future replays see; the successor's row already owns
//!   the scope. `PgIdempotencyRepo::release`'s `ReleaseOutcome::Superseded`
//!   is the same race, on the `Release` path: this caller's response is
//!   still returned unmarked, the successor's row is left untouched.
//! - `InFlight` — a concurrent duplicate is still executing; 409
//!   `ApiError::IdempotencyKeyInFlight`.
//! - `Replay(stored)` — the same request was already completed; return the
//!   stored status/body/allowlisted headers plus `Idempotent-Replayed: true`.
//! - `Mismatch { existing_fingerprint }` — the same key was reused for a
//!   different request; 409 `ApiError::IdempotencyKeyConflict`.
//!
//! A missing `Idempotency-Key` header passes through untouched (no row is
//! ever created). A missing [`Principal`] (should not happen after
//! `require_authn`, but the login/activate/webhook-ingest routes that skip
//! `require_authn` entirely are exactly the routes D8 classifies
//! `idempotent: false` for this reason) also passes through untouched — the
//! mechanism has no `principal_id` to scope by without one.
//!
//! The store never makes a route unavailable (ORCHESTRATOR RULING,
//! 2026-09-02, `R4-store-write-is-a-hard-precondition`): a store outage
//! *before* the handler degrades to no-dedup, running the handler with no
//! row and no replay and signalling the caller via
//! [`IDEMPOTENCY_DEGRADED_HEADER`]; a store outage *after* the handler (the
//! existing `complete()` failure path above) is fix-forward, since the
//! handler's side effect has already committed.

use axum::RequestExt;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

use crate::auth::middleware::Principal;
use crate::error::ApiError;
use crate::persistence::repos::{IdempotencyScope, InsertOutcome, PgIdempotencyRepo};
use crate::state::AppState;

/// A route's 5xx handling policy (D6 scoped correction,
/// `R4-5xx-release-duplicates-one-shot-jobs`), carried structurally by which
/// of [`idempotency_middleware_release`]/[`idempotency_middleware_store_briefly`]
/// `component_routes!` wired the route's layer to — never read from the
/// registry at request time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdempotencyPolicy {
    pub on_server_error: OnServerError,
}

/// See [`IdempotencyPolicy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnServerError {
    /// Store the 5xx like a 2xx/4xx, but only for [`FAILURE_RETENTION`]: for
    /// a one-shot side effect with no domain uniqueness check to catch a
    /// duplicate (`router_audit::ONE_SHOT_IDEMPOTENT_ROUTES`).
    StoreBriefly,
    /// Delete the `in_flight` row (`PgIdempotencyRepo::release`) so the next
    /// attempt re-executes as `Fresh`: for an ordinary create, whose
    /// duplicate a domain check already catches.
    Release,
}

/// The client-supplied header this middleware reads.
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// The response header marking a replayed response (D5). Never present on
/// the original (first) response for a key.
const IDEMPOTENT_REPLAYED_HEADER: &str = "idempotent-replayed";

/// The response header marking a request served without dedup because
/// `insert_in_flight` itself failed (ORCHESTRATOR RULING, 2026-09-02,
/// `R4-store-write-is-a-hard-precondition`). Never present alongside
/// [`IDEMPOTENT_REPLAYED_HEADER`]: a degraded request never wrote a row to
/// replay from.
const IDEMPOTENCY_DEGRADED_HEADER: &str = "idempotency-degraded";

/// Value of [`IDEMPOTENCY_DEGRADED_HEADER`] for the store-unavailable case.
const IDEMPOTENCY_DEGRADED_STORE_UNAVAILABLE: &str = "store-unavailable";

/// Retention applied to a 5xx response instead of the normal
/// `idempotency_retention_hours` (`R4-5xx-release-duplicates-one-shot-jobs`,
/// ORCHESTRATOR DECISION 2026-09-02): a handler's outcome is unknown when it
/// returns a 5xx, so the stored replay is only good for a short window —
/// long enough to answer an immediate client retry with the same response,
/// short enough that a later retry re-executes the handler instead of
/// replaying a possibly-wrong outcome forever.
const FAILURE_RETENTION: Duration = Duration::minutes(5);

/// Request-header allowlist for the fingerprint hash (D4/D5, R4 fix
/// `R4-fingerprint-denylist-breaks-real-retries`, ORCHESTRATOR RULING
/// 2026-09-02): a denylist could never keep up with every proxy/tracing
/// header a client's hop chain might inject per attempt (B3/Zipkin,
/// `x-amzn-trace-id`, `date`, ...), so only the headers an `idempotent: true`
/// route handler is actually known to read are hashed. Add a new entry only
/// when a handler on such a route genuinely reads it:
///
/// - `content-type` — shapes how every handler parses the body.
/// - `x-create-token` — read by `tasks::create_comment_draft` and
///   `documents::create_comment_draft` (both `idempotent: true`) as the
///   comment-draft's own replay token.
///
/// `x-upload-token`/`x-file-name` (comment-draft and attachment uploads) and
/// `x-hub-signature-256`/`x-github-delivery`/`x-github-event` (webhook
/// ingest) are read only by `idempotent: false` handlers, so they stay out.
/// `x-atlas-csrf` is read by CSRF middleware, not a handler, and already
/// gates the request before this one runs. `accept`/`accept-language` are
/// deliberately excluded: they shape the response's representation, not the
/// request's side effect (Stripe's own fingerprint compares parameters only).
const FINGERPRINT_HEADER_ALLOWLIST: &[&str] = &["content-type", "x-create-token"];

/// Response-header allowlist for storage and replay (D5). A `set-cookie` or
/// any other header minted during the *original* execution must never be
/// stored or replayed on a different connection/session state.
const RESPONSE_HEADER_ALLOWLIST: &[&str] = &["content-type", "location", "etag"];

/// Upper bound on the request/response bytes this middleware buffers to
/// compute the fingerprint / store the response. There is no separate
/// body-limit layer ahead of this middleware to rely on: the route's own
/// `DefaultBodyLimit` (or axum's 2 MiB default when a route sets none) is
/// applied by this middleware itself, via `RequestExt::with_limited_body()`,
/// immediately before buffering (R1 fix) — so the effective cap for any
/// given route is `min(route's DefaultBodyLimit, MAX_BUFFERED_BODY_BYTES)`.
/// This constant only bounds what the idempotency mechanism itself is
/// willing to buffer/store, well above ordinary JSON payload sizes; it is
/// never a wider limit than the route already enforces.
///
/// The two directions this cap is enforced on have different consequences
/// (`R2-response-buffer-silently-emptied` fix, ORCHESTRATOR RULING
/// 2026-09-02): a *request* over the route's own limit answers 413 and
/// writes no row (nothing was claimed yet). A *response* over this ceiling,
/// or any other body-stream failure while buffering it, can only be
/// detected after the handler already ran and produced a real body this
/// middleware failed to read back — that body must never be silently
/// replaced with an empty one and returned/stored as if it were the
/// handler's actual output. Instead the claim is released so a retry
/// re-executes the handler, and this caller gets a 500: the real response
/// is unrecoverable at that point, and an honest 500 beats a fabricated
/// empty 2xx.
const MAX_BUFFERED_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Computes the D4/D5 fingerprint (R4 fix): `method || "\n" ||
/// concrete_path_and_query || "\n" || sorted(allowlisted request headers,
/// lowercased names) || "\n" || raw body bytes`. The query string is part of
/// the fingerprint because it changes what the same body means (e.g. a
/// filter or target parameter); a header outside
/// [`FINGERPRINT_HEADER_ALLOWLIST`] never affects the hash regardless of
/// case.
fn compute_fingerprint(
    method: &str,
    path_and_query: &str,
    headers: &HeaderMap,
    body: &[u8],
) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update(b"\n");
    hasher.update(path_and_query.as_bytes());
    hasher.update(b"\n");

    let mut included: Vec<(String, &str)> = headers
        .iter()
        .filter_map(|(name, value)| {
            let name = name.as_str();
            if !FINGERPRINT_HEADER_ALLOWLIST.contains(&name) {
                return None;
            }
            value
                .to_str()
                .ok()
                .map(|value| (name.to_lowercase(), value))
        })
        .collect();
    included.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

    for (name, value) in included {
        hasher.update(name.as_bytes());
        hasher.update(b":");
        hasher.update(value.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(b"\n");
    hasher.update(body);

    hasher.finalize().to_vec()
}

/// Filters a response's headers down to the D5 storage/replay allowlist,
/// serialized as a small JSON object (`{"content-type": "...", ...}`) for
/// the repo's `response_headers jsonb` column. Returns `None` when no
/// allowlisted header is present.
fn filter_response_headers(headers: &HeaderMap) -> Option<serde_json::Value> {
    let mut map = serde_json::Map::new();

    for name in RESPONSE_HEADER_ALLOWLIST {
        if let Some(value) = headers.get(*name).and_then(|value| value.to_str().ok()) {
            map.insert(
                (*name).to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    if map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(map))
    }
}

/// Applies a stored `response_headers` JSON object back onto a response,
/// keeping only names this middleware itself allowlists on write — a
/// defense-in-depth check, not just trust in what was previously stored.
fn apply_stored_headers(response: &mut Response, headers: &Option<serde_json::Value>) {
    let Some(serde_json::Value::Object(map)) = headers else {
        return;
    };

    for name in RESPONSE_HEADER_ALLOWLIST {
        let Some(value) = map.get(*name).and_then(|value| value.as_str()) else {
            continue;
        };
        let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        response.headers_mut().insert(header_name, header_value);
    }
}

fn mark_replayed(mut response: Response) -> Response {
    response.headers_mut().insert(
        HeaderName::from_static(IDEMPOTENT_REPLAYED_HEADER),
        HeaderValue::from_static("true"),
    );
    response
}

fn fingerprint_hint(fingerprint: &[u8]) -> String {
    fingerprint
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn principal_id(principal: &Principal) -> uuid::Uuid {
    match principal {
        Principal::User(id) => id.0,
        Principal::ApiKey(id) => id.0,
    }
}

/// Entry point for an ordinary `idempotent: true` route (D6 scoped
/// correction): a 5xx releases the `in_flight` row, so the next attempt
/// re-executes the handler as `Fresh`. See the module doc for the full
/// [`OnServerError`] rationale.
pub async fn idempotency_middleware_release(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    idempotency_middleware(
        state,
        request,
        next,
        IdempotencyPolicy {
            on_server_error: OnServerError::Release,
        },
    )
    .await
}

/// Entry point for a `component_routes!` `one_shot` route (D6 scoped
/// correction, `router_audit::ONE_SHOT_IDEMPOTENT_ROUTES`): a 5xx is stored briefly
/// and replayed within [`FAILURE_RETENTION`] instead of released. See the
/// module doc for the full [`OnServerError`] rationale.
pub async fn idempotency_middleware_store_briefly(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    idempotency_middleware(
        state,
        request,
        next,
        IdempotencyPolicy {
            on_server_error: OnServerError::StoreBriefly,
        },
    )
    .await
}

/// The `Idempotency-Key` middleware (D2/D6). Mounted innermost on a
/// declared-`idempotent: true` route only, via
/// [`idempotency_middleware_release`] or [`idempotency_middleware_store_briefly`]
/// — see the module doc for the pass-through cases (no header, no
/// principal) and for `policy`'s effect on a 5xx.
async fn idempotency_middleware(
    state: AppState,
    request: Request,
    next: Next,
    policy: IdempotencyPolicy,
) -> Result<Response, ApiError> {
    let Some(key) = request
        .headers()
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
    else {
        return Ok(next.run(request).await);
    };

    let Some(principal_id) = request.extensions().get::<Principal>().map(principal_id) else {
        // No authenticated principal at this point: either `require_authn`
        // has not run for this route (login/activate/webhook-ingest, all
        // classified `idempotent: false` under D8 for exactly this reason)
        // or a route was misconfigured. Either way there is no
        // `principal_id` to scope dedup by, so this middleware never stores
        // anything and never blocks the request.
        return Ok(next.run(request).await);
    };

    let method = request.method().to_string();

    // Component routers are nested under their namespace, and axum strips
    // the nest prefix from `request.uri()` before a nested route's layers
    // run. The store key and the fingerprint must carry the path the client
    // sent, so both read `OriginalUri`, which the root router records before
    // any nesting.
    let original_uri = request
        .extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|original| original.0.clone())
        .unwrap_or_else(|| request.uri().clone());
    let path = original_uri.path().to_string();
    let path_and_query = original_uri
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| path.clone());

    // Apply the route's own `DefaultBodyLimit` (falling back to axum's 2 MiB
    // default when the route sets none) BEFORE buffering, so a body over the
    // route's own limit gets the same 413 its handler's extractor would have
    // produced — this middleware's `MAX_BUFFERED_BODY_BYTES` is only a
    // ceiling for what it itself is willing to store, never a wider cap than
    // the route already enforces.
    let (parts, limited_body) = request.with_limited_body().into_parts();
    let body_bytes = match axum::body::to_bytes(limited_body, MAX_BUFFERED_BODY_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err(ApiError::PayloadTooLarge {
                message: "Request body exceeds the route's configured body limit.".to_string(),
            });
        }
    };

    let fingerprint = compute_fingerprint(&method, &path_and_query, &parts.headers, &body_bytes);

    let repo = PgIdempotencyRepo {
        conn: (*state.db).clone(),
    };
    let scope = IdempotencyScope {
        principal_id,
        method,
        path,
        key,
    };
    let now = Utc::now();
    let retention = Duration::hours(state.idempotency_retention_hours);

    let outcome = match repo
        .insert_in_flight(&scope, &fingerprint, now, retention)
        .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            // R4 fix (`R4-store-write-is-a-hard-precondition`, ORCHESTRATOR
            // RULING 2026-09-02): a store outage before the handler ever
            // runs must not turn a store dependency into a route
            // dependency. Degrade to no-dedup instead of a 5xx — no row is
            // written, no replay is possible — and mark the response so a
            // strict client can tell the request ran without the guarantee.
            tracing::error!(
                ?err,
                principal_id = %scope.principal_id,
                method = %scope.method,
                path = %scope.path,
                key = %scope.key,
                "idempotency insert_in_flight failed; serving without dedup \
                 (Idempotency-Degraded: store-unavailable)"
            );
            let request = Request::from_parts(parts, Body::from(body_bytes));
            let mut response = next.run(request).await;
            response.headers_mut().insert(
                HeaderName::from_static(IDEMPOTENCY_DEGRADED_HEADER),
                HeaderValue::from_static(IDEMPOTENCY_DEGRADED_STORE_UNAVAILABLE),
            );
            return Ok(response);
        }
    };

    match outcome {
        InsertOutcome::InFlight => Err(ApiError::IdempotencyKeyInFlight),
        InsertOutcome::Mismatch {
            existing_fingerprint,
        } => Err(ApiError::IdempotencyKeyConflict {
            existing_fingerprint_hint: fingerprint_hint(&existing_fingerprint),
        }),
        InsertOutcome::Replay(stored) => {
            let status = StatusCode::from_u16(u16::try_from(stored.status).unwrap_or(200))
                .unwrap_or(StatusCode::OK);
            let mut response = (status, stored.body).into_response();
            apply_stored_headers(&mut response, &stored.headers);
            Ok(mark_replayed(response))
        }
        InsertOutcome::Fresh { id, generation } => {
            let request = Request::from_parts(parts, Body::from(body_bytes));
            let scope_for_task = scope.clone();

            // R4 fix (`R4-inflight-residue-blocks-then-duplicates`,
            // ORCHESTRATOR RULING 2026-09-02): once this caller has claimed
            // the row (`Fresh`), running the handler and writing
            // `complete()` must not be cancellable by the client dropping
            // its connection (a client-side timeout is exactly why a caller
            // sends an `Idempotency-Key` in the first place). `tokio::spawn`
            // detaches both onto a task the client's disconnect cannot
            // cancel; `next`, `request`, `repo`, and `scope_for_task` are
            // all owned by the task, so it needs nothing from this
            // request's own future once spawned. `next.run` is `Send +
            // 'static` (`Next` wraps an owned `BoxCloneSyncService`), so
            // this is spawnable without restructuring `Next` itself.
            let task = tokio::spawn(async move {
                let response = next.run(request).await;

                let (resp_parts, resp_body) = response.into_parts();
                let resp_bytes =
                    match axum::body::to_bytes(resp_body, MAX_BUFFERED_BODY_BYTES).await {
                        Ok(bytes) => bytes,
                        Err(err) => {
                            // R2 fix (`R2-response-buffer-silently-emptied`,
                            // ORCHESTRATOR RULING 2026-09-02): the handler's
                            // real response body is unrecoverable here — either
                            // it exceeded `MAX_BUFFERED_BODY_BYTES` or the body
                            // stream itself failed. Never fabricate an empty
                            // body and return/store it as if it were the
                            // handler's real output. Release the claim so a
                            // retry re-executes the handler, and fail honestly.
                            tracing::error!(
                                ?err,
                                principal_id = %scope_for_task.principal_id,
                                method = %scope_for_task.method,
                                path = %scope_for_task.path,
                                key = %scope_for_task.key,
                                status = %resp_parts.status,
                                "idempotency response buffering failed; releasing claim \
                                 so a retry re-executes the handler"
                            );

                            if let Err(release_err) = repo.release(id, generation).await {
                                tracing::warn!(
                                    ?release_err,
                                    principal_id = %scope_for_task.principal_id,
                                    method = %scope_for_task.method,
                                    path = %scope_for_task.path,
                                    key = %scope_for_task.key,
                                    "idempotency release() failed after a response \
                                     buffering error; the row stays in_flight until \
                                     staleness, same degraded mode as a swallowed \
                                     complete() failure"
                                );
                            }

                            return Err(ApiError::Internal {
                                message: "idempotency response buffering failed".to_string(),
                            });
                        }
                    };

                // The allowlist is applied BEFORE `complete()` is ever
                // called — a non-allowlisted header (e.g. `set-cookie`)
                // never reaches the store, so it can neither be stored nor
                // replayed.
                let allowlisted_headers = filter_response_headers(&resp_parts.headers);
                let response_status = i16::try_from(resp_parts.status.as_u16()).unwrap_or(i16::MAX);

                // D6 scoped correction (`R4-5xx-release-duplicates-one-shot-jobs`):
                // a 5xx on a `Release`-policy route (every ordinary create)
                // discards the row outright instead of storing the response,
                // so the next attempt re-executes as `Fresh` and the
                // handler's own domain check catches a genuine duplicate.
                // Every other case (a 2xx/4xx on any policy, or a 5xx on the
                // `StoreBriefly` policy) still goes through `complete()`.
                if resp_parts.status.is_server_error()
                    && policy.on_server_error == OnServerError::Release
                {
                    if let Err(err) = repo.release(id, generation).await {
                        tracing::warn!(
                            ?err,
                            principal_id = %scope_for_task.principal_id,
                            method = %scope_for_task.method,
                            path = %scope_for_task.path,
                            key = %scope_for_task.key,
                            "idempotency release() failed after a 5xx; the row stays \
                             in_flight until staleness, same degraded mode as a \
                             swallowed complete() failure"
                        );
                    }

                    return Ok(Response::from_parts(resp_parts, Body::from(resp_bytes)));
                }

                // A 5xx on the `StoreBriefly` policy is still `complete()`d
                // and stored so an immediate retry replays the same response
                // instead of re-executing a one-shot handler (`purge_trash`,
                // `semantic_reindex_start`, ...), but only for
                // `FAILURE_RETENTION` rather than the normal retention, so
                // the replay window is short and a later retry re-executes
                // as `Fresh` once it lapses.
                let retention_for_status = if resp_parts.status.is_server_error() {
                    FAILURE_RETENTION
                } else {
                    retention
                };

                // R4 fix (`R4-inflight-residue-blocks-then-duplicates`): a
                // single retry after yielding once absorbs a transient
                // store hiccup (a momentarily saturated pool, a brief
                // network blip) instead of giving up on the very first
                // failure. This is one bounded retry, not a loop — a second
                // failure falls straight through to the documented
                // degraded mode below.
                let mut complete_result = repo
                    .complete(
                        id,
                        generation,
                        response_status,
                        &resp_bytes,
                        allowlisted_headers.clone(),
                        Utc::now(),
                        retention_for_status,
                    )
                    .await;

                if complete_result.is_err() {
                    tokio::task::yield_now().await;
                    complete_result = repo
                        .complete(
                            id,
                            generation,
                            response_status,
                            &resp_bytes,
                            allowlisted_headers,
                            Utc::now(),
                            retention_for_status,
                        )
                        .await;
                }

                // Regardless of `Stored`/`Superseded`, THIS caller's
                // response is always its own handler's real response —
                // `Superseded` only means a later caller's reclaim now owns
                // the row, so this response is never treated as replayable
                // for anyone else.
                //
                // A store error here — on both attempts — must never mask
                // the handler's already-committed write behind a 500; fix
                // forward by logging and returning that response to THIS
                // caller unmarked (no `Idempotent-Replayed`), exactly as for
                // `Superseded`. The row stays `in_flight`: a retry within
                // the staleness window answers 409 and after it
                // re-executes; for one-shot handlers that can duplicate a
                // side effect.
                if let Err(err) = complete_result {
                    tracing::error!(
                        ?err,
                        principal_id = %scope_for_task.principal_id,
                        method = %scope_for_task.method,
                        path = %scope_for_task.path,
                        key = %scope_for_task.key,
                        "idempotency complete() failed twice after the handler ran; \
                         the row stays in_flight; a retry within the staleness window \
                         answers 409 and after it re-executes; for one-shot handlers \
                         that can duplicate a side effect"
                    );
                }

                Ok(Response::from_parts(resp_parts, Body::from(resp_bytes)))
            });

            match task.await {
                Ok(Ok(response)) => Ok(response),
                Ok(Err(api_err)) => Err(api_err),
                Err(join_err) => {
                    // The handler panicked before `complete()` ever ran.
                    // The row is left exactly where a swallowed `complete()`
                    // error would leave it: `in_flight` until staleness, so
                    // treat this the same way — log and answer this caller
                    // with a 500, never a stored/replayable response.
                    tracing::error!(
                        ?join_err,
                        principal_id = %scope.principal_id,
                        method = %scope.method,
                        path = %scope.path,
                        key = %scope.key,
                        "idempotency handler task panicked; the row stays in_flight, \
                         same degraded mode as a swallowed complete() failure"
                    );
                    Err(ApiError::Internal {
                        message: "idempotency handler task panicked".to_string(),
                    })
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).expect("valid header name"),
                HeaderValue::from_str(value).expect("valid header value"),
            );
        }
        headers
    }

    #[test]
    fn fingerprint_is_stable_for_identical_inputs() {
        let headers = headers_with(&[("content-type", "application/json"), ("accept", "*/*")]);
        let a = compute_fingerprint("POST", "/api/workspaces/w/tasks", &headers, b"{}");
        let b = compute_fingerprint("POST", "/api/workspaces/w/tasks", &headers, b"{}");
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_changes_with_body() {
        let headers = headers_with(&[("content-type", "application/json")]);
        let a = compute_fingerprint("POST", "/p", &headers, b"{\"a\":1}");
        let b = compute_fingerprint("POST", "/p", &headers, b"{\"a\":2}");
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_changes_with_method_or_path() {
        let headers = HeaderMap::new();
        let a = compute_fingerprint("POST", "/a", &headers, b"{}");
        let b = compute_fingerprint("PATCH", "/a", &headers, b"{}");
        let c = compute_fingerprint("POST", "/b", &headers, b"{}");
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    /// R4 fix (`R4-fingerprint-denylist-breaks-real-retries`): a proxy- or
    /// tracing-injected header that legitimately varies per attempt
    /// (`x-b3-traceid`, `x-amzn-trace-id`, `date`), plus a representative
    /// sample of the previously denylisted headers (`user-agent`,
    /// `authorization`, `idempotency-key`, `x-request-id`, `traceparent`),
    /// must NOT change the fingerprint — none of them is in
    /// [`FINGERPRINT_HEADER_ALLOWLIST`], so a byte-identical retry through a
    /// different proxy hop still replays instead of 409ing.
    #[test]
    fn fingerprint_ignores_headers_outside_the_allowlist() {
        let base = headers_with(&[("content-type", "application/json")]);
        let with_noise = headers_with(&[
            ("content-type", "application/json"),
            ("x-b3-traceid", "80f198ee5634"),
            ("x-amzn-trace-id", "Root=1-5e1b4151"),
            ("date", "Tue, 01 Sep 2026 12:00:00 GMT"),
            ("user-agent", "test-agent/1.0"),
            ("authorization", "Bearer secret-token"),
            ("idempotency-key", "key-1"),
            ("x-request-id", "req-1"),
            ("traceparent", "00-abc-def-01"),
        ]);

        let a = compute_fingerprint("POST", "/p", &base, b"{}");
        let b = compute_fingerprint("POST", "/p", &with_noise, b"{}");
        assert_eq!(
            a, b,
            "a header outside the allowlist must never affect the fingerprint"
        );
    }

    /// R4 fix: `x-create-token` is on [`FINGERPRINT_HEADER_ALLOWLIST`]
    /// because `tasks::create_comment_draft` and
    /// `documents::create_comment_draft` (both `idempotent: true`) read it as
    /// the comment-draft's own replay token, so reusing an `Idempotency-Key`
    /// with a different `x-create-token` DOES change the fingerprint.
    #[test]
    fn fingerprint_changes_with_allowlisted_custom_header() {
        let a = headers_with(&[("content-type", "application/json")]);
        let b = headers_with(&[
            ("content-type", "application/json"),
            ("x-create-token", "3f3a6a9e-8f0e-4b8a-9f0e-4b8a9f0e4b8a"),
        ]);

        assert_ne!(
            compute_fingerprint("POST", "/p", &a, b"{}"),
            compute_fingerprint("POST", "/p", &b, b"{}"),
            "an allowlisted custom header must change the fingerprint"
        );
    }

    /// A header outside the allowlist (e.g. `x-file-name`, read only by the
    /// `idempotent: false` attachment-upload handlers) must NOT change the
    /// fingerprint — it is not on [`FINGERPRINT_HEADER_ALLOWLIST`].
    #[test]
    fn fingerprint_ignores_a_non_allowlisted_custom_header() {
        let a = headers_with(&[("content-type", "application/json")]);
        let b = headers_with(&[
            ("content-type", "application/json"),
            ("x-file-name", "report.pdf"),
        ]);

        assert_eq!(
            compute_fingerprint("POST", "/p", &a, b"{}"),
            compute_fingerprint("POST", "/p", &b, b"{}"),
            "a non-allowlisted custom header must not change the fingerprint"
        );
    }

    /// R3 fix: the fingerprint is computed over `path_and_query`, so the
    /// same path with a different query string is a different fingerprint.
    #[test]
    fn fingerprint_changes_with_query_string() {
        let headers = HeaderMap::new();
        let a = compute_fingerprint("POST", "/p?scope=a", &headers, b"{}");
        let b = compute_fingerprint("POST", "/p?scope=b", &headers, b"{}");
        assert_ne!(a, b, "a different query string must change the fingerprint");
    }

    #[test]
    fn fingerprint_header_order_does_not_matter() {
        let a = headers_with(&[("accept", "*/*"), ("content-type", "application/json")]);
        let b = headers_with(&[("content-type", "application/json"), ("accept", "*/*")]);
        assert_eq!(
            compute_fingerprint("POST", "/p", &a, b"{}"),
            compute_fingerprint("POST", "/p", &b, b"{}"),
            "headers are sorted before hashing, so insertion order must not matter"
        );
    }

    /// D5: `set-cookie` must never be captured by the response-header
    /// allowlist — the direct unit-level proof for the store-time filter,
    /// independent of the live-sweep integration test.
    #[test]
    fn response_header_allowlist_excludes_set_cookie() {
        let headers = headers_with(&[
            ("content-type", "application/json"),
            ("set-cookie", "atlas_session=abc; HttpOnly"),
            ("location", "/api/workspaces/w/tasks/T-1"),
            ("etag", "\"abc123\""),
            ("x-request-id", "req-1"),
        ]);

        let filtered = filter_response_headers(&headers).expect("allowlisted headers present");
        let serde_json::Value::Object(map) = filtered else {
            panic!("expected a JSON object");
        };

        assert!(
            !map.contains_key("set-cookie"),
            "set-cookie must be excluded"
        );
        assert!(
            !map.contains_key("x-request-id"),
            "x-request-id must be excluded"
        );
        assert_eq!(
            map.get("content-type").and_then(|v| v.as_str()),
            Some("application/json")
        );
        assert_eq!(
            map.get("location").and_then(|v| v.as_str()),
            Some("/api/workspaces/w/tasks/T-1")
        );
        assert_eq!(map.get("etag").and_then(|v| v.as_str()), Some("\"abc123\""));
    }

    /// D5: replay never re-applies a `set-cookie` even if one somehow ended
    /// up in the stored JSON (defense in depth against the write-side
    /// allowlist alone) — `apply_stored_headers` re-checks the allowlist on
    /// read too.
    #[test]
    fn applying_stored_headers_never_reintroduces_set_cookie() {
        let stored = Some(serde_json::json!({
            "content-type": "application/json",
            "set-cookie": "atlas_session=stale; HttpOnly",
        }));

        let mut response = (StatusCode::OK, "ok").into_response();
        apply_stored_headers(&mut response, &stored);

        assert!(response.headers().get("set-cookie").is_none());
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("application/json")
        );
    }

    #[test]
    fn fingerprint_hint_is_a_short_hex_prefix() {
        let hint = fingerprint_hint(&[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(hint, "deadbeef");
    }
}
