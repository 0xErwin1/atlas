//! Behavioral tests for Acta's resource provider (PROV-1, ACTA-AUTHZ-1/5)
//! over a fake store: paths built from each row's path parent, existence
//! only when the whole chain is live, canonical ids, `members_of` for the
//! workspace `members` set, and the catalog taken from the registry.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use atlas_acta::provider::{ActaKind, ActaNode, ActaResourceProvider, ActaResourceStore};
use atlas_core::capabilities::{
    CapabilityError, ResourceExistence, ResourceFacts, ResourceProvider,
};
use atlas_core::error::DomainError;
use atlas_core::ids::{ResourcePath, ResourceRef};
use atlas_core::registry::Authorization;
use uuid::Uuid;

const KINDS: [(&str, ActaKind); 19] = [
    ("workspace", ActaKind::Workspace),
    ("project", ActaKind::Project),
    ("folder", ActaKind::Folder),
    ("document", ActaKind::Document),
    ("board", ActaKind::Board),
    ("column", ActaKind::Column),
    ("task", ActaKind::Task),
    ("comment", ActaKind::Comment),
    ("attachment", ActaKind::Attachment),
    ("checklist_item", ActaKind::ChecklistItem),
    ("saved_search", ActaKind::SavedSearch),
    ("task_view", ActaKind::TaskView),
    ("tag", ActaKind::Tag),
    ("property_definition", ActaKind::PropertyDefinition),
    ("status_template", ActaKind::StatusTemplate),
    ("share_link", ActaKind::ShareLink),
    ("webhook", ActaKind::Webhook),
    ("automation_rule", ActaKind::AutomationRule),
    ("integration_config", ActaKind::IntegrationConfig),
];

/// A store answering from fixed rows, recording each query.
#[derive(Default)]
struct FakeStore {
    rows: HashMap<(ActaKind, Uuid), ActaNode>,
    members: HashMap<Uuid, Vec<Uuid>>,
    fail: bool,
    queries: Mutex<Vec<(ActaKind, usize)>>,
}

impl FakeStore {
    /// Adds a live row of `kind` under `parent` and returns its id.
    fn add(&mut self, kind: ActaKind, parent: Option<(ActaKind, Uuid)>) -> Uuid {
        let id = Uuid::now_v7();
        self.rows.insert(
            (kind, id),
            ActaNode {
                id,
                live: true,
                parent,
            },
        );
        id
    }

    fn trash(&mut self, kind: ActaKind, id: Uuid) {
        self.rows.get_mut(&(kind, id)).expect("seeded row").live = false;
    }

    fn reparent(&mut self, kind: ActaKind, id: Uuid, parent: Option<(ActaKind, Uuid)>) {
        self.rows.get_mut(&(kind, id)).expect("seeded row").parent = parent;
    }
}

#[async_trait]
impl ActaResourceStore for FakeStore {
    async fn nodes(&self, kind: ActaKind, ids: &[Uuid]) -> Result<Vec<ActaNode>, DomainError> {
        self.queries.lock().unwrap().push((kind, ids.len()));

        if self.fail {
            return Err(DomainError::Internal {
                message: "store down".to_string(),
            });
        }

        Ok(ids
            .iter()
            .filter_map(|id| self.rows.get(&(kind, *id)).copied())
            .collect())
    }

    async fn workspace_members(&self, workspace: Uuid) -> Result<Option<Vec<Uuid>>, DomainError> {
        if self.fail {
            return Err(DomainError::Internal {
                message: "store down".to_string(),
            });
        }

        Ok(self.members.get(&workspace).cloned())
    }
}

fn authorization() -> Authorization {
    Authorization {
        resource_kinds: vec!["document".to_string(), "workspace".to_string()],
        actions: vec!["acta::document::read".parse().unwrap()],
        role_definitions: vec!["viewer".to_string()],
        principal_sets: vec!["members".to_string()],
        provider: true,
    }
}

fn provider(store: FakeStore) -> ActaResourceProvider<FakeStore> {
    ActaResourceProvider::new(store, &authorization())
}

fn acta(kind: ActaKind, id: Uuid) -> ResourceRef {
    format!("acta::{}::{id}", kind.as_str()).parse().unwrap()
}

fn path(segments: &[(ActaKind, Uuid)]) -> ResourcePath {
    let joined: Vec<String> = segments
        .iter()
        .map(|(kind, id)| format!("{}::{id}", kind.as_str()))
        .collect();

    format!("acta::{}", joined.join("/")).parse().unwrap()
}

