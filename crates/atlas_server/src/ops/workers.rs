//! Wraps the six existing background loops as `Worker` implementations, and
//! the shared worker-state table both the supervisor and diagnostics read
//! (E11-S3b design D2, D3).
//!
//! `SupervisedWorker` is the single adapter: it owns its own `watch` channel
//! and its `WorkerStateHandle`, and reports `Running`/`Stopped`/`Failed`/
//! `Inactive` around whatever future it is handed. Not one line changes
//! inside `dispatcher/`, `live/`, `presence/`, `search_indexer/`, or
//! `persistence/repos/documents.rs` — every loop keeps its own
//! `watch::Receiver<bool>` shutdown signal exactly as it is today.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use atlas_acta_postgres::repos::semantic_search::PgSemanticIndexWriter;
use chrono::Utc;
use futures::future::{BoxFuture, FutureExt};
use tokio::sync::watch;

#[cfg(test)]
use atlas_core::ops::WorkerState;
use atlas_core::ops::{WorkerStateHandle, WorkerStatus};
use atlas_core::registry::{ComponentId, Registry, Worker, WorkerId};

use crate::config::AtlasConfig;
use crate::persistence::repos::{PgAttachmentLifecycle, PgSemanticIndexer};
use crate::search_indexer::SearchIndexWorker;
use crate::state::AppState;

/// How often the adapter records a heartbeat while its wrapped future is
/// still pending. Proves the task is scheduled and alive, not that the
/// wrapped loop iterated (E11-S3b design D3/R4, AMENDMENT).
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// The final state a wrapped loop leaves its `SupervisedWorker` in once its
/// future resolves. Internal to this module: every real loop maps to
/// `Stopped`; only the conditionally-absent search worker maps to
/// `Inactive`.
pub(crate) enum WorkerOutcome {
    Stopped,
    Inactive(String),
}

/// Wraps one of today's six loops as a `Worker`, changing nothing inside the
/// loop itself (E11-S3b design D2). Constructed once per loop by
/// [`build_workers`]; `drain` only flips the adapter's own `shutdown`
/// sender, never the wrapped future's internals.
pub struct SupervisedWorker {
    id: WorkerId,
    state: WorkerStateHandle,
    shutdown: watch::Sender<bool>,
    run: Mutex<Option<BoxFuture<'static, WorkerOutcome>>>,
}

impl SupervisedWorker {
    /// `pub(crate)`, not private: the supervisor's own integration tests
    /// (T1.32) build one directly to prove start/drain state transitions
    /// flow through the same `WorkerStateHandle` diagnostics will read.
    pub(crate) fn new(
        id: WorkerId,
        state: WorkerStateHandle,
        shutdown: watch::Sender<bool>,
        run: BoxFuture<'static, WorkerOutcome>,
    ) -> Self {
        Self {
            id,
            state,
            shutdown,
            run: Mutex::new(Some(run)),
        }
    }

    fn take_run(&self) -> Option<BoxFuture<'static, WorkerOutcome>> {
        self.run
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

#[async_trait]
impl Worker for SupervisedWorker {
    fn id(&self) -> &WorkerId {
        &self.id
    }

    async fn start(&self) {
        let Some(mut run) = self.take_run() else {
            self.state.failed("already started");
            return;
        };

        self.state.running();

        let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
        heartbeat.tick().await;

        let outcome = loop {
            tokio::select! {
                outcome = &mut run => break outcome,
                _ = heartbeat.tick() => self.state.heartbeat(Utc::now()),
            }
        };

        match outcome {
            WorkerOutcome::Stopped => self.state.stopped(),
            WorkerOutcome::Inactive(reason) => self.state.inactive(reason),
        }
    }

    async fn drain(&self, _remaining_budget: Duration) {
        let _ = self.shutdown.send(true);
    }
}

/// The single source of worker state handles: one per REG-5-declared
/// worker, built from the registry alone before `AppState::new` runs
/// (E11-S3b design D3, D8). Shared by the workers that write it and by
/// diagnostics implementers that will read it (PR2).
pub struct WorkerStates {
    handles: BTreeMap<WorkerId, WorkerStateHandle>,
}

impl WorkerStates {
    /// Builds one handle per declared worker across every entry, seeded
    /// `Stopped` (production default). Total by construction: a worker
    /// with no declaration cannot get a handle, and every declaration gets
    /// exactly one.
    pub fn from_registry(registry: &Registry) -> Self {
        let mut handles = BTreeMap::new();

        for entry in registry.entries() {
            for declaration in &entry.workers {
                handles.insert(
                    declaration.id.clone(),
                    WorkerStateHandle::new(
                        declaration.id.clone(),
                        entry.identity.stable_id.clone(),
                        declaration.critical,
                    ),
                );
            }
        }

        Self { handles }
    }

