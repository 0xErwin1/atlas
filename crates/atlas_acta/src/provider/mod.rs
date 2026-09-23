//! Acta's own resource provider (PROV-1, ACTA-AUTHZ-1/5): existence and
//! current paths for every Acta resource kind, and the workspace `members`
//! principal set.
//!
//! Paths are built from each stored row's path parent, walked up to its
//! workspace: `workspace/project?/folder*/document|board`, a column or task
//! through its own board (a subtask too, never through its parent task), a
//! comment, attachment or checklist item through its parent, and the
//! workspace-level kinds as `workspace/<kind>`. A resource exists only when
//! every row on that chain is live (not soft-deleted); an archived board
//! stays live. Share links have no rows this release, and an id that is not
//! a canonical (lowercase, hyphenated) UUID never names a row, so both are
//! missing. The catalog is the registry's Acta declaration, handed to the
//! constructor.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use async_trait::async_trait;
use atlas_core::capabilities::{
    CapabilityError, ProviderCatalog, ResourceExistence, ResourceFacts, ResourceProvider,
};
use atlas_core::error::DomainError;
use atlas_core::ids::{PrincipalId, PrincipalSetId, ResourcePath, ResourceRef, canonical_uuid};
use atlas_core::registry::Authorization;
use uuid::Uuid;

/// The product Acta resources belong to.
const ACTA_PRODUCT: &str = "acta";

/// The name of the principal set every workspace declares.
pub const MEMBERS_SET: &str = "members";

/// The longest chain the provider follows. A deeper or cyclic chain never
/// reaches its workspace and is missing.
const MAX_CHAIN_DEPTH: usize = 64;

/// The Acta resource kinds (spec §2), in their singular V2 spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActaKind {
    Workspace,
    Project,
    Folder,
    Document,
    Board,
    Column,
    Task,
    Comment,
    Attachment,
    ChecklistItem,
    SavedSearch,
    TaskView,
    Tag,
    PropertyDefinition,
    StatusTemplate,
    ShareLink,
    Webhook,
    AutomationRule,
    IntegrationConfig,
}

impl ActaKind {
    const ALL: [Self; 19] = [
        Self::Workspace,
        Self::Project,
        Self::Folder,
        Self::Document,
        Self::Board,
        Self::Column,
        Self::Task,
        Self::Comment,
        Self::Attachment,
        Self::ChecklistItem,
        Self::SavedSearch,
        Self::TaskView,
        Self::Tag,
        Self::PropertyDefinition,
        Self::StatusTemplate,
        Self::ShareLink,
        Self::Webhook,
        Self::AutomationRule,
        Self::IntegrationConfig,
    ];

    /// The kind named by a ref's kind segment, if Acta knows it.
    pub fn parse(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|known| known.as_str() == kind)
    }

    /// The kind's segment in refs and paths.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Project => "project",
            Self::Folder => "folder",
            Self::Document => "document",
            Self::Board => "board",
            Self::Column => "column",
            Self::Task => "task",
            Self::Comment => "comment",
            Self::Attachment => "attachment",
            Self::ChecklistItem => "checklist_item",
            Self::SavedSearch => "saved_search",
            Self::TaskView => "task_view",
            Self::Tag => "tag",
            Self::PropertyDefinition => "property_definition",
            Self::StatusTemplate => "status_template",
            Self::ShareLink => "share_link",
            Self::Webhook => "webhook",
            Self::AutomationRule => "automation_rule",
            Self::IntegrationConfig => "integration_config",
        }
    }

    /// Whether resources of this kind are rows a store answers for.
    fn row_backed(self) -> bool {
        self != Self::ShareLink
    }
}

/// One stored row: whether the row itself is live, and the next segment up
/// its path. A workspace has none; any other row without one has no
/// addressable parent and is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActaNode {
    pub id: Uuid,
    pub live: bool,
    pub parent: Option<(ActaKind, Uuid)>,
}

/// Row lookups for the Acta resource kinds.
#[async_trait]
pub trait ActaResourceStore: Send + Sync {
    /// The rows of `kind` among `ids`; an id with no row is absent from the
    /// answer. Never asked for share links.
    async fn nodes(&self, kind: ActaKind, ids: &[Uuid]) -> Result<Vec<ActaNode>, DomainError>;

    /// The users holding a membership (any role) in `workspace`, or `None`
    /// when the workspace does not exist or is not live.
    async fn workspace_members(&self, workspace: Uuid) -> Result<Option<Vec<Uuid>>, DomainError>;
}

/// Acta's [`ResourceProvider`] over an [`ActaResourceStore`].
pub struct ActaResourceProvider<S> {
    store: S,
    catalog: ProviderCatalog,
}

/// A resolved chain segment.
type Segment = (ActaKind, Uuid);

