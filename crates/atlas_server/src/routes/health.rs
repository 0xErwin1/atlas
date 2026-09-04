use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde_json::json;

use atlas_api::dtos::ServerMetaDto;

use crate::{error::ApiError, state::AppState};

#[utoipa::path(
    get,
    path = "/health",
    responses((status = 200, description = "Service is healthy"))
)]
pub(crate) async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

/// Readiness probe: liveness plus a `SELECT 1` round-trip to the database.
///
/// Unlike `/health` (a cheap liveness signal), this endpoint touches the pool so
/// an orchestrator can withhold traffic while the database is unreachable or the
/// pool is exhausted. Returns 503 on any database error.
#[utoipa::path(
    get,
    path = "/ready",
    responses(
        (status = 200, description = "Service is ready: the database is reachable"),
        (status = 503, description = "Service is not ready: the database is unreachable"),
    )
)]
pub(crate) async fn ready(State(state): State<AppState>) -> Response {
    let probe = state
        .db
        .execute_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT 1",
        ))
        .await;

    match probe {
        Ok(_) => (StatusCode::OK, Json(json!({"status": "ready"}))).into_response(),
        Err(e) => {
            tracing::warn!(
                target: "health.readiness",
                event = "readiness_failed",
                error = %e,
                "readiness probe failed: database unreachable"
            );
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"status": "unavailable"})),
            )
                .into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/version",
    responses((status = 200, description = "Service version"))
)]
pub(crate) async fn version() -> impl IntoResponse {
    Json(json!({"version": env!("CARGO_PKG_VERSION")}))
}

#[utoipa::path(
    get,
    path = "/meta",
    tag = "meta",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Server build information", body = ServerMetaDto),
        (status = 401, description = "Unauthenticated"),
    )
)]
pub(crate) async fn meta(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let semantic_search_enabled =
        state
            .semantic_search_enabled_now()
            .await
            .map_err(|error| ApiError::Internal {
                message: format!("semantic search schema readiness check failed: {error}"),
            })?;

    Ok(Json(ServerMetaDto {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build: state.build.clone(),
        url: state.server_url.clone(),
        max_attachment_bytes: Some(state.max_attachment_bytes),
        semantic_search_enabled: Some(semantic_search_enabled),
    }))
}
