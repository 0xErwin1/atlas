use atlas_core::error::DomainError;
use atlas_custos::entities::principals::{Principal, PrincipalKind};
use atlas_custos::ids::PrincipalId;
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod principal {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "custos", table_name = "principals")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub kind: String,
        pub display_name: String,
        pub deactivated_at: Option<DateTime<Utc>>,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// Maps a stored row onto the pure `Principal` record. An unknown `kind` is a
/// data-integrity fault, not a defaulting situation, so it surfaces as an error.
pub fn principal_from(m: principal::Model) -> Result<Principal, DomainError> {
    let kind = m
        .kind
        .parse::<PrincipalKind>()
        .map_err(|e| DomainError::InvalidInput {
            message: format!("stored principal kind is invalid: {e}"),
        })?;

    Ok(Principal {
        id: PrincipalId(m.id),
        kind,
        display_name: m.display_name,
        deactivated_at: m.deactivated_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}
