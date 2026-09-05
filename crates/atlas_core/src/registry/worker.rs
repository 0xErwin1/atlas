use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::ids::impl_string_conversions;

use super::name::{RegistryIdError, validate_namespaced_id};
use super::validated::Registry;

/// The stable id of a background worker: `{component}.{name}`, e.g.
/// `acta.webhook_dispatcher` or `search.pgvector_embeddings.index_worker`
/// (the component half may itself contain further dots).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkerId(String);

impl WorkerId {
    pub fn new(value: &str) -> Result<Self, RegistryIdError> {
        validate_namespaced_id("worker id", value)?;
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for WorkerId {
    type Err = RegistryIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for WorkerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_string_conversions!(WorkerId, RegistryIdError);

/// A background worker declared by a component. Start/drain ordering is
/// derived from `Registry::startup_order()`, not from a field here.
#[derive(Debug)]
pub struct WorkerDeclaration {
    pub id: WorkerId,
    /// Whether a stopped worker makes its owning component's readiness
    /// aggregation report `NotReady` (SHELL-OPS-5).
    pub critical: bool,
}

impl WorkerDeclaration {
    pub fn critical(&self) -> bool {
        self.critical
    }
}

/// The executable half of a worker: never stored inside a `ComponentEntry`
/// (E11-S2 design D1). Bound to its declaration at runtime by
/// `BoundWorkers::bind`.
#[async_trait]
pub trait Worker: Send + Sync + 'static {
    /// The id this implementation answers for; must match a declared
    /// `WorkerDeclaration.id` for `bind` to accept it.
    fn id(&self) -> &WorkerId;

    /// Runs the worker until the caller stops driving this future.
    async fn start(&self);

    /// Bounds the worker's own internal waits by `remaining_budget`. The
    /// caller owns the deadline and any cancellation signal; this trait
    /// carries no `tokio` type and starts no timer of its own.
    async fn drain(&self, remaining_budget: Duration);
}

/// A violation found while reconciling declared workers against their
/// runtime implementations (E11-S2 design D1, D5). Reported in bulk by
/// `BoundWorkers::bind`, mirroring `RegistryBuildError`'s contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerBindError {
    #[error("worker `{worker}` has an implementation but no declaration in the registry")]
    UnknownWorker { worker: WorkerId },
    #[error("worker `{worker}` is declared but no implementation was bound")]
    UnboundWorker { worker: WorkerId },
    #[error("worker `{worker}` has more than one bound implementation")]
    DuplicateBinding { worker: WorkerId },
}

/// A total reconciliation of every declared `WorkerId` in a `Registry`
/// against a set of runtime `Worker` implementations: every declared worker
/// is bound exactly once, and every bound implementation matches a
/// declaration (E11-S2 design D1). Never stored inside `ComponentEntry`.
pub struct BoundWorkers {
    workers: BTreeMap<WorkerId, Arc<dyn Worker>>,
}

impl BoundWorkers {
    /// Reconciles `registry`'s declared workers against `workers`,
    /// reporting every violation at once rather than stopping at the first
    /// (mirrors `registry::build()`'s bulk-error contract).
    pub fn bind(
        registry: &Registry,
        workers: Vec<Arc<dyn Worker>>,
    ) -> Result<Self, Vec<WorkerBindError>> {
        let declared: BTreeSet<WorkerId> = registry
            .entries()
            .iter()
            .flat_map(|entry| {
                entry
                    .workers
                    .iter()
                    .map(|declaration| declaration.id.clone())
            })
            .collect();

        let mut grouped: BTreeMap<WorkerId, Vec<Arc<dyn Worker>>> = BTreeMap::new();
        for worker in workers {
            grouped.entry(worker.id().clone()).or_default().push(worker);
        }

        let mut errors = Vec::new();
        let mut bound = BTreeMap::new();

        let mut provided: BTreeSet<WorkerId> = BTreeSet::new();

        for (id, mut implementers) in grouped {
            provided.insert(id.clone());

            if implementers.len() > 1 {
                errors.push(WorkerBindError::DuplicateBinding { worker: id });
                continue;
            }

            if !declared.contains(&id) {
                errors.push(WorkerBindError::UnknownWorker { worker: id });
                continue;
            }

            if let Some(implementer) = implementers.pop() {
                bound.insert(id, implementer);
            }
        }

        for id in &declared {
            if !provided.contains(id) {
                errors.push(WorkerBindError::UnboundWorker { worker: id.clone() });
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(Self { workers: bound })
    }

    /// Every bound worker id, in ascending order.
    pub fn ids(&self) -> impl Iterator<Item = &WorkerId> {
        self.workers.keys()
    }

    /// Whether `id` has a bound implementation.
    pub fn contains(&self, id: &WorkerId) -> bool {
        self.workers.contains_key(id)
    }

    /// The bound implementation for `id`, or `None` when it is unbound.
    pub fn worker(&self, id: &WorkerId) -> Option<Arc<dyn Worker>> {
        self.workers.get(id).cloned()
    }
}

impl fmt::Debug for BoundWorkers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoundWorkers")
            .field("workers", &self.workers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    use super::*;
    use crate::registry::{
        Api, Authorization, Capabilities, ComponentEntry, ComponentId, ComponentKind,
        ContractVersion, Diagnostics, Experience, Identity, build,
    };

    /// Polls `future` to completion using a no-op waker, without spawning a
    /// runtime. Only valid for futures that never actually suspend — this
    /// module's own local copy of `capabilities::test_support::block_on`,
    /// which is private to `capabilities` and stays that way in this PR.
    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = Box::pin(future);

        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
        }
    }

