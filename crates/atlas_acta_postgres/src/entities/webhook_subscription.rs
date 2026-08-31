use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod webhook_subscriptions {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "webhook_subscriptions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub target_url: String,
        pub event_types: Vec<String>,
        pub scope_type: String,
        pub scope_id: Option<Uuid>,
        pub encrypted_secret: Vec<u8>,
        pub secret_nonce: Vec<u8>,
        pub is_active: bool,
        pub label: Option<String>,
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
