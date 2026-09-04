//! Per-component `GET /health`/`GET /ready` handlers for `custos` and
//! `acta` (design D2): each renders that component's own bound
//! `Health`/`Readiness` result exactly, with no aggregation
//! (SHELL-OPS-3, INV-NO-REINTERPRET). `platform`'s root triad is its own
//! probe (design D2) and is not duplicated here.

use std::time::Duration;

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use atlas_api::dtos::ComponentProbeDto;
use atlas_core::capabilities::{HealthStatus, ReadinessStatus};
use atlas_core::registry::ComponentId;

use crate::ops::ComponentDiagnostics;
use crate::state::AppState;

/// The fixed reason a probe answers 503 with when its component has no
/// bound diagnostics row. `default_registry` binds every diagnostics-bearing
/// component, so this is reachable only through a test-seam table.
const UNBOUND_REASON: &str = "no diagnostics implementer is bound";

/// The fixed reason a per-component readiness probe answers 503 with when it
/// outlives the same per-component budget root `/ready` applies.
const DEADLINE_REASON: &str = "readiness check exceeded its deadline";

/// Looks up `id`'s bound diagnostics; `None` when no row is bound.
fn diagnostics_for<'a>(state: &'a AppState, id: &str) -> Option<&'a ComponentDiagnostics> {
    let component = ComponentId::new(id).ok()?;

    state.diagnostics.get(&component)
}

fn health_response(component: &str, diagnostics: Option<&ComponentDiagnostics>) -> Response {
    let Some(diagnostics) = diagnostics else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ComponentProbeDto {
                component: component.to_string(),
                status: "down".to_string(),
                reason: Some(UNBOUND_REASON.to_string()),
            }),
        )
            .into_response();
    };

    let probe = match diagnostics.health.health() {
        HealthStatus::Ok => ComponentProbeDto {
            component: component.to_string(),
            status: "ok".to_string(),
            reason: None,
        },
        HealthStatus::Degraded { reason } => ComponentProbeDto {
            component: component.to_string(),
            status: "degraded".to_string(),
            reason: Some(reason),
        },
        HealthStatus::Down { reason } => ComponentProbeDto {
            component: component.to_string(),
            status: "down".to_string(),
            reason: Some(reason),
        },
    };

    (StatusCode::OK, Json(probe)).into_response()
}

/// Renders one component's readiness, bounded by `timeout` so a stalled
/// dependency yields `not_ready` instead of an open-ended request.
async fn readiness_response(
    component: &str,
    diagnostics: Option<&ComponentDiagnostics>,
    timeout: Duration,
) -> Response {
    let Some(diagnostics) = diagnostics else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ComponentProbeDto {
                component: component.to_string(),
                status: "not_ready".to_string(),
                reason: Some(UNBOUND_REASON.to_string()),
            }),
        )
            .into_response();
    };

    let status = tokio::time::timeout(timeout, diagnostics.readiness.readiness())
        .await
        .unwrap_or_else(|_elapsed| ReadinessStatus::NotReady {
            reason: DEADLINE_REASON.to_string(),
        });

    match status {
        ReadinessStatus::Ready => (
            StatusCode::OK,
            Json(ComponentProbeDto {
                component: component.to_string(),
                status: "ready".to_string(),
                reason: None,
            }),
        )
            .into_response(),
        ReadinessStatus::NotReady { reason } => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ComponentProbeDto {
                component: component.to_string(),
                status: "not_ready".to_string(),
                reason: Some(reason),
            }),
        )
            .into_response(),
    }
}

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "custos's own health signal", body = ComponentProbeDto),
        (status = 503, description = "custos has no bound diagnostics implementer", body = ComponentProbeDto),
    )
)]
pub(crate) async fn custos_health(State(state): State<AppState>) -> Response {
    health_response("custos", diagnostics_for(&state, "custos"))
}

#[utoipa::path(
    get,
    path = "/ready",
    responses(
        (status = 200, description = "custos is ready", body = ComponentProbeDto),
        (status = 503, description = "custos is not ready", body = ComponentProbeDto),
    )
)]
pub(crate) async fn custos_ready(State(state): State<AppState>) -> Response {
    readiness_response(
        "custos",
        diagnostics_for(&state, "custos"),
        state.readiness_timeout,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "acta's own health signal", body = ComponentProbeDto),
        (status = 503, description = "acta has no bound diagnostics implementer", body = ComponentProbeDto),
    )
)]
pub(crate) async fn acta_health(State(state): State<AppState>) -> Response {
    health_response("acta", diagnostics_for(&state, "acta"))
}

#[utoipa::path(
    get,
    path = "/ready",
    responses(
        (status = 200, description = "acta is ready", body = ComponentProbeDto),
        (status = 503, description = "acta is not ready", body = ComponentProbeDto),
    )
)]
pub(crate) async fn acta_ready(State(state): State<AppState>) -> Response {
    readiness_response(
        "acta",
        diagnostics_for(&state, "acta"),
        state.readiness_timeout,
    )
    .await
}
