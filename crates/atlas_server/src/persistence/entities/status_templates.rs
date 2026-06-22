use atlas_domain::{StatusTemplateId, WorkspaceId, entities::status_templates::StatusTemplate};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod status_template {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "workspace_status_templates")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub name: String,
        pub color: Option<String>,
        pub position_key: String,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn status_template_from(m: status_template::Model) -> StatusTemplate {
    StatusTemplate {
        id: StatusTemplateId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        name: m.name,
        color: m.color,
        position_key: m.position_key,
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}
