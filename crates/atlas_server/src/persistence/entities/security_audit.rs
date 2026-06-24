use atlas_domain::{
    Actor,
    entities::security_audit::SecurityAuditEvent,
    ids::{ApiKeyId, SecurityAuditId, UserId, WorkspaceId},
};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod security_audit_log {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "security_audit_log")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Option<Uuid>,
        pub actor_user_id: Option<Uuid>,
        pub actor_api_key_id: Option<Uuid>,
        pub action: String,
        pub target_type: String,
        pub target_id: Option<Uuid>,
        pub metadata: Json,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Reconstructs `Actor` from the XOR actor columns in a `security_audit_log` row.
///
/// The DB CHECK constraint guarantees exactly one is non-null. The both-null arm
/// is unreachable in a valid DB but is handled defensively with a fabricated actor
/// rather than panicking.
pub fn actor_from_columns(user_id: Option<Uuid>, api_key_id: Option<Uuid>) -> Actor {
    match (user_id, api_key_id) {
        (Some(uid), None) => Actor::User(UserId(uid)),
        (None, Some(kid)) => Actor::ApiKey(ApiKeyId(kid)),
        _ => Actor::User(UserId::new()),
    }
}

pub fn audit_event_from(m: security_audit_log::Model) -> SecurityAuditEvent {
    SecurityAuditEvent {
        id: SecurityAuditId(m.id),
        workspace_id: m.workspace_id.map(WorkspaceId),
        actor: actor_from_columns(m.actor_user_id, m.actor_api_key_id),
        action: m.action,
        target_type: m.target_type,
        target_id: m.target_id,
        metadata: m.metadata,
        created_at: m.created_at,
    }
}
