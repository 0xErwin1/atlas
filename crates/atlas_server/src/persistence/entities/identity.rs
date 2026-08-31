use crate::platform::UserUiState;
use atlas_core::principal::UserId;
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

// R1 scaffolding: `workspace`/`workspace_membership` (and their `_from`
// conversions) now live in `atlas_acta_postgres` (S4 PR1). Re-exporting them
// here keeps every existing `crate::persistence::entities::identity::*` call
// site unaffected by the move (retired at S5 per the S2/S3 plan).
pub use atlas_acta_postgres::entities::identity::{
    membership, membership_from, workspace, workspace_from,
};

pub mod user_ui_state {
    use super::*;

    // S4 PR9: `user_ui_state` moved to `platform.ui_state` (design §D4). The
    // module and Rust type names stay `user_ui_state` (retired at S5 per the
    // S2/S3 plan) — only the sea-orm schema/table attribution changed.
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "platform", table_name = "ui_state")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: Uuid,
        pub state: Json,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub fn user_ui_state_from(m: user_ui_state::Model) -> UserUiState {
    UserUiState {
        user_id: UserId(m.user_id),
        state: m.state,
        updated_at: m.updated_at,
    }
}
