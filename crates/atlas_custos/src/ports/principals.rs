use crate::entities::principals::{NewPrincipal, Principal, PrincipalKind};
use crate::ids::PrincipalId;
use async_trait::async_trait;
use atlas_core::error::DomainError;

/// Persistence for the shared `custos.principals` identity. The minimum the
/// E4 slices need: mint a principal (in the caller's transaction, so the
/// principal and its user/api-key row commit together), resolve one by id,
/// and list by kind.
#[async_trait]
pub trait PrincipalRepo: Send + Sync {
    async fn create(&self, new: NewPrincipal) -> Result<Principal, DomainError>;
    async fn find_by_id(&self, id: PrincipalId) -> Result<Option<Principal>, DomainError>;
    async fn find_by_kind(&self, kind: PrincipalKind) -> Result<Vec<Principal>, DomainError>;
}
