use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::ids::ResourceRef;

/// Revokes every permission grant targeting one of `resources`.
///
/// Backs the hygiene replacement for the four dead-on-arrival Custos-owned
/// cascades D1 drops from `permission_grants`' target FKs. The purge closure
/// (`atlas_server::services::trash_service::delete_purge_closure`) collects
/// the ids of every row it hard-deletes and calls this port inside the same
/// transaction, so a grant never outlives the resource it targets.
///
/// Soft delete never calls this port — grants survive a soft delete exactly
/// as before S3c.
#[async_trait]
pub trait GrantHygiene: Send + Sync {
    async fn revoke_grants_for(&self, resources: &[ResourceRef]) -> Result<(), DomainError>;
}
