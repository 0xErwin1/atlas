//! The reverse-grant discovery port (E11-S8, D-S8-1): given a principal,
//! answer which resource refs it holds grants on, grouped by product.
//!
//! Deliberately not `atlas_core::principal::Principal` (D1): that enum
//! covers `Group` as an authenticating actor, which discovery never receives
//! directly (a user reaches a grant *through* a group, but a group is never
//! itself the caller). `DiscoveryPrincipal` also carries the admin flags so
//! `is_platform_admin()` is answered without an extra round trip to the user
//! repo, matching `routes/auth.rs:216-265`'s assembly pattern.
use crate::ids::{ApiKeyId, UserId};
use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::ids::ResourceRef;
use std::collections::{BTreeMap, BTreeSet};

/// The authenticated caller asking "what can I reach?".
///
/// Exactly one of `user_id`/`api_key_id` is set, mirroring the
/// `permission_grants_principal_xor` DB invariant (D2) — a group is never a
/// discovery caller. `is_root`/`is_system_admin` are always `false` for an
/// api-key principal (`routes/auth.rs:261-262`).
#[derive(Debug, Clone)]
pub struct DiscoveryPrincipal {
    pub user_id: Option<UserId>,
    pub api_key_id: Option<ApiKeyId>,
    pub is_root: bool,
    pub is_system_admin: bool,
}

impl DiscoveryPrincipal {
    /// The single admin derivation used across discovery (INV-ADMIN-FROM-FLAGS):
    /// `is_root || is_system_admin`, never a function of grant count.
    pub fn is_platform_admin(&self) -> bool {
        self.is_root || self.is_system_admin
    }
}

/// Cap on `GrantedScopes` (review CRITICAL, resilience): a fixed constant,
/// not a caller-supplied field, so no caller can opt out of it.
pub const MAX_DISCOVERY_SCOPES: usize = 500;

/// The grant-derived discovery result: every resource ref the principal holds
/// a grant on (directly, or through a group membership), grouped by
/// `ResourceRef::product()`.
///
/// A product absent from the map means the principal holds zero grants for
/// that product (INV-ABSENT-NOT-EMPTY) — callers must not insert an empty
/// set as a substitute for omission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantedScopes {
    by_product: BTreeMap<String, BTreeSet<ResourceRef>>,
    pub truncated: bool,
}

impl GrantedScopes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts one granted ref under its own `product()`, creating the
    /// product's entry on first insert.
    pub fn insert(&mut self, resource_ref: ResourceRef) {
        let total: usize = self.by_product.values().map(BTreeSet::len).sum();
        if total >= MAX_DISCOVERY_SCOPES {
            self.truncated = true;
            return;
        }
        self.by_product
            .entry(resource_ref.product().to_string())
            .or_default()
            .insert(resource_ref);
    }

    /// Scopes granted for a single product, or `None` if the principal has
    /// no grant for that product at all.
    pub fn for_product(&self, product: &str) -> Option<&BTreeSet<ResourceRef>> {
        self.by_product.get(product)
    }

    pub fn is_empty(&self) -> bool {
        self.by_product.is_empty()
    }
}

/// The Custos-owned reverse-grant lookup (D-S8-1): given a principal, return
/// every resource ref it holds a direct or group-derived grant on.
///
/// Deliberately not `PermissionGrantRepo::load_grants_for_resolution`
/// (workspace-forward, wrong direction) nor a membership listing (Acta-owned,
/// D-S8-9 unions that separately in `atlas_server`). Root/system-admin
/// principals are answered from `DiscoveryPrincipal::is_platform_admin()` by
/// the caller, without ever invoking this port (spec: "Admin flag
/// short-circuits grant evaluation") — implementations MAY still be called
/// for an admin principal and MUST return whatever grants exist, but callers
/// own the short-circuit.
#[async_trait]
pub trait DiscoveryPort: Send + Sync {
    /// Returns the union of every resource ref the principal reaches through
    /// a direct grant or a group grant, grouped by product.
    async fn granted_scopes(
        &self,
        principal: &DiscoveryPrincipal,
    ) -> Result<GrantedScopes, DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(product: &str, kind: &str, id: &str) -> ResourceRef {
        ResourceRef::new(product, kind, id).expect("valid resource ref")
    }

    #[test]
    fn is_platform_admin_is_true_when_either_flag_is_set() {
        let root = DiscoveryPrincipal {
            user_id: Some(UserId::new()),
            api_key_id: None,
            is_root: true,
            is_system_admin: false,
        };
        let system_admin = DiscoveryPrincipal {
            user_id: Some(UserId::new()),
            api_key_id: None,
            is_root: false,
            is_system_admin: true,
        };

        assert!(root.is_platform_admin());
        assert!(system_admin.is_platform_admin());
    }

    #[test]
    fn is_platform_admin_is_false_when_neither_flag_is_set() {
        let principal = DiscoveryPrincipal {
            user_id: Some(UserId::new()),
            api_key_id: None,
            is_root: false,
            is_system_admin: false,
        };

        assert!(!principal.is_platform_admin());
    }

    #[test]
    fn api_key_principal_is_never_platform_admin() {
        let principal = DiscoveryPrincipal {
            user_id: None,
            api_key_id: Some(ApiKeyId::new()),
            is_root: false,
            is_system_admin: false,
        };

        assert!(!principal.is_platform_admin());
    }

    #[test]
    fn granted_scopes_groups_inserted_refs_by_product() {
        let mut scopes = GrantedScopes::new();
        scopes.insert(resource("acta", "workspace", "w1"));
        scopes.insert(resource("acta", "project", "p1"));
        scopes.insert(resource("custos", "admin", "root"));

        assert_eq!(
            scopes.for_product("acta"),
            Some(&BTreeSet::from([
                resource("acta", "workspace", "w1"),
                resource("acta", "project", "p1"),
            ]))
        );
        assert_eq!(
            scopes.for_product("custos"),
            Some(&BTreeSet::from([resource("custos", "admin", "root")]))
        );
    }

    #[test]
    fn a_product_with_no_grants_is_absent_not_empty() {
        let scopes = GrantedScopes::new();

        assert_eq!(scopes.for_product("acta"), None);
        assert!(scopes.is_empty());
    }

    #[test]
    fn insert_enforces_the_cap() {
        let mut scopes = GrantedScopes::new();
        for i in 0..(MAX_DISCOVERY_SCOPES + 1) {
            scopes.insert(resource("acta", "project", &i.to_string()));
        }

        assert_eq!(
            scopes.for_product("acta").expect("acta present").len(),
            MAX_DISCOVERY_SCOPES
        );
        assert!(scopes.truncated);
    }
}
