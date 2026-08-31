use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod event_outbox {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "events_outbox")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub event_type: String,
        pub event_version: i32,
        pub source: String,
        pub project_id: Option<Uuid>,
        pub board_id: Option<Uuid>,
        pub aggregate_type: String,
        pub aggregate_id: Uuid,
        pub payload: Json,
        pub occurred_at: DateTime<Utc>,
        pub status: String,
        pub attempt_count: i32,
        pub next_attempt_at: DateTime<Utc>,
        pub locked_until: Option<DateTime<Utc>>,
        pub last_error: Option<String>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
