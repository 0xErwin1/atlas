//! R2's runtime probe (E11-S7 design §0.1, D3.4): is `axum::extract::MatchedPath`
//! present in a request's extensions at the point a `.layer()` applied at the
//! outermost composition level — over a router already assembled with
//! `.nest()` and `.merge()` — builds its span?
//!
//! `apply_layers` (`lib.rs`) is private to `atlas_server`, and its current,
//! unmodified `make_span_with` closure does not read `MatchedPath` at all —
//! that capability is what PR3 adds. So this probe does not call
//! `apply_layers`; it reconstructs its exact structural shape (one outer
//! `.layer(TraceLayer::new_for_http().make_span_with(..))` over a router
//! built the same way `app()` composes its own: `.nest(..)` for a sub-path
//! plus `.merge(..)` for a root-level route, `lib.rs:172-177`) on this
//! repository's pinned axum/tower_http versions (`Cargo.lock`), and records
//! from inside that exact closure position whether `MatchedPath` was seen.
//!
//! **Outcome (T3.2, recorded in the PR3 body regardless of result)**: both
//! cases below passed on axum 0.8.9 / tower-http (workspace-pinned). A
//! nested route's `MatchedPath` carries its full template
//! (`/probe/items/{id}`), and an unmatched path carries none. This confirms
//! design §0.1's static claim, so PR3 proceeds with D2's chosen design —
//! tag at `make_span_with`, two-arm `info_span!` — and the named
//! `on_response` fallback is not taken.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Body,
    extract::MatchedPath,
    http::{Request, StatusCode},
    routing::get,
};
use tower::ServiceExt;
use tower_http::trace::TraceLayer;

async fn ok() -> StatusCode {
    StatusCode::OK
}

/// Builds a two-route router — one nested under `/probe`, one merged at the
/// root — and applies one outer `TraceLayer` at the same composition point
/// `apply_layers` occupies in `app()`: after every `.nest()`/`.merge()` call,
/// via a single trailing `.layer()`. The `make_span_with` closure records
/// whatever `MatchedPath` it observed for each request into `captured`,
/// rather than opening a real span field with it (that belongs to D2's
/// production change, not this probe).
fn probe_router(captured: Arc<Mutex<Vec<Option<String>>>>) -> Router {
    let nested = Router::new().route("/items/{id}", get(ok));
    let root = Router::new().route("/other", get(ok));

    let router = Router::new().nest("/probe", nested).merge(root);

    let trace_layer = {
        let captured = captured.clone();
        TraceLayer::new_for_http().make_span_with(move |request: &Request<Body>| {
            let matched = request
                .extensions()
                .get::<MatchedPath>()
                .map(|matched| matched.as_str().to_string());
            captured.lock().unwrap().push(matched);
            tracing::info_span!("probe")
        })
    };

    router.layer(trace_layer)
}

#[tokio::test]
async fn nested_route_matched_path_carries_full_template() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let app = probe_router(captured.clone());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/probe/items/42")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        captured.lock().unwrap().as_slice(),
        [Some("/probe/items/{id}".to_string())],
        "MatchedPath at the outer layer must carry the full nested template, not a relative fragment"
    );
}

#[tokio::test]
async fn unmatched_path_carries_no_matched_path() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let app = probe_router(captured.clone());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        captured.lock().unwrap().as_slice(),
        [None],
        "an unmatched path must carry no MatchedPath extension"
    );
}
