use crate::WorkspaceScope;
use crate::ids::{SecurityAuditId, UserId};
use atlas_core::Attribution;
use atlas_core::attribution::ActorTypeFilter;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// All security-relevant action verbs. The column in `security_audit_log` is
/// TEXT (no enum constraint in the DB), but this type guards write-sites against
/// typo drift across the 16 instrumented call-sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityAction {
    MembershipAdded,
    MembershipRoleChanged,
    MembershipRemoved,
    GrantCreated,
    GrantRevoked,
    ApiKeyCreated,
    ApiKeyRevoked,
    ApiKeyGlobalChanged,
    ApiKeyScopesChanged,
    ApiKeyGrantRevoked,
    UserCreated,
    UserDisabled,
    UserEnabled,
    UserSystemAdminSet,
    UserPasswordReset,
    UserActivationRegenerated,
    AccountActivated,
    GroupCreated,
    GroupDeleted,
    GroupMemberAdded,
    GroupMemberRemoved,
    ResourceDeleted,
    ResourceRestored,
    ResourcePurgeCommitted,
    ResourcePurgeCleanupPending,
    ResourcePurgeCleanupFailed,
    ResourcePurgeCompleted,
}

impl SecurityAction {
    pub fn as_str(self) -> &'static str {
        match self {
            SecurityAction::MembershipAdded => "membership.added",
            SecurityAction::MembershipRoleChanged => "membership.role_changed",
            SecurityAction::MembershipRemoved => "membership.removed",
            SecurityAction::GrantCreated => "grant.created",
            SecurityAction::GrantRevoked => "grant.revoked",
            SecurityAction::ApiKeyCreated => "api_key.created",
            SecurityAction::ApiKeyRevoked => "api_key.revoked",
            SecurityAction::ApiKeyGlobalChanged => "api_key.global_changed",
            SecurityAction::ApiKeyScopesChanged => "api_key.scopes_changed",
            SecurityAction::ApiKeyGrantRevoked => "api_key_grant.revoked",
            SecurityAction::UserCreated => "user.created",
            SecurityAction::UserDisabled => "user.disabled",
            SecurityAction::UserEnabled => "user.enabled",
            SecurityAction::UserSystemAdminSet => "user.system_admin_set",
            SecurityAction::UserPasswordReset => "user.password_reset",
            SecurityAction::UserActivationRegenerated => "user.activation_regenerated",
            SecurityAction::AccountActivated => "account.activated",
            SecurityAction::GroupCreated => "group.created",
            SecurityAction::GroupDeleted => "group.deleted",
            SecurityAction::GroupMemberAdded => "group.member_added",
            SecurityAction::GroupMemberRemoved => "group.member_removed",
            SecurityAction::ResourceDeleted => "resource.deleted",
            SecurityAction::ResourceRestored => "resource.restored",
            SecurityAction::ResourcePurgeCommitted => "resource.purge_committed",
            SecurityAction::ResourcePurgeCleanupPending => "resource.purge_cleanup_pending",
            SecurityAction::ResourcePurgeCleanupFailed => "resource.purge_cleanup_failed",
            SecurityAction::ResourcePurgeCompleted => "resource.purge_completed",
        }
    }
}

/// Input type for inserting a new security audit event.
///
/// Passed to `SecurityAuditRepo::append_in`. The actor field is an `Attribution`
/// enum that enforces the XOR invariant at the type level; the DB also enforces it
/// via CHECK constraint.
#[derive(Debug, Clone)]
pub struct NewSecurityAuditEvent {
    pub workspace_id: Option<WorkspaceScope>,
    pub actor: Attribution,
    pub action: SecurityAction,
    pub target_type: String,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
}

/// A persisted security audit event (read model).
#[derive(Debug, Clone)]
pub struct SecurityAuditEvent {
    pub id: SecurityAuditId,
    pub workspace_id: Option<WorkspaceScope>,
    pub actor: Attribution,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Opaque keyset cursor for paginating `security_audit_log` in `(created_at DESC, id DESC)` order.
#[derive(Debug, Clone)]
pub struct AuditCursor {
    pub created_at: DateTime<Utc>,
    pub id: SecurityAuditId,
}

/// Optional filters for audit log queries.
#[derive(Debug, Clone, Default)]
pub struct AuditFilters {
    pub actor_user_id: Option<UserId>,
    pub actor_type: Option<ActorTypeFilter>,
    pub action: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_action_as_str_round_trips() {
        let cases = [
            (SecurityAction::MembershipAdded, "membership.added"),
            (
                SecurityAction::MembershipRoleChanged,
                "membership.role_changed",
            ),
            (SecurityAction::MembershipRemoved, "membership.removed"),
            (SecurityAction::GrantCreated, "grant.created"),
            (SecurityAction::GrantRevoked, "grant.revoked"),
            (SecurityAction::ApiKeyCreated, "api_key.created"),
            (SecurityAction::ApiKeyRevoked, "api_key.revoked"),
            (
                SecurityAction::ApiKeyGlobalChanged,
                "api_key.global_changed",
            ),
            (
                SecurityAction::ApiKeyScopesChanged,
                "api_key.scopes_changed",
            ),
            (SecurityAction::ApiKeyGrantRevoked, "api_key_grant.revoked"),
            (SecurityAction::UserCreated, "user.created"),
            (SecurityAction::UserDisabled, "user.disabled"),
            (SecurityAction::UserEnabled, "user.enabled"),
            (SecurityAction::UserSystemAdminSet, "user.system_admin_set"),
            (SecurityAction::UserPasswordReset, "user.password_reset"),
            (
                SecurityAction::UserActivationRegenerated,
                "user.activation_regenerated",
            ),
            (SecurityAction::AccountActivated, "account.activated"),
            (SecurityAction::GroupCreated, "group.created"),
            (SecurityAction::GroupDeleted, "group.deleted"),
            (SecurityAction::GroupMemberAdded, "group.member_added"),
            (SecurityAction::GroupMemberRemoved, "group.member_removed"),
            (SecurityAction::ResourceDeleted, "resource.deleted"),
            (SecurityAction::ResourceRestored, "resource.restored"),
            (
                SecurityAction::ResourcePurgeCommitted,
                "resource.purge_committed",
            ),
            (
                SecurityAction::ResourcePurgeCleanupPending,
                "resource.purge_cleanup_pending",
            ),
            (
                SecurityAction::ResourcePurgeCleanupFailed,
                "resource.purge_cleanup_failed",
            ),
            (
                SecurityAction::ResourcePurgeCompleted,
                "resource.purge_completed",
            ),
        ];

        for (action, expected) in cases {
            assert_eq!(
                action.as_str(),
                expected,
                "action {action:?} as_str mismatch"
            );
        }
    }

    #[test]
    fn audit_filters_default_has_all_nones() {
        let f = AuditFilters::default();
        assert!(f.actor_user_id.is_none());
        assert!(f.actor_type.is_none());
        assert!(f.action.is_none());
        assert!(f.from.is_none());
        assert!(f.to.is_none());
    }
}