fn existing(resource: &ResourceRef, segments: &[(ActaKind, Uuid)]) -> ResourceFacts {
    ResourceFacts {
        resource: resource.clone(),
        existence: ResourceExistence::Exists,
        path: Some(path(segments)),
    }
}

fn missing(resource: &ResourceRef) -> ResourceFacts {
    ResourceFacts {
        resource: resource.clone(),
        existence: ResourceExistence::Missing,
        path: None,
    }
}

async fn facts_of(
    provider: &ActaResourceProvider<FakeStore>,
    resource: &ResourceRef,
) -> ResourceFacts {
    provider
        .resource_facts(std::slice::from_ref(resource))
        .await
        .unwrap()
        .remove(0)
}

fn queries(provider: &ActaResourceProvider<FakeStore>) -> Vec<(ActaKind, usize)> {
    provider.store().queries.lock().unwrap().clone()
}

/// A workspace with a project, two nested folders and a document in the
/// inner folder.
struct Tree {
    store: FakeStore,
    workspace: Uuid,
    project: Uuid,
    outer: Uuid,
    inner: Uuid,
    document: Uuid,
}

fn tree() -> Tree {
    let mut store = FakeStore::default();
    let workspace = store.add(ActaKind::Workspace, None);
    let project = store.add(ActaKind::Project, Some((ActaKind::Workspace, workspace)));
    let outer = store.add(ActaKind::Folder, Some((ActaKind::Project, project)));
    let inner = store.add(ActaKind::Folder, Some((ActaKind::Folder, outer)));
    let document = store.add(ActaKind::Document, Some((ActaKind::Folder, inner)));

    Tree {
        store,
        workspace,
        project,
        outer,
        inner,
        document,
    }
}

#[test]
fn every_acta_kind_parses_from_its_singular_name() {
    for (raw, kind) in KINDS {
        assert_eq!(ActaKind::parse(raw), Some(kind), "{raw}");
        assert_eq!(kind.as_str(), raw);
    }

    assert_eq!(ActaKind::parse("doc"), None);
    assert_eq!(ActaKind::parse("documents"), None);
}

#[tokio::test]
async fn a_document_path_runs_from_the_workspace_through_project_and_folders() {
    let tree = tree();
    let expected = [
        (ActaKind::Workspace, tree.workspace),
        (ActaKind::Project, tree.project),
        (ActaKind::Folder, tree.outer),
        (ActaKind::Folder, tree.inner),
        (ActaKind::Document, tree.document),
    ];
    let document = acta(ActaKind::Document, tree.document);
    let provider = provider(tree.store);

    assert_eq!(
        facts_of(&provider, &document).await,
        existing(&document, &expected)
    );
}

#[tokio::test]
async fn a_resource_exists_only_while_its_whole_chain_is_live() {
    for trashed in [
        ActaKind::Workspace,
        ActaKind::Project,
        ActaKind::Folder,
        ActaKind::Document,
    ] {
        let mut tree = tree();
        let id = match trashed {
            ActaKind::Workspace => tree.workspace,
            ActaKind::Project => tree.project,
            ActaKind::Folder => tree.outer,
            _ => tree.document,
        };
        tree.store.trash(trashed, id);
        let document = acta(ActaKind::Document, tree.document);
        let provider = provider(tree.store);

        assert_eq!(
            facts_of(&provider, &document).await,
            missing(&document),
            "{trashed:?}"
        );
    }
}

#[tokio::test]
async fn tasks_and_subtasks_run_through_their_own_board() {
    let mut store = FakeStore::default();
    let workspace = store.add(ActaKind::Workspace, None);
    let project = store.add(ActaKind::Project, Some((ActaKind::Workspace, workspace)));
    let board = store.add(ActaKind::Board, Some((ActaKind::Project, project)));
    let column = store.add(ActaKind::Column, Some((ActaKind::Board, board)));
    let task = store.add(ActaKind::Task, Some((ActaKind::Board, board)));
    let subtask = store.add(ActaKind::Task, Some((ActaKind::Board, board)));
    let board_path = [
        (ActaKind::Workspace, workspace),
        (ActaKind::Project, project),
        (ActaKind::Board, board),
    ];
    let provider = provider(store);

    for (kind, id) in [
        (ActaKind::Column, column),
        (ActaKind::Task, task),
        (ActaKind::Task, subtask),
    ] {
        let resource = acta(kind, id);
        let mut expected = board_path.to_vec();
        expected.push((kind, id));

        assert_eq!(
            facts_of(&provider, &resource).await,
            existing(&resource, &expected)
        );
    }
}