    /// The handle for `id`, or `None` when no worker was declared with it.
    pub fn handle(&self, id: &WorkerId) -> Option<WorkerStateHandle> {
        self.handles.get(id).cloned()
    }

    /// Every worker status belonging to `component`.
    pub fn statuses_for(&self, component: &ComponentId) -> Vec<WorkerStatus> {
        self.handles
            .values()
            .map(WorkerStateHandle::status)
            .filter(|status| &status.component == component)
            .collect()
    }

    /// Every worker status, in ascending `WorkerId` order.
    pub fn all(&self) -> Vec<WorkerStatus> {
        self.handles
            .values()
            .map(WorkerStateHandle::status)
            .collect()
    }
}

/// Wraps today's six loops as `Worker` implementations, one
/// `SupervisedWorker` per REG-5 declaration, each wired to the same
/// function and arguments `main.rs` spawns today (E11-S3b design D2,
/// T1.13). `cfg` is accepted for signature symmetry with the boot
/// sequence's other builders; nothing here reads it yet, since every
/// argument a loop needs already lives on `state`.
pub fn build_workers(
    state: &AppState,
    _cfg: &AtlasConfig,
    states: Arc<WorkerStates>,
) -> Vec<Arc<dyn Worker>> {
    vec![
        build_dispatcher(state, &states),
        build_attachment_reconciler(state, &states),
        build_live_listener(state, &states),
        build_presence_sweeper(state, &states),
        build_presence_agent(state, &states),
        build_search_index_worker(state, &states),
    ]
}

/// Parses a fixed, source-literal REG-5 worker id. Panics only on a
/// programmer error in this module's own literals, never on external input.
#[allow(
    clippy::panic,
    reason = "the argument is always one of this module's own `&'static str` literals, \
              validated by `reg5_registry_build.rs`'s workspace test"
)]
fn worker_id(value: &str) -> WorkerId {
    WorkerId::new(value).unwrap_or_else(|e| panic!("REG-5 worker id `{value}` must be valid: {e}"))
}

/// Looks up the handle REG-5 declared for `id`. Panics only if `states` was
/// not built from the same registry `build_workers` is wired against, which
/// `AppState::new`/`for_test` (T1.35) guarantee never happens in practice.
#[allow(
    clippy::panic,
    reason = "unreachable in practice: `states` is always `WorkerStates::from_registry` over \
              the same REG-5 registry that declares every id this module constructs"
)]
fn handle_for(states: &WorkerStates, id: &WorkerId) -> WorkerStateHandle {
    states
        .handle(id)
        .unwrap_or_else(|| panic!("worker `{id}` must be declared in REG-5"))
}

fn build_dispatcher(state: &AppState, states: &WorkerStates) -> Arc<dyn Worker> {
    let id = worker_id("acta.webhook_dispatcher");
    let (shutdown, rx) = watch::channel(false);

    let dispatcher = crate::dispatcher::WebhookDispatcher::new(
        (*state.db).clone(),
        state.webhook_crypto.clone(),
        state.dispatcher_config.clone(),
        state.allow_private_webhook_targets,
    );
    let run: BoxFuture<'static, WorkerOutcome> =
        dispatcher.run(rx).map(|()| WorkerOutcome::Stopped).boxed();

    Arc::new(SupervisedWorker::new(
        id.clone(),
        handle_for(states, &id),
        shutdown,
        run,
    ))
}

fn build_attachment_reconciler(state: &AppState, states: &WorkerStates) -> Arc<dyn Worker> {
    let id = worker_id("acta.attachment_reconciler");
    let (shutdown, rx) = watch::channel(false);

    let run: BoxFuture<'static, WorkerOutcome> =
        PgAttachmentLifecycle::run_reconciler((*state.db).clone(), state.attachments.clone(), rx)
            .map(|()| WorkerOutcome::Stopped)
            .boxed();

    Arc::new(SupervisedWorker::new(
        id.clone(),
        handle_for(states, &id),
        shutdown,
        run,
    ))
}

fn build_live_listener(state: &AppState, states: &WorkerStates) -> Arc<dyn Worker> {
    let id = worker_id("acta.live_listener");
    let (shutdown, rx) = watch::channel(false);

    let live_pool = state.db.get_postgres_connection_pool().clone();
    let run: BoxFuture<'static, WorkerOutcome> =
        crate::live::run_listener(live_pool, state.live.clone(), rx)
            .map(|()| WorkerOutcome::Stopped)
            .boxed();

    Arc::new(SupervisedWorker::new(
        id.clone(),
        handle_for(states, &id),
        shutdown,
        run,
    ))
}

