use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod webhook_delivery_log {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "webhook_delivery_log")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub subscription_id: Uuid,
        pub outbox_event_id: Uuid,
        pub attempt_no: i32,
        pub outcome: String,
        pub status_code: Option<i32>,
        pub response_snippet: Option<String>,
        pub error: Option<String>,
        pub duration_ms: Option<i32>,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
