//! Container-free contract tests for `DiscoveryPort`, exercised against a
//! `FakeDiscoveryRepo` rather than a real Postgres connection.
//!
//! These tests prove the port's shape is usable behind `Arc<dyn
//! DiscoveryPort>` (the way `atlas_server` will hold it once PR3 wires a
//! route to it) and that a well-behaved implementation upholds
//! INV-ABSENT-NOT-EMPTY and the union-of-sources contract described in the
//! spec ("Grant-derived and membership-derived sources are both present" /
//! "Group grants are not missed") at the level this port owns: a principal
//! reaching several distinct resource refs — whether through a direct grant
//! or (from the caller's point of view) a group grant already folded in by
//! the adapter — must see all of them unioned in one `GrantedScopes`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use async_trait::async_trait;
use atlas_core::error::DomainError;
use atlas_core::ids::ResourceRef;
use atlas_core::principal::{ApiKeyId, UserId};
use atlas_custos::ports::discovery::MAX_DISCOVERY_SCOPES;
use atlas_custos::ports::discovery::{DiscoveryPort, DiscoveryPrincipal, GrantedScopes};
use std::collections::HashMap;

/// A fake keyed by principal id, standing in for a real reverse-grant query.
/// Each entry already represents the union a real adapter's SQL would
/// produce (direct grants plus group-derived grants collapsed together) —
/// this fake never re-derives that union, it only proves callers of the
/// trait see whatever the implementation returns, unchanged.
struct FakeDiscoveryRepo {
    by_user: HashMap<UserId, Vec<ResourceRef>>,
    by_api_key: HashMap<ApiKeyId, Vec<ResourceRef>>,
}

#[async_trait]
impl DiscoveryPort for FakeDiscoveryRepo {
    async fn granted_scopes(
        &self,
        principal: &DiscoveryPrincipal,
    ) -> Result<GrantedScopes, DomainError> {
        let refs = match (principal.user_id, principal.api_key_id) {
            (Some(user_id), _) => self.by_user.get(&user_id).cloned().unwrap_or_default(),
            (None, Some(api_key_id)) => self
                .by_api_key
                .get(&api_key_id)
                .cloned()
                .unwrap_or_default(),
            (None, None) => Vec::new(),
        };

        let mut scopes = GrantedScopes::new();
        for resource_ref in refs {
            scopes.insert(resource_ref);
        }

        Ok(scopes)
    }
}

fn resource(product: &str, kind: &str, id: &str) -> ResourceRef {
    ResourceRef::new(product, kind, id).expect("valid resource ref")
}

#[tokio::test]
async fn a_user_principal_sees_direct_and_group_derived_grants_unioned() {
    let user_id = UserId::new();
    let mut by_user = HashMap::new();
    by_user.insert(
        user_id,
        vec![
            resource("acta", "workspace", "direct-ws"),
            resource("acta", "project", "group-project"),
        ],
    );
    let repo = FakeDiscoveryRepo {
        by_user,
        by_api_key: HashMap::new(),
    };

    let principal = DiscoveryPrincipal {
        user_id: Some(user_id),
        api_key_id: None,
        is_root: false,
        is_system_admin: false,
    };

    let scopes = repo
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");

    let acta = scopes.for_product("acta").expect("acta scopes present");
    assert!(acta.contains(&resource("acta", "workspace", "direct-ws")));
    assert!(acta.contains(&resource("acta", "project", "group-project")));
    assert_eq!(acta.len(), 2);
}

#[tokio::test]
async fn an_api_key_principal_is_looked_up_by_its_own_id_never_by_user_id() {
    let api_key_id = ApiKeyId::new();
    let mut by_api_key = HashMap::new();
    by_api_key.insert(api_key_id, vec![resource("acta", "project", "p1")]);
    let repo = FakeDiscoveryRepo {
        by_user: HashMap::new(),
        by_api_key,
    };

    let principal = DiscoveryPrincipal {
        user_id: None,
        api_key_id: Some(api_key_id),
        is_root: false,
        is_system_admin: false,
    };

    let scopes = repo
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");

    assert_eq!(
        scopes.for_product("acta"),
        Some(&std::collections::BTreeSet::from([resource(
            "acta", "project", "p1"
        )]))
    );
}

#[tokio::test]
async fn a_principal_with_zero_grants_yields_no_product_entries() {
    let repo = FakeDiscoveryRepo {
        by_user: HashMap::new(),
        by_api_key: HashMap::new(),
    };
    let principal = DiscoveryPrincipal {
        user_id: Some(UserId::new()),
        api_key_id: None,
        is_root: false,
        is_system_admin: false,
    };

    let scopes = repo
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");

    assert!(
        scopes.is_empty(),
        "a principal with zero grants must produce no product entries (INV-ABSENT-NOT-EMPTY)"
    );
    assert_eq!(scopes.for_product("acta"), None);
}

#[tokio::test]
async fn grant_volume_never_implies_admin_the_port_never_answers_it() {
    // The port itself has no notion of "admin" — that flag is answered from
    // `DiscoveryPrincipal::is_platform_admin()` by the caller, without
    // invoking this port at all (spec: "Admin flag short-circuits grant
    // evaluation"). This test documents that a non-admin principal with many
    // grants stays a plain grant list; nothing here ever produces an "admin"
    // signal.
    let user_id = UserId::new();
    let mut by_user = HashMap::new();
    by_user.insert(
        user_id,
        (0..50)
            .map(|i| resource("acta", "project", &i.to_string()))
            .collect(),
    );
    let repo = FakeDiscoveryRepo {
        by_user,
        by_api_key: HashMap::new(),
    };
    let principal = DiscoveryPrincipal {
        user_id: Some(user_id),
        api_key_id: None,
        is_root: false,
        is_system_admin: false,
    };

    assert!(!principal.is_platform_admin());
    let scopes = repo
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");
    assert_eq!(scopes.for_product("acta").expect("acta present").len(), 50);
}

/// Cap enforcement at under/at/over the cap, parameterized.
#[tokio::test]
async fn cap_enforcement_is_correct_at_under_at_and_over_the_cap() {
    for (yielded, expected_scopes, expected_truncated) in [
        (10, 10, false),
        (MAX_DISCOVERY_SCOPES, MAX_DISCOVERY_SCOPES, false),
        (MAX_DISCOVERY_SCOPES + 1, MAX_DISCOVERY_SCOPES, true),
    ] {
        let user_id = UserId::new();
        let refs = (0..yielded)
            .map(|i| resource("acta", "project", &i.to_string()))
            .collect();
        let repo = FakeDiscoveryRepo {
            by_user: HashMap::from([(user_id, refs)]),
            by_api_key: HashMap::new(),
        };
        let principal = DiscoveryPrincipal {
            user_id: Some(user_id),
            api_key_id: None,
            is_root: false,
            is_system_admin: false,
        };
        let scopes = repo
            .granted_scopes(&principal)
            .await
            .expect("granted_scopes");

        assert_eq!(
            scopes.for_product("acta").expect("acta present").len(),
            expected_scopes,
            "yielded {yielded}"
        );
        assert_eq!(scopes.truncated, expected_truncated, "yielded {yielded}");
    }
}
