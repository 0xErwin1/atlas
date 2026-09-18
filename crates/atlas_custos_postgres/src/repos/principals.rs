use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_custos::entities::principals::{NewPrincipal, Principal, PrincipalKind};
use atlas_custos::ids::PrincipalId;
use atlas_custos::ports::principals::PrincipalRepo;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, QueryOrder,
};

use crate::entities::principals::{principal, principal_from};
use atlas_postgres::db_err;

pub use atlas_custos::entities::principals::NewPrincipal as NewPrincipalInput;
pub use atlas_custos::ports::principals::PrincipalRepo as PrincipalRepoTrait;

pub struct PgPrincipalRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl PrincipalRepo for PgPrincipalRepo {
    async fn create(&self, new: NewPrincipal) -> Result<Principal, DomainError> {
        let model = principal::ActiveModel {
            id: Set(new.id.0),
            kind: Set(new.kind.as_str().to_string()),
            display_name: Set(new.display_name),
            deactivated_at: Set(new.deactivated_at),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        };
        model
            .insert(&self.conn)
            .await
            .map_err(db_err)
            .and_then(principal_from)
    }

    async fn find_by_id(&self, id: PrincipalId) -> Result<Option<Principal>, DomainError> {
        principal::Entity::find_by_id(id.0)
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(principal_from)
            .transpose()
    }

    async fn find_by_kind(&self, kind: PrincipalKind) -> Result<Vec<Principal>, DomainError> {
        principal::Entity::find()
            .filter(principal::Column::Kind.eq(kind.as_str()))
            .order_by_asc(principal::Column::CreatedAt)
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(principal_from)
            .collect()
    }
}

impl PgPrincipalRepo {
    /// Creates a principal using the provided connection or transaction.
    ///
    /// Used when the insert must be atomic with the `users`/`api_keys` row it
    /// backs inside an existing transaction: the sync discipline requires the
    /// principal and its owning row to commit together.
    pub async fn create_in<C: ConnectionTrait>(
        conn: &C,
        new: NewPrincipal,
    ) -> Result<Principal, DomainError> {
        let model = principal::ActiveModel {
            id: Set(new.id.0),
            kind: Set(new.kind.as_str().to_string()),
            display_name: Set(new.display_name),
            deactivated_at: Set(new.deactivated_at),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
        };
        model
            .insert(conn)
            .await
            .map_err(db_err)
            .and_then(principal_from)
    }
}
