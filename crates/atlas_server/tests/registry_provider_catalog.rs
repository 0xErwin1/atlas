//! SHELL-REG-4 for the V2 catalog (`v2-e7-s1a-acta-catalog`): what a
//! product declares in its registry `Authorization` is exactly what its
//! `ResourceProvider` publishes as `ProviderCatalog`, and the declaration
//! itself is well formed (singular kinds, actions of declared kinds beside
//! the plural V1 families, versioned roles whose names are also in
//! `role_definitions`, role actions drawn from the product's own catalog or
//! the Custos delegation vocabulary).
//!
//! The provider side is generic over a `(product, provider)` list: Custos
//! today; Acta joins once its provider registers (`v2-e7-s1b`), by adding
//! one constructor to [`providers_under_test`].

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use atlas_core::capabilities::ResourceProvider;
use atlas_core::error::DomainError;
use atlas_core::ids::ActionId;
use atlas_core::registry::{ComponentEntry, ComponentId, Registry, build};
use atlas_custos::provider::{CustosKind, CustosResourceProvider, CustosResourceStore};
use atlas_server::authz::v2_service::{product_specs, validation_catalog};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use uuid::Uuid;

/// The catalog never touches the store, so the provider under test needs no
/// database.
struct NoStore;

#[async_trait]
impl CustosResourceStore for NoStore {
    async fn existing(
        &self,
        _kind: CustosKind,
        _ids: &[Uuid],
    ) -> Result<HashSet<Uuid>, DomainError> {
        Ok(HashSet::new())
    }
}

fn registry() -> Registry {
    build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator")
}

fn entry<'a>(registry: &'a Registry, product: &str) -> &'a ComponentEntry {
    registry
        .get(&ComponentId::new(product).expect("valid component id"))
        .unwrap_or_else(|| panic!("{product} is a REG-5 component"))
}

/// Every provider the server composes, built from the same registry
/// declaration the server passes it.
fn providers_under_test(registry: &Registry) -> Vec<(&'static str, Arc<dyn ResourceProvider>)> {
    let custos = entry(registry, "custos");

    vec![(
        "custos",
        Arc::new(CustosResourceProvider::new(NoStore, &custos.authorization)),
    )]
}

/// The delegation actions a product's built-in role may carry beside its
/// own actions (D-S5a-1).
const DELEGATION_ACTIONS: &[&str] = &[
    "custos::grant::create",
    "custos::grant::delete",
    "custos::group::create",
    "custos::group::update",
    "custos::group::delete",
    "custos::group::add_member",
    "custos::group::remove_member",
];

#[tokio::test]
async fn every_provider_publishes_exactly_its_registry_declaration() {
    let registry = registry();
    let providers = providers_under_test(&registry);
    assert!(
        !providers.is_empty(),
        "the cross-check must examine at least one provider"
    );

    for (product, provider) in providers {
        let declared = &entry(&registry, product).authorization;
        assert!(declared.provider, "{product} declares itself a provider");

        let published = provider
            .catalog()
            .await
            .unwrap_or_else(|e| panic!("{product}: provider catalog failed: {e}"));

        assert_eq!(
            published.resource_kinds, declared.resource_kinds,
            "{product}: kinds"
        );
        assert_eq!(published.actions, declared.actions, "{product}: actions");
        assert_eq!(
            published.role_definitions, declared.role_definitions,
            "{product}: role names"
        );
        assert_eq!(
            published.principal_sets, declared.principal_sets,
            "{product}: principal sets"
        );

        let published_roles: Vec<(String, u32, Vec<ActionId>)> = published
            .role_definitions_v2
            .iter()
            .map(|role| (role.name.clone(), role.version, role.actions.clone()))
            .collect();
        let declared_roles: Vec<(String, u32, Vec<ActionId>)> = declared
            .role_definitions_v2
            .iter()
            .map(|role| (role.name.clone(), role.version, role.actions.clone()))
            .collect();
        assert_eq!(
            published_roles, declared_roles,
            "{product}: versioned roles"
        );
    }
}