    fn worker_id(value: &str) -> WorkerId {
        WorkerId::new(value).expect("valid worker id")
    }

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn entry_with_worker(stable_id: &str, worker: &str, critical: bool) -> ComponentEntry {
        let mut entry = minimal_entry(stable_id);
        entry.workers.push(WorkerDeclaration {
            id: worker_id(worker),
            critical,
        });
        entry
    }

    fn minimal_entry(stable_id: &str) -> ComponentEntry {
        ComponentEntry {
            identity: Identity {
                stable_id: component(stable_id),
                kind: ComponentKind::Module,
                contract_version: ContractVersion::new(1),
            },
            dependencies: vec![],
            capabilities: Capabilities {
                provided: vec![],
                required_mandatory: vec![],
                required_optional: vec![],
            },
            api: Api {
                namespace: None,
                routes: vec![],
                dto_owner: None,
            },
            authorization: Authorization {
                resource_kinds: vec![],
                actions: vec![],
                role_definitions: vec![],
                principal_sets: vec![],
                provider: false,
            },
            diagnostics: Diagnostics {
                health: false,
                readiness: true,
                doctor: false,
            },
            experience: Experience {
                navigation_providers: vec![],
                context_providers: vec![],
            },
            persistence: None,
            config: None,
            workers: vec![],
            satellites: vec![],
        }
    }

    fn registry_with(entries: Vec<ComponentEntry>) -> Registry {
        build(entries).expect("valid entries build")
    }

    #[test]
    fn worker_id_accepts_a_dotted_component_and_multi_dot_name() {
        for value in ["acta.reindex", "search.pgvector_embeddings.index_worker"] {
            assert_eq!(
                WorkerId::new(value).expect("valid worker id").as_str(),
                value
            );
        }
    }

    #[test]
    fn worker_id_rejects_a_single_label_with_no_separator() {
        assert!(WorkerId::new("acta").is_err());
    }

    #[test]
    fn worker_id_rejects_an_empty_component_half() {
        assert!(WorkerId::new(".reindex").is_err());
    }

    #[test]
    fn worker_declaration_critical_accessor_reads_back_true() {
        let declaration = WorkerDeclaration {
            id: worker_id("acta.reindex"),
            critical: true,
        };

        assert!(declaration.critical());
    }

    #[test]
    fn worker_is_object_safe() {
        let _: Option<Box<dyn Worker>> = None;
        let _: Option<std::sync::Arc<dyn Worker>> = None;
    }

    struct RecordingWorker {
        id: WorkerId,
        events: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl Worker for RecordingWorker {
        fn id(&self) -> &WorkerId {
            &self.id
        }

        async fn start(&self) {
            self.events.lock().expect("lock").push("start");
        }

        async fn drain(&self, _remaining_budget: Duration) {
            self.events.lock().expect("lock").push("drain");
        }
    }

