//! Read ports the authorization service loads evaluation facts through:
//! the actor's group memberships and the stored grants and deny rules that
//! can reach it.

use crate::entities::authorization::{CustomRole, DenyRecord, GrantRecord};
use crate::ids::{GroupId, PrincipalId};
use async_trait::async_trait;
use atlas_core::error::DomainError;

/// The products whose stored facts one evaluation needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductScope {
    /// Only facts of these products.
    Products(Vec<String>),
    /// Facts of every product: delegation actions apply to targets of any
    /// product.
    All,
}

/// A principal set declared by a product: stored set subjects of that
/// product ending in this set name are loadable, any other set is not.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeclaredSet {
    pub product: String,
    pub name: String,
}

/// The stored facts one evaluation loads in a single logical query.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredAuthorizationFacts {
    pub grants: Vec<GrantRecord>,
    pub denies: Vec<DenyRecord>,
    /// The custom roles the loaded grants reference.
    pub custom_roles: Vec<CustomRole>,
}

/// The groups an actor belongs to, read from the V1 group tables.
#[async_trait]
pub trait GroupMembershipSource: Send + Sync {
    /// Every live group `principal` is a member of. Only user principals
    /// belong to groups; any other principal has none.
    async fn groups_of(&self, principal: PrincipalId) -> Result<Vec<GroupId>, DomainError>;
}

/// The evaluation load of stored grants and deny rules.
#[async_trait]
pub trait AuthorizationFactsStore: Send + Sync {
    /// Every grant and deny rule in `scope` addressed to `principal`, to one
    /// of `groups`, or to an instance of one of `declared_sets`, plus the
    /// custom roles those grants reference. Principal-set rows are returned
    /// whatever the actor's membership, which the caller resolves only for
    /// the sets the rows name; with no declared sets, no set rows load. One
    /// logical load: an implementation may use several statements.
    async fn load(
        &self,
        scope: &ProductScope,
        principal: PrincipalId,
        groups: &[GroupId],
        declared_sets: &[DeclaredSet],
    ) -> Result<StoredAuthorizationFacts, DomainError>;
}
