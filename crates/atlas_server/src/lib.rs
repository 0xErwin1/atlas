#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

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
pub mod ops;
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
    // Each component's own router is built once and nested at its own
    // `/api/v2/<component>` mount (`v2-e3-s4` PR7, D10: SHELL-API-1 and the
    // E3 proposal fix the V2 URL with the component segment, correcting
    // PR5's flat, shared `/api/v2` mount). Since `v2-e3-s7` this is the
    // ONLY mount a component's routes are reachable at: the shared `/api`
    // nest that used to also serve every route (S1's original mount) was
    // retired once every consumer this repository owns migrated to the V2
    // form. The root-level routes (`/health`, `/ready`, `/version`,
    // `/openapi.json`, `/scalar`) live in the separately built
    // `root_router`, outside every nest.
    //
    // Unmatched paths run the protected stack before answering 404, so an
    // unauthenticated probe of a path that does not exist gets 401 and only
    // an authenticated one gets 404. `Router::nest` drops a nested router's
    // default fallback, so the fallback is declared explicitly at the root
    // and merged last: merging a router with a custom fallback into one with
    // the default fallback adopts the custom one, covering every nest.
    let platform_router = routes::platform::router(state.clone());
    let custos_router = routes::custos::router(state.clone());
    let acta_router = routes::acta::router(state.clone());

    let root_router =
        routes::platform::root_router(state.clone()).merge(routes::acta::root_router());

    let fallback_router = routes::protection::protect(Router::new().fallback(not_found), state);

    let router = Router::new()
        .nest("/api/v2/platform", platform_router)
        .nest("/api/v2/custos", custos_router)
        .nest("/api/v2/acta", acta_router)
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
