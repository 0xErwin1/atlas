use atlas_domain::{
    DomainError,
    entities::identity::{ApiKeyType, NewApiKey},
    ids::UserId,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, Statement,
};
use uuid::Uuid;

use crate::persistence::entities::integration_config::integration_configs;
use crate::persistence::repos::identity::PgApiKeyRepo;

pub struct PgIntegrationConfigRepo;

impl PgIntegrationConfigRepo {
    /// Creates an integration config and provisions a linked `ApiKeyType::Integration`
    /// api key for attribution.
    ///
    /// The provisioned key is never usable for bearer auth: its `token_hash` is a
    /// SHA-256 of random bytes and the plaintext is never returned. The caller
    /// should wrap this call in a transaction so that key provisioning and config
    /// creation succeed or fail atomically.
    pub async fn create(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        integration: String,
        encrypted_secret: Vec<u8>,
        secret_nonce: Vec<u8>,
        created_by_user_id: Uuid,
    ) -> Result<integration_configs::Model, DomainError> {
        let key = PgApiKeyRepo::create_for_user_in(
            conn,
            UserId(created_by_user_id),
            NewApiKey {
                name: format!("integration/{integration}"),
                token_hash: random_unissued_token_hash(),
                type_: ApiKeyType::Integration,
                expires_at: None,
                // This key's token_hash is a hash of random bytes that is never
                // issued, so it can never authenticate and its scope set is inert.
                // Seed it fail-closed with no scopes: if that never-authenticates
                // invariant were ever broken, an empty scope set denies every
                // capability-gated route rather than granting the full catalog.
                scopes: Vec::new(),
            },
        )
        .await?;

        let now = Utc::now();
        let model = integration_configs::ActiveModel {
            id: Set(Uuid::now_v7()),
            workspace_id: Set(workspace_id),
            integration: Set(integration),
            encrypted_secret: Set(encrypted_secret),
            secret_nonce: Set(secret_nonce),
            integration_api_key_id: Set(key.id.0),
            is_active: Set(true),
            created_by_user_id: Set(created_by_user_id),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };

        model.insert(conn).await.map_err(db_err)
    }

    /// Returns the single active (not soft-deleted) config for a workspace and
    /// integration slug, or `None` when none exists.
    pub async fn find_active(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        integration: &str,
    ) -> Result<Option<integration_configs::Model>, DomainError> {
        integration_configs::Entity::find()
            .filter(integration_configs::Column::WorkspaceId.eq(workspace_id))
            .filter(integration_configs::Column::Integration.eq(integration))
            .filter(integration_configs::Column::IsActive.eq(true))
            .filter(integration_configs::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)
    }

    /// Returns a single non-deleted config by its UUID within a workspace.
    pub async fn get_by_id(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Option<integration_configs::Model>, DomainError> {
        integration_configs::Entity::find_by_id(id)
            .filter(integration_configs::Column::WorkspaceId.eq(workspace_id))
            .filter(integration_configs::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)
    }

    /// Lists all non-deleted configs for a workspace, ordered by creation time.
    pub async fn list(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
    ) -> Result<Vec<integration_configs::Model>, DomainError> {
        integration_configs::Entity::find()
            .filter(integration_configs::Column::WorkspaceId.eq(workspace_id))
            .filter(integration_configs::Column::DeletedAt.is_null())
            .order_by_asc(integration_configs::Column::CreatedAt)
            .all(conn)
            .await
            .map_err(db_err)
    }

    /// Sets the `is_active` flag on a config, returning the updated row.
    ///
    /// Returns `NotFound` when the config does not exist (or is deleted) in the
    /// workspace. Deactivating a config makes the inbound ingest reject its
    /// events (ingest resolves configs via `find_active`).
    pub async fn set_active(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
        is_active: bool,
    ) -> Result<integration_configs::Model, DomainError> {
        let config = integration_configs::Entity::find_by_id(id)
            .filter(integration_configs::Column::WorkspaceId.eq(workspace_id))
            .filter(integration_configs::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "integration_config",
                id,
            })?;

        let mut active = config.into_active_model();
        active.is_active = Set(is_active);
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)
    }

    /// Soft-deletes a config and revokes its provisioned `integration_api_key_id`.
    ///
    /// The api key row is kept (the FK uses `ON DELETE RESTRICT` because tasks may
    /// reference it for attribution). The caller should wrap this in a transaction
    /// so that the config deletion and key revocation are atomic.
    pub async fn soft_delete_and_revoke_key(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), DomainError> {
        let config = integration_configs::Entity::find_by_id(id)
            .filter(integration_configs::Column::WorkspaceId.eq(workspace_id))
            .filter(integration_configs::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "integration_config",
                id,
            })?;

        let api_key_id = config.integration_api_key_id;

        let mut active = config.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;

        conn.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE api_keys SET revoked_at = $1 WHERE id = $2 AND revoked_at IS NULL",
            [Utc::now().into(), api_key_id.into()],
        ))
        .await
        .map_err(db_err)?;

        Ok(())
    }
}

/// Generates a random SHA-256 token hash that will never be matched during
/// bearer-auth lookup. The plaintext is discarded immediately; only the hash is
/// returned so the key cannot be used for authentication.
fn random_unissued_token_hash() -> String {
    use rand::RngCore;
    use sha2::{Digest, Sha256};
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("{:x}", Sha256::digest(bytes))
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
