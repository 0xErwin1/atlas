//! The one audit site for break-glass (root) sessions (`v2-e4-s3c`).
//!
//! Every state-changing request (POST/PUT/PATCH/DELETE) made under a root
//! session appends exactly one `root.action` audit row carrying the HTTP
//! method, the request path, and the justification the root stated at login
//! (`custos.sessions.root_reason`). The logic lives here and is invoked from
//! a single point inside `require_authn`, so no per-route call site can
//! forget it.
//!
//! Reads (GET/HEAD/OPTIONS/TRACE) are deliberately excluded: the per-request
//! row would be dominated by dashboard/polling read traffic, multiplying
//! audit volume without adding accountability — the break-glass reason
//! justifies *acting* on the system, and every mutation that acts is
//! captured here. A root read that matters (sessions, audit log, user
//! records) is already observable through targeted events and the audit log
//! itself.
//!
//! Fail-closed: a root session whose stored reason is missing (impossible
//! after the login gate and the `m20260920_000057` back-fill, but reachable
//! through out-of-band tampering) is refused with an actionable 403 and an
//! `error`-level log, never executed unaudited. An audit-write failure is
//! propagated, failing the request rather than letting it proceed unrecorded.

use axum::http::Method;

use crate::{error::ApiError, state::AppState};
use atlas_core::principal::UserId;
use atlas_custos::entities::security_audit::{NewSecurityAuditEvent, SecurityAction};
use atlas_custos_postgres::repos::security_audit::PgSecurityAuditRepo;

/// Whether this request mutates state and is therefore audited for a root
/// session. Only the four mutating methods are audited; reads are excluded
/// by the volume argument in the module docs.
pub(crate) fn is_state_changing(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// Appends the `root.action` audit row for one state-changing request made
/// under the root session `user_id`.
///
/// Fails the request on any audit-write error: a root mutation that could
/// not be recorded must not proceed silently.
pub(crate) async fn append_root_action(
    state: &AppState,
    user_id: UserId,
    method: &Method,
    path: &str,
    reason: &str,
) -> Result<(), ApiError> {
    PgSecurityAuditRepo::append_in(
        &*state.db,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: atlas_core::Attribution::User(atlas_core::attribution::UserAttributionId(
                user_id.0,
            )),
            action: SecurityAction::RootAction,
            target_type: "http_request".to_string(),
            target_id: None,
            metadata: serde_json::json!({
                "method": method.as_str(),
                "path": path,
                "reason": reason,
            }),
        },
    )
    .await
    .map_err(|e| {
        tracing::error!(
            user_id = ?user_id,
            method = method.as_str(),
            path,
            error = %e,
            "root-action audit write failed: refusing to let a break-glass request proceed unrecorded"
        );
        ApiError::Internal {
            message: e.to_string(),
        }
    })
}

/// Refuses a root session with no stored reason (fail closed).
fn missing_reason_error(user_id: UserId, method: &Method, path: &str) -> ApiError {
    tracing::error!(
        user_id = ?user_id,
        method = method.as_str(),
        path,
        "root session has no stored justification: refusing state-changing request (fail closed)"
    );
    ApiError::Forbidden {
        message: "this root session carries no recorded justification, so it cannot \
                  perform state-changing actions; log in again stating a reason"
            .into(),
    }
}

/// The single gate invoked from `require_authn` for every authenticated
/// request that resolved to a root session: refuses unjustified break-glass
/// mutations and audits justified ones, before the handler runs.
pub(crate) async fn gate_root_session(
    state: &AppState,
    user_id: UserId,
    reason: Option<&str>,
    method: &Method,
    path: &str,
) -> Result<(), ApiError> {
    if !is_state_changing(method) {
        return Ok(());
    }
    let reason = reason.ok_or_else(|| missing_reason_error(user_id, method, path))?;
    append_root_action(state, user_id, method, path, reason).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_changing_methods_are_the_four_mutating_verbs() {
        assert!(is_state_changing(&Method::POST));
        assert!(is_state_changing(&Method::PUT));
        assert!(is_state_changing(&Method::PATCH));
        assert!(is_state_changing(&Method::DELETE));
    }

    #[test]
    fn read_methods_are_not_state_changing() {
        assert!(!is_state_changing(&Method::GET));
        assert!(!is_state_changing(&Method::HEAD));
        assert!(!is_state_changing(&Method::OPTIONS));
        assert!(!is_state_changing(&Method::TRACE));
    }
}