    fn recording_worker(id: &str) -> Arc<RecordingWorker> {
        Arc::new(RecordingWorker {
            id: worker_id(id),
            events: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    #[test]
    fn worker_trait_methods_are_callable_through_the_object_safe_form() {
        let recorded = recording_worker("acta.reindex");
        let worker: Box<dyn Worker> = Box::new(RecordingWorker {
            id: worker_id("acta.reindex"),
            events: recorded.events.clone(),
        });

        block_on(worker.start());
        block_on(worker.drain(Duration::from_secs(1)));

        let events = recorded.events.lock().expect("lock").clone();
        assert_eq!(events, vec!["start", "drain"]);
    }

    #[test]
    fn bind_reports_unbound_worker_when_nothing_is_provided() {
        let registry = registry_with(vec![entry_with_worker("acta", "acta.reindex", false)]);

        let errors =
            BoundWorkers::bind(&registry, vec![]).expect_err("no implementation was bound");

        assert_eq!(
            errors,
            vec![WorkerBindError::UnboundWorker {
                worker: worker_id("acta.reindex")
            }]
        );
    }

    #[test]
    fn bind_reports_unknown_worker_when_no_declaration_matches() {
        let registry = registry_with(vec![minimal_entry("acta")]);
        let ghost = recording_worker("ghost.worker");

        let errors =
            BoundWorkers::bind(&registry, vec![ghost]).expect_err("undeclared implementation");

        assert_eq!(
            errors,
            vec![WorkerBindError::UnknownWorker {
                worker: worker_id("ghost.worker")
            }]
        );
    }

    #[test]
    fn bind_reports_duplicate_binding_when_two_implementers_share_an_id() {
        let registry = registry_with(vec![entry_with_worker("acta", "acta.reindex", false)]);
        let first = recording_worker("acta.reindex");
        let second = recording_worker("acta.reindex");

        let errors = BoundWorkers::bind(&registry, vec![first, second])
            .expect_err("two implementers for one id");

        assert_eq!(
            errors,
            vec![WorkerBindError::DuplicateBinding {
                worker: worker_id("acta.reindex")
            }]
        );
    }

    #[test]
    fn bind_reports_every_violation_together_never_stopping_at_the_first() {
        let registry = registry_with(vec![entry_with_worker("a", "a.one", false)]);
        let ghost = recording_worker("b.ghost");
        let dup_first = recording_worker("b.dup");
        let dup_second = recording_worker("b.dup");

        let errors = BoundWorkers::bind(&registry, vec![ghost, dup_first, dup_second])
            .expect_err("multiple violations");

        assert_eq!(errors.len(), 3);
        assert!(errors.contains(&WorkerBindError::UnboundWorker {
            worker: worker_id("a.one")
        }));
        assert!(errors.contains(&WorkerBindError::UnknownWorker {
            worker: worker_id("b.ghost")
        }));
        assert!(errors.contains(&WorkerBindError::DuplicateBinding {
            worker: worker_id("b.dup")
        }));
    }

    #[test]
    fn worker_accessor_returns_the_bound_implementation_and_none_for_an_unbound_id() {
        let registry = registry_with(vec![entry_with_worker("acta", "acta.reindex", false)]);
        let reindex = recording_worker("acta.reindex");

        let bound = BoundWorkers::bind(&registry, vec![reindex]).expect("valid binding");

        assert!(bound.worker(&worker_id("acta.reindex")).is_some());
        assert!(bound.worker(&worker_id("acta.unbound")).is_none());
    }

    #[test]
    fn bind_succeeds_over_a_non_zero_worker_count_and_exposes_both_ids() {
        let registry = registry_with(vec![
            entry_with_worker("acta", "acta.reindex", false),
            entry_with_worker(
                "search.pgvector_embeddings",
                "search.pgvector_embeddings.index_worker",
                false,
            ),
        ]);
        let reindex = recording_worker("acta.reindex");
        let indexer = recording_worker("search.pgvector_embeddings.index_worker");

        let bound = BoundWorkers::bind(&registry, vec![reindex, indexer]).expect("valid binding");

        let ids: BTreeSet<&WorkerId> = bound.ids().collect();
        assert_eq!(ids.len(), 2);
        assert!(bound.contains(&worker_id("acta.reindex")));
        assert!(bound.contains(&worker_id("search.pgvector_embeddings.index_worker")));
    }
}