impl<S: ActaResourceStore> ActaResourceProvider<S> {
    /// Builds the provider over `store`, publishing `authorization` (the
    /// registry's Acta declaration) as its catalog.
    pub fn new(store: S, authorization: &Authorization) -> Self {
        Self {
            store,
            catalog: ProviderCatalog {
                resource_kinds: authorization.resource_kinds.clone(),
                actions: authorization.actions.clone(),
                role_definitions: authorization.role_definitions.clone(),
                principal_sets: authorization.principal_sets.clone(),
                role_definitions_v2: Vec::new(),
            },
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    /// Loads every row the requested chains pass through, one store query
    /// per kind per chain level, until each chain reaches its workspace, a
    /// missing row, or [`MAX_CHAIN_DEPTH`].
    async fn load_chains(
        &self,
        requested: &[Option<Segment>],
    ) -> Result<HashMap<Segment, Option<ActaNode>>, CapabilityError> {
        let mut loaded: HashMap<Segment, Option<ActaNode>> = HashMap::new();
        let mut frontier: BTreeSet<Segment> = requested.iter().flatten().copied().collect();

        for _ in 0..MAX_CHAIN_DEPTH {
            let mut by_kind: BTreeMap<ActaKind, Vec<Uuid>> = BTreeMap::new();
            for (kind, id) in &frontier {
                if !loaded.contains_key(&(*kind, *id)) {
                    by_kind.entry(*kind).or_default().push(*id);
                }
            }

            if by_kind.is_empty() {
                break;
            }

            let mut next = BTreeSet::new();

            for (kind, ids) in by_kind {
                let nodes = self
                    .store
                    .nodes(kind, &ids)
                    .await
                    .map_err(|error| CapabilityError::unavailable(error.to_string()))?;

                for id in &ids {
                    loaded.insert((kind, *id), None);
                }

                for node in nodes {
                    if let Some(parent) = node.parent {
                        next.insert(parent);
                    }
                    loaded.insert((kind, node.id), Some(node));
                }
            }

            frontier = next;
        }

        Ok(loaded)
    }

    /// The facts of every resource, in input order.
    async fn facts(
        &self,
        resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        let requested: Vec<Option<Segment>> = resources.iter().map(segment_of).collect();
        let loaded = self.load_chains(&requested).await?;

        Ok(resources
            .iter()
            .zip(requested)
            .map(|(resource, segment)| {
                let path = segment.and_then(|segment| live_path(segment, &loaded));

                ResourceFacts {
                    resource: resource.clone(),
                    existence: if path.is_some() {
                        ResourceExistence::Exists
                    } else {
                        ResourceExistence::Missing
                    },
                    path,
                }
            })
            .collect())
    }

    /// The chain of an existing resource as refs, root first and ending at
    /// the resource itself.
    async fn breadcrumbs(
        &self,
        resource: &ResourceRef,
    ) -> Result<Vec<ResourceRef>, CapabilityError> {
        let facts = self.facts(std::slice::from_ref(resource)).await?;
        let Some(path) = facts.into_iter().find_map(|fact| fact.path) else {
            return Err(CapabilityError::not_found_ref(resource));
        };

        path.segments()
            .map(|segment| ResourceRef::new(ACTA_PRODUCT, segment.kind(), segment.id()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| CapabilityError::invalid(error.to_string()))
    }
}

/// The Acta row a ref names: an Acta product, a known row-backed kind and a
/// canonical row id.
fn segment_of(resource: &ResourceRef) -> Option<Segment> {
    if resource.product() != ACTA_PRODUCT {
        return None;
    }

    let kind = ActaKind::parse(resource.kind()).filter(|kind| kind.row_backed())?;

    canonical_uuid(resource.id()).map(|id| (kind, id))
}

/// The path of `leaf` when every row from it up to its workspace is loaded
/// and live, root first.
fn live_path(leaf: Segment, loaded: &HashMap<Segment, Option<ActaNode>>) -> Option<ResourcePath> {
    let mut chain: Vec<Segment> = Vec::new();
    let mut visited: HashSet<Segment> = HashSet::new();
    let mut current = leaf;

    loop {
        let node = loaded.get(&current).copied().flatten()?;

        if !node.live || !visited.insert(current) {
            return None;
        }

        chain.push(current);

        match node.parent {
            Some(parent) => current = parent,
            None if current.0 == ActaKind::Workspace => break,
            None => return None,
        }
    }

    let segments: Vec<String> = chain
        .iter()
        .rev()
        .map(|(kind, id)| format!("{}::{id}", kind.as_str()))
        .collect();

    format!("{ACTA_PRODUCT}::{}", segments.join("/"))
        .parse()
        .ok()
}

#[async_trait]
impl<S: ActaResourceStore> ResourceProvider for ActaResourceProvider<S> {
    async fn validate_ref(&self, resource: &ResourceRef) -> Result<bool, CapabilityError> {
        let facts = self.facts(std::slice::from_ref(resource)).await?;

        Ok(facts
            .iter()
            .any(|fact| fact.existence == ResourceExistence::Exists))
    }

    async fn path_of(&self, resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        self.breadcrumbs(resource).await
    }

    async fn ancestors(&self, resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        let mut chain = self.breadcrumbs(resource).await?;
        chain.pop();
        chain.reverse();

        Ok(chain)
    }

    /// Answers `acta::workspace::<id>::members`: every user holding a
    /// membership of any role in a live workspace, as canonical UUID text.
    /// Any other set, or a workspace that does not exist, is not found.
    async fn members_of(&self, set: &PrincipalSetId) -> Result<Vec<PrincipalId>, CapabilityError> {
        let scope = set.scope();
        let not_found = || CapabilityError::not_found(set.to_string());

        if scope.product() != ACTA_PRODUCT
            || scope.kind() != ActaKind::Workspace.as_str()
            || set.set() != MEMBERS_SET
        {
            return Err(not_found());
        }

        let workspace = canonical_uuid(scope.id()).ok_or_else(not_found)?;
        let members = self
            .store
            .workspace_members(workspace)
            .await
            .map_err(|error| CapabilityError::unavailable(error.to_string()))?
            .ok_or_else(not_found)?;

        members
            .iter()
            .map(|user| PrincipalId::new(&user.hyphenated().to_string()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| CapabilityError::invalid(error.to_string()))
    }

    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
        Ok(self.catalog.clone())
    }

    /// Answers every resource with one store query per kind per chain
    /// level. A store failure makes the whole answer unavailable.
    async fn resource_facts(
        &self,
        resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        self.facts(resources).await
    }
}
