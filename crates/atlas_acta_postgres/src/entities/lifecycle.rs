use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod purge_operation {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "purge_operations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub target_kind: String,
        pub target_id: Uuid,
        pub original_actor_user_id: Uuid,
        pub commit_audit_id: Uuid,
        pub status: String,
        pub attempts: i32,
        pub last_action: String,
        pub last_executor_type: String,
        pub last_executor_id: Option<Uuid>,
        pub last_error: Option<String>,
        pub last_attempt_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod purge_operation_digest {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "purge_operation_digests")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub operation_id: Uuid,
        #[sea_orm(primary_key, auto_increment = false)]
        pub digest: String,
        pub status: String,
        pub attempts: i32,
        pub last_error: Option<String>,
        pub last_attempt_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
