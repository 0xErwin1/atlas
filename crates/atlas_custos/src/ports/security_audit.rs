use async_trait::async_trait;
use atlas_core::error::DomainError;

use crate::WorkspaceScope;
use crate::entities::security_audit::{AuditCursor, AuditFilters, SecurityAuditEvent};

/// Read-only port for the security audit log.
///
/// The write path (`append_in`) is a static method on `PgSecurityAuditRepo` rather than
/// a trait method because it requires a generic `ConnectionTrait` parameter (to participate
/// in the caller's transaction), and domain traits cannot depend on sea_orm. Callers invoke
/// it as `PgSecurityAuditRepo::append_in(&txn, event)` on the concrete type.
#[async_trait]
pub trait SecurityAuditRepo: Send + Sync {
    /// Lists audit events for `scope`, newest-first, with optional filters and keyset pagination.
    ///
    /// The caller overfetches by 1 to derive `has_more`; this method returns exactly what
    /// the DB returns (no truncation).
    async fn list_for_workspace(
        &self,
        scope: WorkspaceScope,
        filters: &AuditFilters,
        cursor: Option<AuditCursor>,
        limit: u64,
    ) -> Result<Vec<SecurityAuditEvent>, DomainError>;

    /// Lists platform-scoped audit events (`workspace_id IS NULL`), newest-first.
    ///
    /// Same overfetch contract as `list_for_workspace`.
    async fn list_platform(
        &self,
        filters: &AuditFilters,
        cursor: Option<AuditCursor>,
        limit: u64,
    ) -> Result<Vec<SecurityAuditEvent>, DomainError>;
}
