//! Behavioral tests for Custos's own resource provider (E5 S6a, PROV-1):
//! existence and single-segment paths for every Custos resource kind in one
//! call, the singleton platform, kinds without rows, and the catalog taken
//! from the registry declaration.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use atlas_core::capabilities::{
    CapabilityError, ResourceExistence, ResourceFacts, ResourceProvider,
};
use atlas_core::error::DomainError;
use atlas_core::ids::{ResourcePath, ResourceRef};
use atlas_core::registry::Authorization;
use atlas_custos::provider::{CustosKind, CustosResourceProvider, CustosResourceStore};
use uuid::Uuid;

const ROW_KINDS: [(&str, CustosKind); 9] = [
    ("user", CustosKind::User),
    ("agent", CustosKind::Agent),
    ("group", CustosKind::Group),
    ("role", CustosKind::Role),
    ("grant", CustosKind::Grant),
    ("deny", CustosKind::Deny),
    ("session", CustosKind::Session),
    ("personal_api_key", CustosKind::PersonalApiKey),
    ("agent_api_key", CustosKind::AgentApiKey),
];

/// A store answering from fixed rows per kind, recording each query.
#[derive(Default)]
struct FakeStore {
    rows: HashMap<CustosKind, HashSet<Uuid>>,
    fail: bool,
    queries: Mutex<Vec<(CustosKind, usize)>>,
}

#[async_trait]
impl CustosResourceStore for FakeStore {
    async fn existing(&self, kind: CustosKind, ids: &[Uuid]) -> Result<HashSet<Uuid>, DomainError> {
        self.queries.lock().unwrap().push((kind, ids.len()));

        if self.fail {
            return Err(DomainError::Internal {
                message: "store down".to_string(),
            });
        }

        let rows = self.rows.get(&kind).cloned().unwrap_or_default();

        Ok(ids.iter().copied().filter(|id| rows.contains(id)).collect())
    }
}

fn authorization() -> Authorization {
    Authorization {
        resource_kinds: vec!["user".to_string(), "platform".to_string()],
        actions: vec!["custos::user::read".parse().unwrap()],
        role_definitions: Vec::new(),
        principal_sets: Vec::new(),
        provider: true,
    }
}

fn provider(store: FakeStore) -> CustosResourceProvider<FakeStore> {
    CustosResourceProvider::new(store, &authorization())
}

fn custos(kind: &str, id: &str) -> ResourceRef {
    format!("custos::{kind}::{id}").parse().unwrap()
}

fn existing(resource: &ResourceRef) -> ResourceFacts {
    ResourceFacts {
        resource: resource.clone(),
        existence: ResourceExistence::Exists,
        path: Some(ResourcePath::from(resource.clone())),
    }
}

fn missing(resource: &ResourceRef) -> ResourceFacts {
    ResourceFacts {
        resource: resource.clone(),
        existence: ResourceExistence::Missing,
        path: None,
    }
}

#[test]
fn every_custos_kind_parses_and_unknown_kinds_do_not() {
    for (raw, kind) in ROW_KINDS {
        assert_eq!(CustosKind::parse(raw), Some(kind));
    }

    assert_eq!(CustosKind::parse("platform"), Some(CustosKind::Platform));
    assert_eq!(CustosKind::parse("audit"), Some(CustosKind::Audit));
    assert_eq!(
        CustosKind::parse("share_link_credential"),
        Some(CustosKind::ShareLinkCredential)
    );
    assert_eq!(CustosKind::parse("document"), None);
}

#[tokio::test]
async fn every_row_backed_kind_reports_existence_with_a_single_segment_path() {
    let mut store = FakeStore::default();
    let mut resources = Vec::new();
    let mut expected = Vec::new();

    for (raw, kind) in ROW_KINDS {
        let present = Uuid::now_v7();
        let absent = Uuid::now_v7();
        store.rows.insert(kind, HashSet::from([present]));

        let present = custos(raw, &present.to_string());
        let absent = custos(raw, &absent.to_string());
        expected.push(existing(&present));
        expected.push(missing(&absent));
        resources.push(present);
        resources.push(absent);
    }

    let provider = provider(store);
    let facts = provider.resource_facts(&resources).await.unwrap();

    assert_eq!(facts, expected);
}

#[tokio::test]
async fn a_mixed_batch_queries_each_row_kind_once() {
    let user = Uuid::now_v7();
    let group = Uuid::now_v7();
    let mut store = FakeStore::default();
    store.rows.insert(CustosKind::User, HashSet::from([user]));
    store.rows.insert(CustosKind::Group, HashSet::from([group]));
    let provider = provider(store);
    let resources = [
        custos("user", &user.to_string()),
        custos("group", &group.to_string()),
        custos("user", &Uuid::now_v7().to_string()),
        custos("platform", "atlas"),
        custos("group", &Uuid::now_v7().to_string()),
    ];

    let facts = provider.resource_facts(&resources).await.unwrap();

    assert_eq!(
        facts.iter().map(|fact| fact.existence).collect::<Vec<_>>(),
        vec![
            ResourceExistence::Exists,
            ResourceExistence::Exists,
            ResourceExistence::Missing,
            ResourceExistence::Exists,
            ResourceExistence::Missing,
        ]
    );
    let mut queries = provider_queries(&provider);
    queries.sort();
    assert_eq!(queries, vec![(CustosKind::User, 2), (CustosKind::Group, 2)]);
}

