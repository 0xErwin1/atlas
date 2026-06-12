use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod permission_grant {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "permission_grants")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub user_id: Option<Uuid>,
        pub api_key_id: Option<Uuid>,
        pub project_id: Option<Uuid>,
        pub folder_id: Option<Uuid>,
        pub document_id: Option<Uuid>,
        pub board_id: Option<Uuid>,
        pub role: String,
        pub created_by_user_id: Option<Uuid>,
        pub created_by_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
