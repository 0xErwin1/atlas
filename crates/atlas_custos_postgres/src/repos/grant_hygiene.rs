use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::ids::ResourceRef;
use atlas_custos::ports::grant_hygiene::GrantHygiene as GrantHygieneTrait;
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::entities::permissions::permission_grant;
use atlas_postgres::db_err;

pub struct PgGrantHygiene {
    pub conn: DatabaseConnection,
}

impl PgGrantHygiene {
    /// Deletes every grant targeting one of `resources`, on the caller's
    /// connection or transaction. Called by
    /// `atlas_server::services::trash_service::delete_purge_closure` inside
    /// the same transaction as the purge, so the revoke commits or rolls back
    /// with the parent-row deletes it follows.
    pub async fn revoke_grants_for_in<C: ConnectionTrait>(
        conn: &C,
        resources: &[ResourceRef],
    ) -> Result<(), DomainError> {
        // Each ref binds one parameter, and Postgres caps a statement at
        // 65535 binds, so an unbounded purge closure must not become one
        // unbounded IN list. Chunking keeps every statement's parameter
        // count fixed while staying set-based per chunk.
        const REVOKE_CHUNK: usize = 1024;

        for chunk in resources.chunks(REVOKE_CHUNK) {
            let refs: Vec<String> = chunk.iter().map(|r| r.to_string()).collect();

            permission_grant::Entity::delete_many()
                .filter(permission_grant::Column::ResourceRef.is_in(refs))
                .exec(conn)
                .await
                .map_err(db_err)?;
        }

        Ok(())
    }
}

#[async_trait]
impl GrantHygieneTrait for PgGrantHygiene {
    async fn revoke_grants_for(&self, resources: &[ResourceRef]) -> Result<(), DomainError> {
        PgGrantHygiene::revoke_grants_for_in(&self.conn, resources).await
    }
}
