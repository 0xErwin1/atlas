use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

use atlas_api::dtos::{NotReadyComponentDto, ReadinessReportDto, ServerMetaDto};
use atlas_core::ops::readiness::{ReadinessReport, aggregate_readiness};

use crate::ops::deadline::TokioDeadline;
use crate::{error::ApiError, state::AppState};

#[utoipa::path(
    get,
    path = "/health",
    responses((status = 200, description = "Service is healthy"))
)]
pub(crate) async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

/// Readiness probe: aggregates every mandatory component's own `Readiness`
/// result (`Registry::readiness_components()`, today `platform`, `custos`,
/// `acta`) through `aggregate_readiness`, each bounded by
/// `AppState::readiness_timeout` (default
/// [`crate::state::DEFAULT_READINESS_TIMEOUT`]; design D3, SHELL-OPS-2).
/// The mandatory set itself was captured once at startup by
/// `DiagnosticsRegistry::bind`, so no registry is rebuilt per request.
///
/// Unlike `/health` (a cheap liveness signal), this endpoint performs one
/// bounded probe per mandatory component. Answers 503 naming every
/// not-ready component with its own reason (SH2), never just the first.
#[utoipa::path(
    get,
    path = "/ready",
    responses(
        (status = 200, description = "Service is ready: every mandatory component is ready", body = ReadinessReportDto),
        (status = 503, description = "Service is not ready: at least one mandatory component is not ready", body = ReadinessReportDto),
    )
)]
pub(crate) async fn ready(State(state): State<AppState>) -> Response {
    let set = match state.diagnostics.readiness_set() {
        Ok(set) => set,
        Err(component) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadinessReportDto {
                    ready: false,
                    not_ready: vec![NotReadyComponentDto {
                        component: component.as_str().to_string(),
                        reason: "no readiness implementer is bound".to_string(),
                    }],
                }),
            )
                .into_response();
        }
    };

    let deadline = TokioDeadline {
        per_component: state.readiness_timeout,
    };
    let report = aggregate_readiness(&set, &deadline).await;

    match report {
        ReadinessReport::Ready => (
            StatusCode::OK,
            Json(ReadinessReportDto {
                ready: true,
                not_ready: vec![],
            }),
        )
            .into_response(),
        ReadinessReport::NotReady { not_ready } => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessReportDto {
                ready: false,
                not_ready: not_ready
                    .into_iter()
                    .map(|component| NotReadyComponentDto {
                        component: component.component.as_str().to_string(),
                        reason: component.reason,
                    })
                    .collect(),
            }),
        )
            .into_response(),
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
