//! Custos's own resource provider (PROV-1): existence and paths for the
//! Custos resource kinds, answered in one call per batch.
//!
//! Custos resources have no ancestry, so every existing resource's path is
//! the single segment of its own ref. The platform is a singleton named
//! [`PLATFORM_ID`]; audit entries and share-link credentials have no rows
//! this release and are always missing. A kind Custos does not know, a ref
//! of another product and an id that is not a canonical (lowercase,
//! hyphenated) row id are missing, never an error. The catalog is the registry's Custos declaration, handed to the
//! constructor, so no second copy of it exists.

use std::collections::{BTreeMap, HashSet};

use crate::eval::CUSTOS_PRODUCT;
use async_trait::async_trait;
use atlas_core::capabilities::{
    CapabilityError, ProviderCatalog, ResourceExistence, ResourceFacts, ResourceProvider,
};
use atlas_core::error::DomainError;
use atlas_core::ids::{PrincipalId, PrincipalSetId, ResourcePath, ResourceRef, canonical_uuid};
use atlas_core::registry::Authorization;
use uuid::Uuid;

/// The id of the singleton platform resource.
pub const PLATFORM_ID: &str = "atlas";

/// The Custos resource kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CustosKind {
    User,
    Agent,
    Group,
    Role,
    Grant,
    Deny,
    Session,
    PersonalApiKey,
    AgentApiKey,
    Platform,
    Audit,
    ShareLinkCredential,
}

impl CustosKind {
    /// The kind named by a ref's kind segment, if Custos knows it.
    pub fn parse(kind: &str) -> Option<Self> {
        let parsed = match kind {
            "user" => Self::User,
            "agent" => Self::Agent,
            "group" => Self::Group,
            "role" => Self::Role,
            "grant" => Self::Grant,
            "deny" => Self::Deny,
            "session" => Self::Session,
            "personal_api_key" => Self::PersonalApiKey,
            "agent_api_key" => Self::AgentApiKey,
            "platform" => Self::Platform,
            "audit" => Self::Audit,
            "share_link_credential" => Self::ShareLinkCredential,
            _ => return None,
        };

        Some(parsed)
    }

    /// Whether resources of this kind are rows a store answers for.
    fn row_backed(self) -> bool {
        !matches!(
            self,
            Self::Platform | Self::Audit | Self::ShareLinkCredential
        )
    }
}

/// Existence lookups for the row-backed Custos kinds.
#[async_trait]
pub trait CustosResourceStore: Send + Sync {
    /// The ids among `ids` that name an existing resource of `kind`. Only
    /// row-backed kinds are asked, once per kind per call.
    async fn existing(&self, kind: CustosKind, ids: &[Uuid]) -> Result<HashSet<Uuid>, DomainError>;
}

/// Custos's [`ResourceProvider`] over a [`CustosResourceStore`].
pub struct CustosResourceProvider<S> {
    store: S,
    catalog: ProviderCatalog,
}

impl<S: CustosResourceStore> CustosResourceProvider<S> {
    /// Builds the provider over `store`, publishing `authorization` (the
    /// registry's Custos declaration) as its catalog.
    pub fn new(store: S, authorization: &Authorization) -> Self {
        Self {
            store,
            catalog: ProviderCatalog::from(authorization),
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    async fn exists(&self, resource: &ResourceRef) -> Result<bool, CapabilityError> {
        let facts = self.resource_facts(std::slice::from_ref(resource)).await?;

        Ok(facts
            .iter()
            .any(|fact| fact.existence == ResourceExistence::Exists))
    }
}

#[async_trait]
impl<S: CustosResourceStore> ResourceProvider for CustosResourceProvider<S> {
    async fn validate_ref(&self, resource: &ResourceRef) -> Result<bool, CapabilityError> {
        self.exists(resource).await
    }

    async fn path_of(&self, resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        if self.exists(resource).await? {
            Ok(vec![resource.clone()])
        } else {
            Err(CapabilityError::not_found_ref(resource))
        }
    }

    async fn ancestors(&self, resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        if self.exists(resource).await? {
            Ok(Vec::new())
        } else {
            Err(CapabilityError::not_found_ref(resource))
        }
    }

    /// Custos declares no principal sets.
    async fn members_of(&self, set: &PrincipalSetId) -> Result<Vec<PrincipalId>, CapabilityError> {
        Err(CapabilityError::not_found(set.to_string()))
    }

    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
        Ok(self.catalog.clone())
    }

    /// Answers every resource with one store query per row-backed kind
    /// present. A store failure makes the whole answer unavailable.
    async fn resource_facts(
        &self,
        resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        let mut found = vec![false; resources.len()];
        let mut by_kind: BTreeMap<CustosKind, Vec<(usize, Uuid)>> = BTreeMap::new();

        for (index, resource) in resources.iter().enumerate() {
            if resource.product() != CUSTOS_PRODUCT {
                continue;
            }

            let Some(kind) = CustosKind::parse(resource.kind()) else {
                continue;
            };

            if kind == CustosKind::Platform {
                if let Some(slot) = found.get_mut(index) {
                    *slot = resource.id() == PLATFORM_ID;
                }
            } else if kind.row_backed()
                && let Some(id) = canonical_uuid(resource.id())
            {
                by_kind.entry(kind).or_default().push((index, id));
            }
        }

        for (kind, entries) in by_kind {
            let ids: Vec<Uuid> = entries.iter().map(|(_, id)| *id).collect();
            let existing = self
                .store
                .existing(kind, &ids)
                .await
                .map_err(|error| CapabilityError::unavailable(error.to_string()))?;

            for (index, id) in entries {
                if let Some(slot) = found.get_mut(index) {
                    *slot = existing.contains(&id);
                }
            }
        }

        Ok(resources
            .iter()
            .zip(found)
            .map(|(resource, exists)| ResourceFacts {
                resource: resource.clone(),
                existence: if exists {
                    ResourceExistence::Exists
                } else {
                    ResourceExistence::Missing
                },
                path: exists.then(|| ResourcePath::from(resource.clone())),
            })
            .collect())
    }
}
