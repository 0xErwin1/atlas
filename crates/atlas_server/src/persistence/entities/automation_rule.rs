use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod automation_rules {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "automation_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub name: String,
        pub is_active: bool,
        pub trigger_event_type: String,
        pub trigger_filter: Option<Json>,
        pub project_id: Option<Uuid>,
        pub action_type: String,
        pub action_params: Json,
        pub created_by_user_id: Uuid,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
