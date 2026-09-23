//! Persistence ports for the V2 authorization records: custom roles,
//! grants and deny rules. Each port offers the administration surface
//! (create, get, list by product, delete); the grant and deny ports also
//! offer the evaluation load, which returns every row addressed to any
//! identity in a [`SubjectSet`] for one product.

use crate::entities::authorization::{
    CustomRole, DenyRecord, DenyRuleId, GrantId, GrantRecord, NewCustomRole, NewDenyRecord,
    NewGrantRecord, RoleId, SubjectSet,
};
use async_trait::async_trait;
use atlas_core::error::DomainError;

#[async_trait]
pub trait RoleRepo: Send + Sync {
    /// Creates a role. `(product, name)` is unique: a second role with the
    /// same pair fails with `DomainError::AlreadyExists`.
    async fn create(&self, new: NewCustomRole) -> Result<CustomRole, DomainError>;

    async fn get(&self, id: RoleId) -> Result<Option<CustomRole>, DomainError>;

    async fn list_by_product(&self, product: &str) -> Result<Vec<CustomRole>, DomainError>;

    /// Deletes a role, returning `false` when no such role exists. A role
    /// still referenced by a grant cannot be deleted: the call fails with
    /// `DomainError::ComponentConflict { code: ROLE_IN_USE_CONFLICT, .. }`
    /// (see [`crate::entities::authorization::ROLE_IN_USE_CONFLICT`]).
    async fn delete(&self, id: RoleId) -> Result<bool, DomainError>;
}

#[async_trait]
pub trait GrantV2Repo: Send + Sync {
    async fn create(&self, new: NewGrantRecord) -> Result<GrantRecord, DomainError>;

    async fn get(&self, id: GrantId) -> Result<Option<GrantRecord>, DomainError>;

    async fn list_by_product(&self, product: &str) -> Result<Vec<GrantRecord>, DomainError>;

    async fn delete(&self, id: GrantId) -> Result<bool, DomainError>;

    /// Every grant in `product` addressed to the principal, any of its
    /// groups, or any of its principal sets.
    async fn list_for_subjects(
        &self,
        product: &str,
        subjects: &SubjectSet,
    ) -> Result<Vec<GrantRecord>, DomainError>;
}

#[async_trait]
pub trait DenyRuleRepo: Send + Sync {
    async fn create(&self, new: NewDenyRecord) -> Result<DenyRecord, DomainError>;

    async fn get(&self, id: DenyRuleId) -> Result<Option<DenyRecord>, DomainError>;

    async fn list_by_product(&self, product: &str) -> Result<Vec<DenyRecord>, DomainError>;

    async fn delete(&self, id: DenyRuleId) -> Result<bool, DomainError>;

    /// Every deny rule in `product` addressed to the principal, any of its
    /// groups, or any of its principal sets.
    async fn list_for_subjects(
        &self,
        product: &str,
        subjects: &SubjectSet,
    ) -> Result<Vec<DenyRecord>, DomainError>;
}
