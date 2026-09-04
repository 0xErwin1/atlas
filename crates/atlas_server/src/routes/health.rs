use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

use atlas_api::dtos::{NotReadyComponentDto, ReadinessReportDto, ServerMetaDto, VersionDto};
use atlas_core::ops::readiness::{ReadinessReport, aggregate_readiness};

use crate::ops::deadline::TokioDeadline;
use crate::state::AppState;

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

/// The fields common to `/version` and `/api/v2/platform/meta` (design D4):
/// built through [`crate::ops::meta::shared_identity`] from the registry
/// `AppState` built once at startup, so the two payloads cannot drift apart
/// on the same process and no handler rebuilds the registry per request.
fn identity_fields(
    state: &AppState,
) -> (
    String,
    Option<String>,
    Vec<atlas_api::dtos::ComponentSummaryDto>,
) {
    crate::ops::meta::shared_identity(&state.registry, state.build.clone())
}

#[utoipa::path(
    get,
    path = "/version",
    responses((status = 200, description = "Service version", body = VersionDto))
)]
pub(crate) async fn version(State(state): State<AppState>) -> impl IntoResponse {
    let (version, build, components) = identity_fields(&state);
    Json(VersionDto {
        version,
        build,
        components,
    })
}

/// Server build information for the About screen: identity only, no config
/// value (SHELL-OPS-7, design D4). Reads no database — `meta` is a pure
/// registry+config read.
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
pub(crate) async fn meta(State(state): State<AppState>) -> impl IntoResponse {
    let (version, build, components) = identity_fields(&state);
    Json(ServerMetaDto {
        version,
        build,
        url: state.server_url.clone(),
        components,
    })
}
