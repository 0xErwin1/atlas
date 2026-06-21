use atlas_domain::DomainError;
use atlas_domain::entities::task_views::{Owner, TaskView, TaskViewFilters};
use atlas_domain::ids::{ApiKeyId, TaskViewId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod task_view {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "task_views")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub workspace_id: Uuid,
        pub name: String,
        pub filters: Json,
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

/// Derives an `Owner` from the XOR user/api-key pair in a task_views row.
///
/// The DB CHECK constraint guarantees exactly one column is non-null. The
/// both-null arm is unreachable; returning a fabricated owner there avoids
/// threading `Result` through every caller. Identical rationale to the
/// saved_searches owner mapper.
pub fn owner_from_columns(user_id: Option<Uuid>, api_key_id: Option<Uuid>) -> Owner {
    match (user_id, api_key_id) {
        (Some(uid), None) => Owner::User(UserId(uid)),
        (None, Some(kid)) => Owner::ApiKey(ApiKeyId(kid)),
        _ => Owner::User(UserId::new()),
    }
}

/// Maps a raw SeaORM model row to the domain `TaskView`, deserializing the
/// `filters` JSONB column.
///
/// Returns an error if the stored JSON cannot be deserialized as `TaskViewFilters`.
/// This can happen if the column is corrupted; unlike saved_searches' `query` field,
/// filters deserialization is fallible and the caller must handle the Result.
pub fn task_view_from(m: task_view::Model) -> Result<TaskView, DomainError> {
    let filters: TaskViewFilters =
        serde_json::from_value(m.filters).map_err(|e| DomainError::Internal {
            message: format!("failed to deserialize task_view filters: {e}"),
        })?;

    Ok(TaskView {
        id: TaskViewId(m.id),
        workspace_id: WorkspaceId(m.workspace_id),
        name: m.name,
        filters,
        owner: owner_from_columns(m.owner_user_id, m.owner_api_key_id),
        created_at: m.created_at,
        updated_at: m.updated_at,
        deleted_at: m.deleted_at,
    })
}
