use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod integration_configs {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "integration_configs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub integration: String,
        pub encrypted_secret: Vec<u8>,
        pub secret_nonce: Vec<u8>,
        pub integration_api_key_id: Uuid,
        pub is_active: bool,
        pub created_by_user_id: Uuid,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