#[tokio::test]
async fn comments_attachments_and_checklist_items_run_through_their_parent() {
    let mut store = FakeStore::default();
    let workspace = store.add(ActaKind::Workspace, None);
    let document = store.add(ActaKind::Document, Some((ActaKind::Workspace, workspace)));
    let project = store.add(ActaKind::Project, Some((ActaKind::Workspace, workspace)));
    let board = store.add(ActaKind::Board, Some((ActaKind::Project, project)));
    let task = store.add(ActaKind::Task, Some((ActaKind::Board, board)));
    let document_comment = store.add(ActaKind::Comment, Some((ActaKind::Document, document)));
    let task_comment = store.add(ActaKind::Comment, Some((ActaKind::Task, task)));
    let comment_attachment = store.add(
        ActaKind::Attachment,
        Some((ActaKind::Comment, task_comment)),
    );
    let document_attachment = store.add(ActaKind::Attachment, Some((ActaKind::Document, document)));
    let checklist = store.add(ActaKind::ChecklistItem, Some((ActaKind::Task, task)));
    let document_path = vec![
        (ActaKind::Workspace, workspace),
        (ActaKind::Document, document),
    ];
    let task_path = vec![
        (ActaKind::Workspace, workspace),
        (ActaKind::Project, project),
        (ActaKind::Board, board),
        (ActaKind::Task, task),
    ];
    let provider = provider(store);

    let cases = [
        (ActaKind::Comment, document_comment, document_path.clone()),
        (ActaKind::Comment, task_comment, task_path.clone()),
        (
            ActaKind::Attachment,
            comment_attachment,
            [task_path.clone(), vec![(ActaKind::Comment, task_comment)]].concat(),
        ),
        (ActaKind::Attachment, document_attachment, document_path),
        (ActaKind::ChecklistItem, checklist, task_path),
    ];

    for (kind, id, parent_path) in cases {
        let resource = acta(kind, id);
        let expected = [parent_path, vec![(kind, id)]].concat();

        assert_eq!(
            facts_of(&provider, &resource).await,
            existing(&resource, &expected)
        );
    }
}

#[tokio::test]
async fn workspace_level_kinds_are_direct_workspace_children() {
    let mut store = FakeStore::default();
    let workspace = store.add(ActaKind::Workspace, None);
    let children = [
        ActaKind::SavedSearch,
        ActaKind::TaskView,
        ActaKind::Tag,
        ActaKind::PropertyDefinition,
        ActaKind::StatusTemplate,
        ActaKind::Webhook,
        ActaKind::AutomationRule,
        ActaKind::IntegrationConfig,
    ]
    .map(|kind| {
        (
            kind,
            store.add(kind, Some((ActaKind::Workspace, workspace))),
        )
    });
    let provider = provider(store);

    for (kind, id) in children {
        let resource = acta(kind, id);

        assert_eq!(
            facts_of(&provider, &resource).await,
            existing(&resource, &[(ActaKind::Workspace, workspace), (kind, id)]),
            "{kind:?}"
        );
    }
}

#[tokio::test]
async fn share_links_aliases_foreign_products_and_unknown_kinds_are_missing_without_a_query() {
    let tree = tree();
    let canonical = tree.document;
    let provider = provider(tree.store);
    let resources: Vec<ResourceRef> = [
        format!("acta::share_link::{}", Uuid::now_v7()),
        format!(
            "acta::document::{}",
            canonical.hyphenated().to_string().to_uppercase()
        ),
        format!("acta::document::{}", canonical.simple()),
        format!("acta::document::{}", canonical.braced()),
        "acta::document::not-a-uuid".to_string(),
        format!("custos::document::{canonical}"),
        format!("acta::doc::{canonical}"),
    ]
    .iter()
    .map(|raw| raw.parse().unwrap())
    .collect();

    let facts = provider.resource_facts(&resources).await.unwrap();

    assert_eq!(facts, resources.iter().map(missing).collect::<Vec<_>>());
    assert!(queries(&provider).is_empty());
}

