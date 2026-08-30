//! Security audit entities relocated to `atlas_custos::entities::security_audit`
//! (S2d). Re-exported here to keep every existing
//! `crate::entities::security_audit::*` import path compiling.

pub use atlas_custos::entities::security_audit::{
    AuditCursor, AuditFilters, NewSecurityAuditEvent, SecurityAction, SecurityAuditEvent,
};
