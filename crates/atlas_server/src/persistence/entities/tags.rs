use atlas_domain::entities::tags::Tag;
use atlas_domain::ids::{TagId, WorkspaceId};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

use super::boards_tasks::actor_from_columns;

pub mod tag {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "tags")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub name: String,
        pub color: Option<String>,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn tag_from(m: tag::Model) -> Tag {
    Tag {
        id: TagId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        name: m.name,
        color: m.color,
        created_by: actor_from_columns(m.created_by_user_id, m.created_by_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}
