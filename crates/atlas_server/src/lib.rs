#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use axum::{Router, middleware as axum_middleware};
use tower_http::{
    classify::ServerErrorsFailureClass,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub mod auth;
pub mod authz;
pub mod config;
pub mod crypto;
pub mod dispatcher;
pub mod embeddings;
pub mod error;
pub mod hybrid_search;
pub mod live;
pub mod middleware;
pub mod persistence;
pub mod platform;
pub mod presence;
pub mod reg5;
pub mod router_audit;
pub mod routes;
pub mod search_indexer;
pub mod semantic_indexer;
pub mod services;
pub mod startup;
pub mod state;
pub mod task_graph;
pub mod webhook_url;

/// Test-only server assembly for the desktop integration gate.
#[cfg(feature = "desktop-gate-support")]
pub mod desktop_gate_support {
    use crate::persistence::repos::{NewUser, SessionRepo, UserRepo};
    use atlas_acta::actor::Actor;
    use atlas_acta::actor::WorkspaceCtx;
    use atlas_acta::entities::identity::MemberRole;
    use atlas_acta::ids::WorkspaceId;
    use atlas_acta_postgres::repos::identity::{
        MembershipRepo, NewWorkspace, PgMembershipRepo, PgWorkspaceRepo, WorkspaceRepo,
    };
    use atlas_core::principal::UserId;
    use atlas_custos_postgres::repos::identity::{PgSessionRepo, PgUserRepo};
    use sea_orm::{ConnectionTrait, DatabaseConnection};

    pub use crate::app;
    pub use crate::state::AppState;

    /// Builds the deterministic server state used by the desktop gate.
    pub async fn app_state(db: DatabaseConnection) -> Result<AppState, anyhow::Error> {
        AppState::for_test(db).await
    }

    /// Identity material generated only for the in-process desktop gate server.
    pub struct EphemeralIdentity {
        user_id: UserId,
        pub username: String,
        pub password: String,
        pub workspace_slug: String,
        pub workspace_id: uuid::Uuid,
    }

    /// Creates an activated workspace owner with credentials confined to the caller.
    pub async fn seed_ephemeral_identity(
        db: &DatabaseConnection,
    ) -> Result<EphemeralIdentity, anyhow::Error> {
        let suffix = uuid::Uuid::now_v7().as_simple().to_string();
        let username = format!("gate-{suffix}");
        let password = format!("Gate{suffix}Aa!");
        let password_hash = crate::auth::password::hash(password.clone())
            .await
            .map_err(|_| anyhow::anyhow!("gate identity password setup failed"))?;

        let user = PgUserRepo { conn: db.clone() }
            .create(NewUser {
                username: username.clone(),
                display_name: username.clone(),
                email: None,
                password_hash: Some(password_hash),
                is_root: false,
                is_system_admin: false,
            })
            .await?;

        db.execute_unprepared(&format!(
            "UPDATE custos.users SET activated_at = now() WHERE id = '{}'",
            user.id.0
        ))
        .await?;

        let workspace_slug = format!("ws-{suffix}");
        let workspace = PgWorkspaceRepo { conn: db.clone() }
            .create(NewWorkspace {
                id: WorkspaceId::new(),
                name: format!("Workspace {suffix}"),
                slug: workspace_slug.clone(),
            })
            .await?;
        let ctx = WorkspaceCtx::new(
            workspace.id,
            Actor::User(atlas_acta::actor::UserAttributionId(user.id.0)),
        );

        PgMembershipRepo { conn: db.clone() }
            .add(&ctx, user.id, MemberRole::Owner)
            .await?;

        Ok(EphemeralIdentity {
            user_id: user.id,
            username,
            password,
            workspace_slug,
            workspace_id: workspace.id.0,
        })
    }

    /// Revokes only the generated gate user's active sessions through the test-only server seam.
    pub async fn revoke_ephemeral_sessions(
        db: &DatabaseConnection,
        identity: &EphemeralIdentity,
    ) -> Result<(), anyhow::Error> {
        PgSessionRepo { conn: db.clone() }
            .revoke_all_for_user(identity.user_id)
            .await?;
        Ok(())
    }
}

use crate::state::AppState;

/// Builds the full application router with all routes and the middleware stack.
pub fn app(state: AppState) -> Router {
    // `platform`'s eight routes, `custos`'s 35 routes, and `acta`'s 169 routes
    // are each assembled by `routes::platform::router` (PR2),
    // `routes::custos::router` (PR3), and `routes::acta::router` (PR4),
    // every one of which reproduces the exact public/protected split and
    // layer stack (including custos's login/activate governors and acta's
    // seven `DefaultBodyLimit`-layered routes plus its own governed ingest
    // route) they carried inline here before their respective conversions —
    // merging an already-layered router is the same operation as merging
    // two already-layered routers (D6).
    //
    // `v2-e3-s4` PR4 (D2): every registry-declared route literal was
    // rewritten to namespace-relative form, so the `/api` prefix that used
    // to be baked into each literal is now applied exactly once, here, via
    // `Router::nest("/api", ...)` — the single mount D1 describes (PR5 adds
    // a second, `/api/v2`, nest at this same composition root). The five
    // routes that never carried an `/api` prefix (`/health`, `/ready`,
    // `/version`, `/openapi.json`, `/scalar`) are NOT part of the nested
    // router: each component now exposes a `root_router()` for exactly
    // those routes, built and merged separately, outside the `/api` nest, so
    // they keep being served at their bare root-level path.
    //
    // Unmatched paths run the protected stack before answering 404, so an
    // unauthenticated probe of any path that does not exist gets 401 and only
    // an authenticated one gets 404. Before the `/api` nest this happened by
    // accident: `Router::merge` keeps whichever side has a non-default
    // fallback, `Router::layer` also wraps the default fallback, and the last
    // merged component's protected layers therefore ended up wrapping the
    // root's default 404. `Router::nest` only carries the inner router's
    // fallback across when that fallback is not the default one, so the
    // nested `api_router` loses it and unmatched paths would fall through to
    // a bare 404. The fallback is now declared explicitly, at the root so
    // it covers every nest, and merged last: merging a router with a custom
    // fallback into one with the default fallback adopts the custom one.
    let api_router = routes::platform::router(state.clone())
        .merge(routes::custos::router(state.clone()))
        .merge(routes::acta::router(state.clone()));

    let root_router =
        routes::platform::root_router(state.clone()).merge(routes::acta::root_router());

    let fallback_router = routes::protection::protect(Router::new().fallback(not_found), state);

    let router = Router::new()
        .nest("/api", api_router)
        .merge(root_router)
        .merge(fallback_router);
    apply_layers(router)
}

/// Response body-less 404, identical to axum's default fallback, so the only
/// difference from the default is the protected stack `app()` wraps it in.
async fn not_found() -> axum::http::StatusCode {
    axum::http::StatusCode::NOT_FOUND
}

/// Wraps `router` with the standard request-id / trace / problem-stamp layer stack.
///
/// The trace layer opens one span per request carrying the method, URI, and the
/// `x-request-id` set by the outer request-id layer, so every log emitted while
/// handling a request is correlated by that id. Request start, completion (with
/// status and latency), and failures are logged at INFO/ERROR.
///
/// `/health`, `/ready`, and `/version` are intentionally excluded: they are polled
/// at high frequency by probes and carry no useful per-request signal. Their span
/// is disabled, and the lifecycle callbacks short-circuit on a disabled span so
/// nothing is logged for them (a failing readiness probe still logs from its own
/// handler).
fn apply_layers(router: Router) -> Router {
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(|request: &axum::http::Request<_>| {
            if matches!(request.uri().path(), "/health" | "/version" | "/ready") {
                return tracing::Span::none();
            }

            let request_id = request
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("-");

            tracing::info_span!(
                "http",
                method = %request.method(),
                uri = %request.uri(),
                request_id = %request_id,
            )
        })
        .on_request(|_request: &axum::http::Request<_>, span: &tracing::Span| {
            if span.is_disabled() {
                return;
            }
            tracing::info!("started processing request");
        })
        .on_response(
            |response: &axum::http::Response<_>,
             latency: std::time::Duration,
             span: &tracing::Span| {
                if span.is_disabled() {
                    return;
                }
                tracing::info!(
                    status = response.status().as_u16(),
                    latency = ?latency,
                    "finished processing request"
                );
            },
        )
        .on_failure(
            |error: ServerErrorsFailureClass,
             latency: std::time::Duration,
             span: &tracing::Span| {
                if span.is_disabled() {
                    return;
                }
                tracing::error!(error = %error, latency = ?latency, "request failed");
            },
        );

    router
        .layer(axum_middleware::from_fn(
            crate::middleware::problem_stamp::problem_stamp,
        ))
        .layer(trace_layer)
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
