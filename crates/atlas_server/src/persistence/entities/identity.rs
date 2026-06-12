use atlas_domain::entities::identity::{
    ApiKey, MemberRole, Session, User, Workspace, WorkspaceMembership,
};
use atlas_domain::ids::{ApiKeyId, MembershipId, SessionId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod workspace {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "workspaces")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub name: String,
        pub slug: String,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod user {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "users")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub username: String,
        pub display_name: String,
        pub password_hash: String,
        pub is_root: bool,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod session {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "sessions")]
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
    #[sea_orm(table_name = "api_keys")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub created_by_user_id: Uuid,
        pub name: String,
        pub token_hash: String,
        pub role: String,
        pub expires_at: Option<DateTime<Utc>>,
        pub last_used_at: Option<DateTime<Utc>>,
        pub revoked_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod membership {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "workspace_memberships")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub user_id: Uuid,
        pub role: String,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn workspace_from(m: workspace::Model) -> Workspace {
    Workspace {
        id: WorkspaceId(m.id),
        name: m.name,
        slug: m.slug,
        created_at: m.created_at,
        updated_at: m.updated_at,
    }
}

pub fn user_from(m: user::Model) -> User {
    User {
        id: UserId(m.id),
        username: m.username,
        display_name: m.display_name,
        password_hash: m.password_hash,
        is_root: m.is_root,
        created_at: m.created_at,
        updated_at: m.updated_at,
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
        workspace_id: WorkspaceId(m.workspace_id),
        created_by_user_id: UserId(m.created_by_user_id),
        name: m.name,
        token_hash: m.token_hash,
        role: m.role,
        expires_at: m.expires_at,
        last_used_at: m.last_used_at,
        revoked_at: m.revoked_at,
        created_at: m.created_at,
    }
}

pub fn membership_from(m: membership::Model) -> Result<WorkspaceMembership, String> {
    let role = match m.role.as_str() {
        "owner" => MemberRole::Owner,
        "admin" => MemberRole::Admin,
        "member" => MemberRole::Member,
        other => return Err(format!("unknown role: {other}")),
    };

    Ok(WorkspaceMembership {
        id: MembershipId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        user_id: UserId(m.user_id),
        role,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}