#[test]
fn every_published_declaration_is_well_formed() {
    let registry = registry();
    let published: Vec<&ComponentEntry> = registry
        .entries()
        .iter()
        .filter(|entry| !entry.authorization.resource_kinds.is_empty())
        .collect();
    let products: Vec<&str> = published
        .iter()
        .map(|entry| entry.identity.stable_id.as_str())
        .collect();
    assert_eq!(
        products,
        ["custos", "acta"],
        "the products with a V2 catalog"
    );

    for entry in published {
        let product = entry.identity.stable_id.as_str();
        let declaration = &entry.authorization;
        let kinds: HashSet<&str> = declaration
            .resource_kinds
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(
            kinds.len(),
            declaration.resource_kinds.len(),
            "{product}: kinds are unique"
        );
        for kind in &kinds {
            assert!(
                !kind.ends_with('s'),
                "{product}: kind `{kind}` must be singular"
            );
        }

        let actions: HashSet<&ActionId> = declaration.actions.iter().collect();
        assert_eq!(
            actions.len(),
            declaration.actions.len(),
            "{product}: actions are unique"
        );
        let catalog_actions: HashSet<&ActionId> = declaration
            .actions
            .iter()
            .filter(|action| kinds.contains(action.kind()))
            .collect();
        assert!(
            !catalog_actions.is_empty(),
            "{product}: at least one action of a declared kind"
        );
        for action in &declaration.actions {
            assert_eq!(
                action.product(),
                product,
                "{product}: action `{action}` product"
            );
        }

        let names: HashSet<&str> = declaration
            .role_definitions
            .iter()
            .map(String::as_str)
            .collect();
        let versioned: HashSet<&str> = declaration
            .role_definitions_v2
            .iter()
            .map(|role| role.name.as_str())
            .collect();
        assert_eq!(
            names, versioned,
            "{product}: every versioned role is named, and vice versa"
        );

        for role in &declaration.role_definitions_v2 {
            assert!(
                !role.actions.is_empty(),
                "{product}: role {} is not empty",
                role.name
            );
            let unique: HashSet<&ActionId> = role.actions.iter().collect();
            assert_eq!(
                unique.len(),
                role.actions.len(),
                "{product}: role {} has no duplicate",
                role.name
            );
            for action in &role.actions {
                let own = catalog_actions.contains(action);
                let delegation = DELEGATION_ACTIONS.contains(&action.to_string().as_str());
                assert!(
                    own || delegation,
                    "{product}: role {} carries `{action}`, which is neither a catalog action of \
                     {product} nor a Custos delegation action",
                    role.name
                );
            }
        }
    }
}

#[test]
fn the_validation_catalog_resolves_every_declared_role_and_only_singular_actions() {
    let registry = registry();
    let catalog = validation_catalog(&registry).expect("the declarations form a catalog");

    for spec in product_specs(&registry) {
        for role in &spec.roles {
            assert!(
                catalog
                    .builtin_role(&spec.product, &role.name, role.version)
                    .is_some(),
                "{}: {}@{} resolves",
                spec.product,
                role.name,
                role.version
            );
        }
        for action in &spec.actions {
            assert!(
                spec.kinds.contains(&action.kind().to_string()),
                "{}: `{action}` entered the catalog without a declared kind",
                spec.product
            );
        }
    }

    let editor = catalog
        .builtin_role("acta", "editor", 1)
        .expect("acta declares editor@1");
    assert!(
        editor
            .actions()
            .contains(&"custos::grant::create".parse().unwrap())
    );
    assert!(
        editor
            .actions()
            .contains(&"acta::document::update_content".parse().unwrap())
    );
    assert!(
        !editor
            .actions()
            .contains(&"acta::workspace::delete".parse().unwrap())
    );

    let admin = catalog
        .builtin_role("acta", "admin", 1)
        .expect("acta declares admin@1");
    assert!(
        admin
            .actions()
            .contains(&"custos::grant::delete".parse().unwrap())
    );
    assert!(
        admin
            .actions()
            .contains(&"acta::project::purge".parse().unwrap())
    );
    assert!(
        !admin
            .actions()
            .contains(&"acta::workspace::transfer".parse().unwrap())
    );

    assert!(
        catalog.builtin_role("custos", "admin", 1).is_none(),
        "custos declares no built-in roles"
    );
}
