#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use axum::{Router, middleware as axum_middleware, routing::get};
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub mod auth;
pub mod authz;
pub mod config;
pub mod error;
pub mod middleware;
pub mod persistence;
mod routes;
pub mod state;

use crate::state::AppState;

/// Builds the full application router with all routes and the middleware stack.
pub fn app(state: AppState) -> Router {
    // burst_size(5) and per_second(1) are non-zero, so finish() always returns Some here.
    #[allow(clippy::expect_used)]
    let login_config = {
        let mut b = GovernorConfigBuilder::default();
        let cfg = b
            .per_second(1)
            .burst_size(5)
            .finish()
            .expect("governor config");
        std::sync::Arc::new(cfg)
    };

    let protected = Router::new()
        .route("/v1/auth/logout", axum::routing::post(routes::auth::logout))
        .route("/v1/auth/me", get(routes::auth::me))
        .route("/v1/workspaces/{ws}/probe", get(routes::probe::probe))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            crate::auth::middleware::require_authn,
        ))
        .with_state(state.clone());

    let public = Router::new()
        .route("/health", get(routes::health::health))
        .route("/version", get(routes::health::version))
        .route(
            "/v1/auth/login",
            axum::routing::post(routes::auth::login).layer(GovernorLayer::new(login_config)),
        )
        .with_state(state.clone());

    let router = public.merge(protected);
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
