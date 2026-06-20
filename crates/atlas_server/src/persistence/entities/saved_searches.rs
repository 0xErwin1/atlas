use atlas_domain::entities::saved_searches::{Owner, SavedSearch};
use atlas_domain::ids::{ApiKeyId, SavedSearchId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod saved_search {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "saved_searches")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub name: String,
        pub query: String,
        pub owner_user_id: Option<Uuid>,
        pub owner_api_key_id: Option<Uuid>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
        pub deleted_at: Option<DateTime<Utc>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Derives an `Owner` from the XOR user/api-key pair in a saved_searches row.
///
/// The DB CHECK constraint guarantees exactly one column is non-null. The
/// both-null arm is unreachable; returning a fabricated owner there avoids
/// threading `Result` through every infallible read mapper.
pub fn owner_from_columns(user_id: Option<Uuid>, api_key_id: Option<Uuid>) -> Owner {
    match (user_id, api_key_id) {
        (Some(uid), None) => Owner::User(UserId(uid)),
        (None, Some(kid)) => Owner::ApiKey(ApiKeyId(kid)),
        _ => Owner::User(UserId::new()),
    }
}

pub fn saved_search_from(m: saved_search::Model) -> SavedSearch {
    SavedSearch {
        id: SavedSearchId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        name: m.name,
        query: m.query,
        owner: owner_from_columns(m.owner_user_id, m.owner_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    }
}
