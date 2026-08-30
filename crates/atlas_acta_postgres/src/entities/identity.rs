use atlas_acta::entities::identity::MemberRole;
use atlas_acta::entities::identity::Workspace;
use atlas_acta::entities::identity::WorkspaceMembership;
use atlas_acta::ids::MembershipId;
use atlas_acta::ids::WorkspaceId;
use atlas_core::principal::UserId;
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
        pub deleted_at: Option<DateTime<Utc>>,
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
