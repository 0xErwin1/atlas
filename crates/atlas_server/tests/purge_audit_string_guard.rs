//! D5 guard: `atlas_acta::entities::lifecycle::PurgeStatus::audit_action_str()`
//! (Acta) must return exactly the same string as
//! `atlas_custos::entities::security_audit::SecurityAction::as_str()` (Custos)
//! for the matching resource-purge action, since both feed the same
//! `security_audit_log.action` TEXT column guarded by the DB CHECK constraint
//! in `m20260721_000043`. A mismatch here would silently desync Acta's
//! purge-status audit strings from the Custos-owned action catalog.

use atlas_acta::entities::lifecycle::PurgeStatus;
use atlas_custos::entities::security_audit::SecurityAction;

#[test]
fn purge_status_audit_strings_match_security_action_strings() {
    let cases = [
        (
            PurgeStatus::DbCommitted,
            SecurityAction::ResourcePurgeCommitted,
        ),
        (
            PurgeStatus::CleanupPending,
            SecurityAction::ResourcePurgeCleanupPending,
        ),
        (
            PurgeStatus::CleanupFailed,
            SecurityAction::ResourcePurgeCleanupFailed,
        ),
        (
            PurgeStatus::Complete,
            SecurityAction::ResourcePurgeCompleted,
        ),
    ];

    for (status, action) in cases {
        assert_eq!(
            status.audit_action_str(),
            action.as_str(),
            "PurgeStatus::{status:?}.audit_action_str() must equal SecurityAction::{action:?}.as_str()"
        );
    }
}