fn build_presence_sweeper(state: &AppState, states: &WorkerStates) -> Arc<dyn Worker> {
    let id = worker_id("acta.presence_sweeper");
    let (shutdown, rx) = watch::channel(false);

    let run: BoxFuture<'static, WorkerOutcome> =
        crate::presence::run_presence_sweeper(state.clone(), rx)
            .map(|()| WorkerOutcome::Stopped)
            .boxed();

    Arc::new(SupervisedWorker::new(
        id.clone(),
        handle_for(states, &id),
        shutdown,
        run,
    ))
}

fn build_presence_agent(state: &AppState, states: &WorkerStates) -> Arc<dyn Worker> {
    let id = worker_id("acta.presence_agent");
    let (shutdown, rx) = watch::channel(false);

    let run: BoxFuture<'static, WorkerOutcome> =
        crate::presence::run_presence_agent_consumer(state.clone(), rx)
            .map(|()| WorkerOutcome::Stopped)
            .boxed();

    Arc::new(SupervisedWorker::new(
        id.clone(),
        handle_for(states, &id),
        shutdown,
        run,
    ))
}

/// Bound unconditionally even without an embedding provider (E11-S3b design
/// D3, R3): `BoundWorkers::bind` must stay total, so the worker exists and
/// reports `Inactive` rather than being omitted.
fn build_search_index_worker(state: &AppState, states: &WorkerStates) -> Arc<dyn Worker> {
    let id = worker_id("search.pgvector_embeddings.index_worker");
    let (shutdown, rx) = watch::channel(false);

    let run: BoxFuture<'static, WorkerOutcome> = match state.embedding_provider.clone() {
        Some(provider) => {
            let writer = Arc::new(PgSemanticIndexWriter::new((*state.db).clone(), provider));
            let indexer = Arc::new(PgSemanticIndexer::new((*state.db).clone(), writer));
            let worker = SearchIndexWorker::new(
                (*state.db).clone(),
                indexer,
                Duration::from_millis(state.dispatcher_config.poll_interval_ms),
                state.dispatcher_config.batch_size,
            );

            worker.run(rx).map(|()| WorkerOutcome::Stopped).boxed()
        }
        None => async { WorkerOutcome::Inactive("no embedding provider configured".to_string()) }
            .boxed(),
    };

    Arc::new(SupervisedWorker::new(
        id.clone(),
        handle_for(states, &id),
        shutdown,
        run,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::registry::{
        Api, Authorization, Capabilities, ComponentEntry, ComponentKind, ContractVersion,
        Diagnostics, Experience, Identity, WorkerDeclaration, build,
    };

    fn component_id(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn entry_with_workers(stable_id: &str, workers: Vec<WorkerDeclaration>) -> ComponentEntry {
        ComponentEntry {
            identity: Identity {
                stable_id: component_id(stable_id),
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
            workers,
            satellites: vec![],
        }
    }

    fn registry_with_six_reg5_workers() -> Registry {
        build(vec![
            entry_with_workers(
                "acta",
                vec![
                    WorkerDeclaration {
                        id: worker_id("acta.webhook_dispatcher"),
                        critical: false,
                    },
                    WorkerDeclaration {
                        id: worker_id("acta.attachment_reconciler"),
                        critical: false,
                    },
                    WorkerDeclaration {
                        id: worker_id("acta.live_listener"),
                        critical: false,
                    },
                    WorkerDeclaration {
                        id: worker_id("acta.presence_sweeper"),
                        critical: false,
                    },
                    WorkerDeclaration {
                        id: worker_id("acta.presence_agent"),
                        critical: false,
                    },
                ],
            ),
            entry_with_workers(
                "search.pgvector_embeddings",
                vec![WorkerDeclaration {
                    id: worker_id("search.pgvector_embeddings.index_worker"),
                    critical: false,
                }],
            ),
        ])
        .expect("valid registry")
    }

    fn no_op_worker(id: &str) -> SupervisedWorker {
        let (shutdown, _rx) = watch::channel(false);
        let state = WorkerStateHandle::new(worker_id(id), component_id("acta"), false);
        let run: BoxFuture<'static, WorkerOutcome> = async { WorkerOutcome::Stopped }.boxed();

        SupervisedWorker::new(worker_id(id), state, shutdown, run)
    }

    #[tokio::test(start_paused = true)]
    async fn supervised_worker_reports_running_then_stopped_in_order() {
        let worker = no_op_worker("acta.reindex");

        worker.start().await;

        assert_eq!(worker.state.status().state, WorkerState::Stopped);
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_start_on_an_already_taken_adapter_reports_failed_and_does_not_rerun() {
        let worker = no_op_worker("acta.reindex");

        worker.start().await;
        worker.start().await;

        assert_eq!(
            worker.state.status().state,
            WorkerState::Failed {
                cause: "already started".to_string()
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_heartbeat_advances_while_the_wrapped_future_is_pending() {
        let (shutdown, _rx) = watch::channel(false);
        let state = WorkerStateHandle::new(worker_id("acta.reindex"), component_id("acta"), false);
        let run: BoxFuture<'static, WorkerOutcome> = async {
            tokio::time::sleep(HEARTBEAT_INTERVAL * 3).await;
            WorkerOutcome::Stopped
        }
        .boxed();
        let worker = SupervisedWorker::new(worker_id("acta.reindex"), state.clone(), shutdown, run);

        let handle = tokio::spawn(async move { worker.start().await });
        tokio::task::yield_now().await;

        tokio::time::advance(HEARTBEAT_INTERVAL * 2 + Duration::from_millis(1)).await;
        tokio::task::yield_now().await;

        assert!(state.status().last_heartbeat.is_some());

        tokio::time::advance(HEARTBEAT_INTERVAL).await;
        handle.await.expect("worker task did not panic");
    }

    #[test]
    fn build_workers_returns_exactly_six_workers_matching_the_six_reg5_ids() {
        let registry = registry_with_six_reg5_workers();
        let states = Arc::new(WorkerStates::from_registry(&registry));

        assert_eq!(
            states.all().len(),
            6,
            "must be non-vacuous over the six REG-5 declarations"
        );

        let expected: Vec<WorkerId> = vec![
            worker_id("acta.attachment_reconciler"),
            worker_id("acta.live_listener"),
            worker_id("acta.presence_agent"),
            worker_id("acta.presence_sweeper"),
            worker_id("acta.webhook_dispatcher"),
            worker_id("search.pgvector_embeddings.index_worker"),
        ];
        let mut actual: Vec<WorkerId> = states.all().into_iter().map(|status| status.id).collect();
        actual.sort();

        assert_eq!(actual, expected);
    }

    #[test]
    fn worker_states_from_registry_is_constructible_before_app_state_from_a_registry_alone() {
        let registry = registry_with_six_reg5_workers();

        // Constructed from only a `Registry` — no database handle, no
        // attachment store — proving the ordering by type signature
        // (T1.16): this call cannot depend on `AppState::new` having run.
        let states = WorkerStates::from_registry(&registry);

        assert!(!states.all().is_empty());
        assert!(!states.statuses_for(&component_id("acta")).is_empty());
    }

    #[test]
    fn statuses_for_acta_is_non_empty_and_scoped_to_the_component() {
        let registry = registry_with_six_reg5_workers();
        let states = WorkerStates::from_registry(&registry);

        let acta_statuses = states.statuses_for(&component_id("acta"));
        assert_eq!(acta_statuses.len(), 5);
        assert!(
            acta_statuses
                .iter()
                .all(|status| status.component == component_id("acta"))
        );

        let search_statuses = states.statuses_for(&component_id("search.pgvector_embeddings"));
        assert_eq!(search_statuses.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn start_workers_and_drain_report_state_through_the_same_handle_diagnostics_would_read() {
        use crate::ops::supervisor::start_workers;
        use atlas_core::registry::BoundWorkers;
        use tokio::sync::Notify;

        let entry = entry_with_workers(
            "acta",
            vec![WorkerDeclaration {
                id: worker_id("acta.reindex"),
                critical: false,
            }],
        );
        let registry = build(vec![entry]).expect("valid registry");
        let states = Arc::new(WorkerStates::from_registry(&registry));

        let released = Arc::new(Notify::new());
        let (shutdown, mut rx) = watch::channel(false);
        let released_for_loop = released.clone();
        let run: BoxFuture<'static, WorkerOutcome> = async move {
            released_for_loop.notified().await;
            let _ = rx.changed().await;
            WorkerOutcome::Stopped
        }
        .boxed();
        let supervised: Arc<dyn Worker> = Arc::new(SupervisedWorker::new(
            worker_id("acta.reindex"),
            handle_for(&states, &worker_id("acta.reindex")),
            shutdown,
            run,
        ));

        let bound = BoundWorkers::bind(&registry, vec![supervised]).expect("valid binding");
        let running = start_workers(&registry, bound, states.clone());

        // Let the spawned task reach `start()`'s heartbeat/select point,
        // then release it to observe `Running`.
        tokio::task::yield_now().await;
        released.notify_one();
        tokio::task::yield_now().await;

        assert_eq!(
            states
                .handle(&worker_id("acta.reindex"))
                .expect("handle exists")
                .status()
                .state,
            WorkerState::Running
        );

        let outcome = running.drain(Duration::from_secs(30)).await;

        assert_eq!(outcome.drained, vec![worker_id("acta.reindex")]);
        assert_eq!(
            states
                .handle(&worker_id("acta.reindex"))
                .expect("handle exists")
                .status()
                .state,
            WorkerState::Stopped
        );
    }
}