#[tokio::test]
async fn the_platform_is_a_singleton_and_rowless_kinds_are_missing() {
    let provider = provider(FakeStore::default());
    let resources = [
        custos("platform", "atlas"),
        custos("platform", "other"),
        custos("audit", &Uuid::now_v7().to_string()),
        custos("share_link_credential", &Uuid::now_v7().to_string()),
    ];

    let facts = provider.resource_facts(&resources).await.unwrap();

    assert_eq!(
        facts,
        vec![
            existing(&resources[0]),
            missing(&resources[1]),
            missing(&resources[2]),
            missing(&resources[3]),
        ]
    );
    assert!(provider_queries(&provider).is_empty());
}

#[tokio::test]
async fn unknown_kinds_foreign_products_and_malformed_ids_are_missing_not_errors() {
    let provider = provider(FakeStore::default());
    let resources: [ResourceRef; 3] = [
        custos("document", &Uuid::now_v7().to_string()),
        format!("acta::user::{}", Uuid::now_v7()).parse().unwrap(),
        custos("user", "not-a-uuid"),
    ];

    let facts = provider.resource_facts(&resources).await.unwrap();

    assert_eq!(facts, resources.iter().map(missing).collect::<Vec<_>>());
    assert!(provider_queries(&provider).is_empty());
}

#[tokio::test]
async fn a_store_failure_is_unavailable_not_missing() {
    let provider = provider(FakeStore {
        fail: true,
        ..FakeStore::default()
    });

    let error = provider
        .resource_facts(&[custos("user", &Uuid::now_v7().to_string())])
        .await
        .unwrap_err();

    assert!(matches!(error, CapabilityError::Unavailable { .. }));
}

#[tokio::test]
async fn the_legacy_methods_answer_through_the_same_facts() {
    let user = Uuid::now_v7();
    let mut store = FakeStore::default();
    store.rows.insert(CustosKind::User, HashSet::from([user]));
    let provider = provider(store);
    let present = custos("user", &user.to_string());
    let absent = custos("user", &Uuid::now_v7().to_string());

    assert!(provider.validate_ref(&present).await.unwrap());
    assert!(!provider.validate_ref(&absent).await.unwrap());
    assert_eq!(
        provider.path_of(&present).await.unwrap(),
        vec![present.clone()]
    );
    assert!(provider.ancestors(&present).await.unwrap().is_empty());
    assert!(matches!(
        provider.path_of(&absent).await,
        Err(CapabilityError::NotFound { .. })
    ));
    assert!(matches!(
        provider
            .members_of(&"custos::platform::atlas::admins".parse().unwrap())
            .await,
        Err(CapabilityError::NotFound { .. })
    ));
}

#[tokio::test]
async fn the_catalog_is_the_registry_declaration() {
    let provider = provider(FakeStore::default());
    let declared = authorization();

    let catalog = provider.catalog().await.unwrap();

    assert_eq!(catalog.resource_kinds, declared.resource_kinds);
    assert_eq!(catalog.actions, declared.actions);
    assert_eq!(catalog.role_definitions, declared.role_definitions);
    assert_eq!(catalog.principal_sets, declared.principal_sets);
    assert!(catalog.role_definitions_v2.is_empty());
}

fn provider_queries(provider: &CustosResourceProvider<FakeStore>) -> Vec<(CustosKind, usize)> {
    provider.store().queries.lock().unwrap().clone()
}

/// Every non-canonical spelling `Uuid::parse_str` accepts for `id`.
fn aliases(id: Uuid) -> [String; 3] {
    [
        id.hyphenated().to_string().to_uppercase(),
        id.simple().to_string(),
        id.braced().to_string(),
    ]
}

#[tokio::test]
async fn only_the_canonical_lowercase_hyphenated_id_names_a_row() {
    let user = Uuid::now_v7();
    let group = Uuid::now_v7();
    let mut store = FakeStore::default();
    store.rows.insert(CustosKind::User, HashSet::from([user]));
    store.rows.insert(CustosKind::Group, HashSet::from([group]));
    let provider = provider(store);

    for (kind, id) in [("user", user), ("group", group)] {
        let canonical = custos(kind, &id.to_string());
        let spelled: Vec<ResourceRef> = aliases(id)
            .iter()
            .map(|alias| custos(kind, alias))
            .collect();

        let facts = provider.resource_facts(&spelled).await.unwrap();

        assert_eq!(
            facts,
            spelled.iter().map(missing).collect::<Vec<_>>(),
            "{kind}"
        );
        assert_eq!(
            provider
                .resource_facts(std::slice::from_ref(&canonical))
                .await
                .unwrap(),
            vec![existing(&canonical)]
        );
    }
}
