use atlas_core::principal::ApiKeyId;
use atlas_core::principal::UserId;
use atlas_custos::capability::Capability;
use atlas_custos::entities::identity::ActivationToken;
use atlas_custos::entities::identity::ApiKey;
use atlas_custos::entities::identity::ApiKeyType;
use atlas_custos::entities::identity::Session;
use atlas_custos::entities::identity::User;
use atlas_custos::ids::ActivationTokenId;
use atlas_custos::ids::SessionId;
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod user {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "custos", table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub username: String,
        pub display_name: String,
        pub email: Option<String>,
        pub password_hash: Option<String>,
        pub is_root: bool,
        pub is_system_admin: bool,
        pub disabled_at: Option<DateTime<Utc>>,
        pub activated_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod activation_token {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "custos", table_name = "user_activation_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        pub token_hash: String,
        pub expires_at: DateTime<Utc>,
        pub consumed_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod session {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "custos", table_name = "sessions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub user_id: Uuid,
        pub token_hash: String,
        pub expires_at: DateTime<Utc>,
        pub last_used_at: Option<DateTime<Utc>>,
        pub revoked_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod api_key {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "custos", table_name = "api_keys")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Option<Uuid>,
        pub created_by_user_id: Uuid,
        pub name: String,
        pub token_hash: String,
        #[sea_orm(column_name = "type")]
        pub type_: String,
        pub expires_at: Option<DateTime<Utc>>,
        pub last_used_at: Option<DateTime<Utc>>,
        pub revoked_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
        pub is_global: bool,
        pub scopes: Vec<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn user_from(m: user::Model) -> User {
    User {
        id: UserId(m.id),
        username: m.username,
        display_name: m.display_name,
        email: m.email,
        password_hash: m.password_hash,
        is_root: m.is_root,
        is_system_admin: m.is_system_admin,
        disabled_at: m.disabled_at,
        activated_at: m.activated_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

pub fn activation_token_from(m: activation_token::Model) -> ActivationToken {
    ActivationToken {
        id: ActivationTokenId(m.id),
        user_id: UserId(m.user_id),
        token_hash: m.token_hash,
        expires_at: m.expires_at,
        consumed_at: m.consumed_at,
        created_at: m.created_at,
    }
}

pub fn session_from(m: session::Model) -> Session {
    Session {
        id: SessionId(m.id),
        user_id: UserId(m.user_id),
        token_hash: m.token_hash,
        expires_at: m.expires_at,
        last_used_at: m.last_used_at,
        revoked_at: m.revoked_at,
        created_at: m.created_at,
    }
}

pub fn api_key_from(m: api_key::Model) -> ApiKey {
    ApiKey {
        id: ApiKeyId(m.id),
        workspace_id: m.workspace_id.map(atlas_custos::WorkspaceScope),
        created_by_user_id: UserId(m.created_by_user_id),
        name: m.name,
        token_hash: m.token_hash,
        type_: m.type_.parse::<ApiKeyType>().unwrap_or_default(),
        expires_at: m.expires_at,
        last_used_at: m.last_used_at,
        revoked_at: m.revoked_at,
        created_at: m.created_at,
        is_global: m.is_global,
        scopes: capabilities_from_stored(&m.scopes),
    }
}

/// Parses stored scope strings into `Capability`s, fail-closed: any entry that
/// does not parse (corrupt row, manual DB edit, or a forward-scope left behind
/// after a rollback) is dropped rather than defaulted, since defaulting an unknown
/// scope string would risk granting a capability the row never actually held.
///
/// Each dropped entry is logged: the fail-closed drop is silent to callers, so a
/// warn is the only signal that a stored scope was discarded. The raw scope string
/// is a capability identifier, not a secret, so it is safe to log for debugging.
pub(crate) fn capabilities_from_stored(raw: &[String]) -> Vec<Capability> {
    raw.iter()
        .filter_map(|s| match s.parse() {
            Ok(capability) => Some(capability),
            Err(_) => {
                tracing::warn!(
                    target: "authz.scope_drop",
                    event = "scope_drop",
                    raw_scope = %s,
                    "dropping unparseable stored capability scope"
                );
                None
            }
        })
        .collect()
}

/// Converts a scope set to its storage representation for the `scopes TEXT[]` column.
pub(crate) fn capabilities_to_stored(scopes: &[Capability]) -> Vec<String> {
    scopes.iter().map(|c| c.as_str().to_string()).collect()
}
