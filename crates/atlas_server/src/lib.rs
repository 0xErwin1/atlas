#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use axum::{Router, middleware as axum_middleware, routing::get};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub mod auth;
pub mod config;
pub mod error;
pub mod middleware;
pub mod persistence;
mod routes;

/// Builds the full application router with the complete middleware stack.
pub fn app() -> Router {
    let router = Router::new()
        .route("/health", get(routes::health::health))
        .route("/version", get(routes::health::version));

    apply_layers(router)
}

/// Wraps `router` with the standard request-id / trace / problem-stamp layer stack.
fn apply_layers(router: Router) -> Router {
    router
        .layer(axum_middleware::from_fn(
            crate::middleware::problem_stamp::problem_stamp,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}

/// Test helper: builds a minimal app with a single route and the full middleware stack.
///
/// Used by `tests/error_model.rs` to exercise the problem-stamp middleware without
/// starting a real server.
pub fn test_app_with_route(path: &str, handler: axum::routing::MethodRouter) -> Router {
    apply_layers(Router::new().route(path, handler))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_returns_200() {
        let app = app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
