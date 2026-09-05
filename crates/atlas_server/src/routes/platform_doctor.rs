//! `POST /api/v2/platform/doctor` (E11-S3b design D6.1): runs every present
//! component's `Doctor` implementer, bounded by `TokioDeadline`, and
//! answers 200 with the aggregated report whether or not it carries
//! findings (SHELL-OPS-4). Never on the readiness path (INV-READY-UNCHANGED)
//! — `run_doctor` has no caller in `routes/health.rs`.

use axum::{Json, extract::State};

use atlas_api::dtos::{DoctorFindingDto, DoctorReportDto};
use atlas_core::capabilities::{DoctorFinding, Severity};
use atlas_core::ops::run_doctor;

use crate::authz::RequireUserAdmin;
use crate::ops::deadline::TokioDeadline;
use crate::state::AppState;

fn severity_str(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Critical => "critical",
    }
}

fn finding_to_dto(finding: DoctorFinding) -> DoctorFindingDto {
    DoctorFindingDto {
        component: finding.component.as_str().to_string(),
        severity: severity_str(finding.severity).to_string(),
        finding: finding.finding,
        action: finding.action,
    }
}

/// Runs `run_doctor` over the registry's declared doctor set
/// (`state.diagnostics`, `state.registry`, both built once at startup — no
/// per-request registry rebuild), bounded by `state.doctor_timeout`.
/// Reused by the handler and directly by tests that want the DTO without an
/// HTTP round trip.
async fn build_report(state: &AppState) -> DoctorReportDto {
    let set = state.diagnostics.doctor_set(&state.registry);
    let deadline = TokioDeadline {
        per_component: state.doctor_timeout,
    };
    let report = run_doctor(&set, &deadline).await;

    DoctorReportDto {
        findings: report.findings.into_iter().map(finding_to_dto).collect(),
    }
}

#[utoipa::path(
    post,
    path = "/doctor",
    tag = "doctor",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Aggregated doctor report, whether or not it carries findings", body = DoctorReportDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Admin access required"),
    )
)]
/// Runs every present component's doctor check and returns the aggregated
/// findings. Platform admin or root only; never gated by, or reachable
/// from, the readiness path.
pub(crate) async fn doctor(
    _admin: RequireUserAdmin,
    State(state): State<AppState>,
) -> Json<DoctorReportDto> {
    Json(build_report(&state).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_str_matches_the_shipped_serde_tag_values() {
        assert_eq!(severity_str(Severity::Info), "info");
        assert_eq!(severity_str(Severity::Warning), "warning");
        assert_eq!(severity_str(Severity::Critical), "critical");
    }

    #[test]
    fn finding_to_dto_carries_exactly_the_shipped_four_fields() {
        let finding = DoctorFinding {
            component: atlas_core::registry::ComponentId::new("acta").expect("valid component id"),
            severity: Severity::Warning,
            finding: "worker acta.webhook_dispatcher is Failed".to_string(),
            action: "restart the dispatcher worker".to_string(),
        };

        let dto = finding_to_dto(finding);

        assert_eq!(dto.component, "acta");
        assert_eq!(dto.severity, "warning");
        assert_eq!(dto.finding, "worker acta.webhook_dispatcher is Failed");
        assert_eq!(dto.action, "restart the dispatcher worker");
    }
}