#[tokio::test]
async fn a_mixed_batch_answers_in_input_order_querying_each_kind_once_per_level() {
    let mut store = FakeStore::default();
    let workspace = store.add(ActaKind::Workspace, None);
    let first = store.add(ActaKind::Document, Some((ActaKind::Workspace, workspace)));
    let second = store.add(ActaKind::Document, Some((ActaKind::Workspace, workspace)));
    let tag = store.add(ActaKind::Tag, Some((ActaKind::Workspace, workspace)));
    let provider = provider(store);
    let resources = [
        acta(ActaKind::Tag, tag),
        acta(ActaKind::Document, Uuid::now_v7()),
        acta(ActaKind::Document, first),
        acta(ActaKind::Workspace, workspace),
        acta(ActaKind::Document, second),
    ];

    let facts = provider.resource_facts(&resources).await.unwrap();

    assert_eq!(
        facts.iter().map(|fact| fact.existence).collect::<Vec<_>>(),
        vec![
            ResourceExistence::Exists,
            ResourceExistence::Missing,
            ResourceExistence::Exists,
            ResourceExistence::Exists,
            ResourceExistence::Exists,
        ]
    );
    let mut asked = queries(&provider);
    asked.sort();
    assert_eq!(
        asked,
        vec![
            (ActaKind::Workspace, 1),
            (ActaKind::Document, 3),
            (ActaKind::Tag, 1),
        ]
    );
}

#[tokio::test]
async fn broken_or_cyclic_chains_are_missing() {
    let mut tree = tree();
    let orphan_parent = Uuid::now_v7();
    let orphan = tree
        .store
        .add(ActaKind::Document, Some((ActaKind::Folder, orphan_parent)));
    let parentless = tree.store.add(ActaKind::Attachment, None);
    tree.store.reparent(
        ActaKind::Folder,
        tree.outer,
        Some((ActaKind::Folder, tree.inner)),
    );
    let cyclic = acta(ActaKind::Document, tree.document);
    let provider = provider(tree.store);

    for resource in [
        acta(ActaKind::Document, orphan),
        acta(ActaKind::Attachment, parentless),
        cyclic,
    ] {
        assert_eq!(facts_of(&provider, &resource).await, missing(&resource));
    }
}

#[tokio::test]
async fn a_store_failure_is_unavailable_not_missing() {
    let provider = provider(FakeStore {
        fail: true,
        ..FakeStore::default()
    });

    let error = provider
        .resource_facts(&[acta(ActaKind::Document, Uuid::now_v7())])
        .await
        .unwrap_err();

    assert!(matches!(error, CapabilityError::Unavailable { .. }));
}

#[tokio::test]
async fn workspace_members_are_the_members_set_as_canonical_uuid_text() {
    let mut store = FakeStore::default();
    let workspace = Uuid::now_v7();
    let empty = Uuid::now_v7();
    let users = vec![Uuid::now_v7(), Uuid::now_v7()];
    store.members.insert(workspace, users.clone());
    store.members.insert(empty, Vec::new());
    let provider = provider(store);

    let members = provider
        .members_of(
            &format!("acta::workspace::{workspace}::members")
                .parse()
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        members
            .iter()
            .map(|member| member.as_str().to_string())
            .collect::<Vec<_>>(),
        users
            .iter()
            .map(|user| user.hyphenated().to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        provider
            .members_of(
                &format!("acta::workspace::{empty}::members")
                    .parse()
                    .unwrap()
            )
            .await
            .unwrap()
            .is_empty()
    );

    for set in [
        format!("acta::workspace::{}::members", Uuid::now_v7()),
        format!("acta::workspace::{workspace}::guests"),
        format!("acta::project::{workspace}::members"),
        format!("custos::workspace::{workspace}::members"),
        format!("acta::workspace::{}::members", workspace.simple()),
    ] {
        assert!(
            matches!(
                provider.members_of(&set.parse().unwrap()).await,
                Err(CapabilityError::NotFound { .. })
            ),
            "{set}"
        );
    }
}

#[tokio::test]
async fn the_legacy_methods_answer_through_the_same_facts() {
    let tree = tree();
    let document = acta(ActaKind::Document, tree.document);
    let ghost = acta(ActaKind::Document, Uuid::now_v7());
    let chain = vec![
        acta(ActaKind::Workspace, tree.workspace),
        acta(ActaKind::Project, tree.project),
        acta(ActaKind::Folder, tree.outer),
        acta(ActaKind::Folder, tree.inner),
    ];
    let provider = provider(tree.store);

    assert!(provider.validate_ref(&document).await.unwrap());
    assert!(!provider.validate_ref(&ghost).await.unwrap());
    assert_eq!(
        provider.path_of(&document).await.unwrap(),
        [chain.clone(), vec![document.clone()]].concat()
    );
    assert_eq!(
        provider.ancestors(&document).await.unwrap(),
        chain.into_iter().rev().collect::<Vec<_>>()
    );
    assert!(matches!(
        provider.path_of(&ghost).await,
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
}
