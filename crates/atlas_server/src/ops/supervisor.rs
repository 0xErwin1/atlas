//! Starts and drains registry-declared workers (E11-S3b design D1, D4).
//!
//! The startup barrier is structural, not a runtime probe: a `BoundWorkers`
//! value cannot exist before `AppState::new` has already returned `Ok`
//! (`build_workers` takes `&AppState`), so no worker starts before the
//! shared state it captures exists. Start order is `Registry::startup_order()`
//! then each entry's `workers` declaration order; drain walks the exact
//! reverse under one global budget.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use atlas_core::registry::{BoundWorkers, Registry, Worker, WorkerId};

use super::workers::WorkerStates;

/// Workers started by [`start_workers`], in the exact order they were
/// spawned.
pub struct RunningWorkers {
    workers: Vec<(WorkerId, Arc<dyn Worker>, JoinHandle<()>)>,
    states: Arc<WorkerStates>,
}

/// The result of [`RunningWorkers::drain`]: every worker that drained within
/// the budget, every worker whose task resolved with a panic, and every
/// worker cut off because it did not resolve in time (E11-S3b design D4).
/// A panicked worker is reported `Failed` through its `WorkerStateHandle`
/// and named in `failed`, never in `drained`; `timed_out` names only workers
/// the supervisor stopped waiting for.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DrainOutcome {
    pub drained: Vec<WorkerId>,
    pub failed: Vec<WorkerId>,
    pub timed_out: Vec<WorkerId>,
}

/// Starts every worker `bound` provides an implementation for, in
/// `registry.startup_order()` then each entry's declaration order (E11-S3b
/// design D1.2). One `tokio::spawn` per worker; the spawn calls are issued
/// in that exact order, which is the barrier this slice can prove: nothing
/// stronger, since the shipped `Worker` trait exposes no `Starting` state to
/// gate on (design D1, F5).
pub fn start_workers(
    registry: &Registry,
    bound: BoundWorkers,
    states: Arc<WorkerStates>,
) -> RunningWorkers {
    let mut workers = Vec::new();

    for component_id in registry.startup_order() {
        let Some(entry) = registry.get(component_id) else {
            continue;
        };

        for declaration in &entry.workers {
            let Some(worker) = bound.worker(&declaration.id) else {
                continue;
            };

            let spawned = worker.clone();
            let handle = tokio::spawn(async move { spawned.start().await });
            workers.push((declaration.id.clone(), worker, handle));
        }
    }

    RunningWorkers { workers, states }
}

impl RunningWorkers {
    /// The ids of every started worker, in the exact order `start_workers`
    /// spawned them. Test-only: proves start order without depending on a
    /// runtime race between components (T1.18/T1.20/T1.22).
    #[cfg(test)]
    fn ids_in_start_order(&self) -> Vec<WorkerId> {
        self.workers.iter().map(|(id, _, _)| id.clone()).collect()
    }

    /// Signals every worker to stop, then joins them in the exact reverse of
    /// the start order under one global `budget` (E11-S3b design D4). Every
    /// `Worker::drain` call completes before any `JoinHandle` is awaited, so
    /// no worker keeps running unsignalled while an earlier one in the
    /// reverse walk consumes the budget. For each join: `remaining = budget
    /// - elapsed`; a worker whose remaining budget is already exhausted is
    /// cut off without being awaited at all (its task is aborted and it is
    /// reported `Failed`); otherwise the supervisor awaits its `JoinHandle`
    /// bounded by `remaining`. Never awaits a handle outside a
    /// `tokio::time::timeout` call, and never awaits without a bound.
    pub async fn drain(self, budget: Duration) -> DrainOutcome {
        let started = tokio::time::Instant::now();
        let mut outcome = DrainOutcome::default();
        let states = self.states;

        for (_, worker, _) in self.workers.iter().rev() {
            worker.drain(budget.saturating_sub(started.elapsed())).await;
        }

        for (id, _, handle) in self.workers.into_iter().rev() {
            let remaining = budget.saturating_sub(started.elapsed());

            if remaining.is_zero() {
                handle.abort();
                Self::mark_failed(&states, &id, "drain exceeded the shutdown budget");
                outcome.timed_out.push(id);
                continue;
            }

            match tokio::time::timeout(remaining, handle).await {
                Ok(Ok(())) => outcome.drained.push(id),
                Ok(Err(join_error)) => {
                    tracing::error!(
                        worker = %id,
                        error = %join_error,
                        "worker task panicked during shutdown"
                    );
                    Self::mark_failed(&states, &id, "worker panicked during drain");
                    outcome.failed.push(id);
                }
                Err(_elapsed) => {
                    Self::mark_failed(&states, &id, "drain exceeded the shutdown budget");
                    outcome.timed_out.push(id);
                }
            }
        }

        outcome
    }

    fn mark_failed(states: &WorkerStates, id: &WorkerId, cause: &str) {
        if let Some(handle) = states.handle(id) {
            handle.failed(cause);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::ops::WorkerState;
    use atlas_core::ops::test_support::FakeWorker;
    use atlas_core::registry::{
        Api, Authorization, Capabilities, CapabilityId, ComponentEntry, ComponentId, ComponentKind,
        ContractVersion, Diagnostics, Experience, Identity, WorkerDeclaration, build,
    };

    fn worker_id(value: &str) -> WorkerId {
        WorkerId::new(value).expect("valid worker id")
    }

    fn component_id(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn single_worker_entry(stable_id: &str, worker: &str) -> ComponentEntry {
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
            workers: vec![WorkerDeclaration {
                id: worker_id(worker),
                critical: false,
            }],
            satellites: vec![],
        }
    }

    fn worker(id: &str) -> FakeWorker {
        FakeWorker::new(worker_id(id))
    }

    fn registry_with(components: &[(&str, &str)]) -> Registry {
        let entries = components
            .iter()
            .map(|(component, worker)| single_worker_entry(component, worker))
            .collect();

        build(entries).expect("valid registry")
    }

    #[tokio::test]
    async fn start_workers_walks_startup_order_then_declaration_order_non_vacuously() {
        // Mirrors real REG-5 data (design D1.2, §0.3): `search.pgvector_embeddings`
        // provides `search.semantic`, which `acta` requires optionally, so
        // the capability edge places the provider before the consumer.
        let mut search = single_worker_entry(
            "search.pgvector_embeddings",
            "search.pgvector_embeddings.index_worker",
        );
        search.capabilities.provided = vec![CapabilityId::new("search.semantic").expect("valid")];
        let mut acta = single_worker_entry("acta", "acta.reindex");
        acta.capabilities.required_optional =
            vec![CapabilityId::new("search.semantic").expect("valid")];
        let registry = build(vec![acta, search]).expect("valid registry");
        let states = Arc::new(WorkerStates::from_registry(&registry));
        let indexer = Arc::new(worker("search.pgvector_embeddings.index_worker"));
        let reindex = Arc::new(worker("acta.reindex"));
        let bound = BoundWorkers::bind(&registry, vec![indexer, reindex]).expect("valid binding");

        let running = start_workers(&registry, bound, states);

        let order = running.ids_in_start_order();
        assert!(!order.is_empty(), "must walk a non-zero worker count");
        // Scope note (design R6, spec §0.3): `custos` declares no worker and
        // is absent from `startup_order()`, so this proves
        // `search.pgvector_embeddings` before `acta` — not "custos before
        // acta", which this order cannot express.
        assert_eq!(
            order,
            vec![
                worker_id("search.pgvector_embeddings.index_worker"),
                worker_id("acta.reindex"),
            ]
        );
    }

    #[tokio::test]
    async fn start_workers_orders_two_workers_in_one_component_by_declaration_order() {
        let mut entry = single_worker_entry("acta", "acta.one");
        entry.workers.push(WorkerDeclaration {
            id: worker_id("acta.two"),
            critical: false,
        });
        let registry = build(vec![entry]).expect("valid registry");
        let states = Arc::new(WorkerStates::from_registry(&registry));
        let one = Arc::new(worker("acta.one"));
        let two = Arc::new(worker("acta.two"));
        let bound = BoundWorkers::bind(&registry, vec![two, one]).expect("valid binding");

        let running = start_workers(&registry, bound, states);

        assert_eq!(
            running.ids_in_start_order(),
            vec![worker_id("acta.one"), worker_id("acta.two")]
        );
    }

    #[tokio::test]
    async fn a_workerless_component_is_not_a_barrier_and_is_absent_from_start_order() {
        // `custos` declares no worker; `acta` does. `startup_order()` already
        // excludes workerless components (E11-S2), so `acta`'s worker starts
        // without anything named "custos" appearing in the walked order.
        let mut acta = single_worker_entry("acta", "acta.reindex");
        let mut custos = single_worker_entry("custos", "custos.ghost");
        custos.workers.clear();
        acta.dependencies.clear();
        let registry = build(vec![custos, acta]).expect("valid registry");
        let states = Arc::new(WorkerStates::from_registry(&registry));
        let reindex = Arc::new(worker("acta.reindex"));
        let bound = BoundWorkers::bind(&registry, vec![reindex]).expect("valid binding");

        let running = start_workers(&registry, bound, states);

        assert_eq!(
            running.ids_in_start_order(),
            vec![worker_id("acta.reindex")]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn drain_walks_the_exact_reverse_of_start_order_within_a_generous_budget() {
        let registry = registry_with(&[("a", "a.one"), ("b", "b.one"), ("c", "c.one")]);
        let states = Arc::new(WorkerStates::from_registry(&registry));
        let a = Arc::new(worker("a.one"));
        let b = Arc::new(worker("b.one"));
        let c = Arc::new(worker("c.one"));
        let bound = BoundWorkers::bind(&registry, vec![a, b, c]).expect("valid binding");

        let running = start_workers(&registry, bound, states);
        assert_eq!(
            running.ids_in_start_order(),
            vec![worker_id("a.one"), worker_id("b.one"), worker_id("c.one")]
        );

        let outcome = running.drain(Duration::from_secs(30)).await;

        assert_eq!(
            outcome.drained,
            vec![worker_id("c.one"), worker_id("b.one"), worker_id("a.one")]
        );
        assert!(outcome.timed_out.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn drain_cuts_three_uncooperative_workers_at_the_budget_and_marks_them_failed() {
        let registry = registry_with(&[("a", "a.one"), ("b", "b.one"), ("c", "c.one")]);
        let states = Arc::new(WorkerStates::from_registry(&registry));
        let a = Arc::new(worker("a.one").stalls_at_start());
        let b = Arc::new(worker("b.one").stalls_at_start());
        let c = Arc::new(worker("c.one").stalls_at_start());
        let bound = BoundWorkers::bind(&registry, vec![a, b, c]).expect("valid binding");

        let running = start_workers(&registry, bound, states.clone());
        let budget = Duration::from_millis(100);

        let started = tokio::time::Instant::now();
        let outcome = running.drain(budget).await;
        let elapsed = started.elapsed();

        assert!(
            elapsed <= budget + Duration::from_millis(20),
            "drain must return at or before the budget, got {elapsed:?}"
        );
        assert!(outcome.drained.is_empty());

        let mut timed_out = outcome.timed_out.clone();
        timed_out.sort();
        assert_eq!(
            timed_out,
            vec![worker_id("a.one"), worker_id("b.one"), worker_id("c.one")]
        );

        for id in &outcome.timed_out {
            assert_eq!(
                states.handle(id).expect("handle exists").status().state,
                WorkerState::Failed {
                    cause: "drain exceeded the shutdown budget".to_string()
                }
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn every_worker_is_signalled_before_the_first_join_consumes_the_budget() {
        // All three stall, so the first join in reverse order (`c`) eats the
        // whole budget and `a`/`b` are aborted with zero remaining, a path
        // that calls no `Worker::drain`. Their `drained` events can therefore
        // only have been recorded by the signal pass that runs before any
        // join is awaited.
        let registry = registry_with(&[("a", "a.one"), ("b", "b.one"), ("c", "c.one")]);
        let states = Arc::new(WorkerStates::from_registry(&registry));
        let a = Arc::new(worker("a.one").stalls_at_start());
        let b = Arc::new(worker("b.one").stalls_at_start());
        let c = Arc::new(worker("c.one").stalls_at_start());
        let bound = BoundWorkers::bind(&registry, vec![a.clone(), b.clone(), c.clone()])
            .expect("valid binding");

        let running = start_workers(&registry, bound, states);
        let outcome = running.drain(Duration::from_millis(100)).await;

        assert_eq!(
            outcome.timed_out,
            vec![worker_id("c.one"), worker_id("b.one"), worker_id("a.one")]
        );
        for fake in [&a, &b, &c] {
            assert!(
                fake.events().contains(&(fake.id().clone(), "drained")),
                "{} must have been signalled to stop",
                fake.id()
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_worker_that_drains_immediately_does_not_shrink_the_next_workers_remaining_budget() {
        // Reverse order drains `b` first (started second), then `a`
        // (started first). `b` resolves instantly; `a` stalls and ignores
        // cancellation. `a` must still receive nearly the full budget, not
        // one shrunk by `b`'s near-zero elapsed time.
        let registry = registry_with(&[("a", "a.one"), ("b", "b.one")]);
        let states = Arc::new(WorkerStates::from_registry(&registry));
        let a = Arc::new(worker("a.one").stalls_at_start());
        let b = Arc::new(worker("b.one"));
        let bound = BoundWorkers::bind(&registry, vec![a, b]).expect("valid binding");

        let running = start_workers(&registry, bound, states);
        let budget = Duration::from_millis(200);

        let started = tokio::time::Instant::now();
        let outcome = running.drain(budget).await;
        let elapsed = started.elapsed();

        assert_eq!(outcome.drained, vec![worker_id("b.one")]);
        assert_eq!(outcome.timed_out, vec![worker_id("a.one")]);
        assert!(
            elapsed >= Duration::from_millis(190),
            "the uncooperative worker must receive nearly the full budget, got {elapsed:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_panicking_worker_is_reported_failed_and_the_rest_still_drain_within_budget() {
        let registry = registry_with(&[("a", "a.one"), ("b", "b.one")]);
        let states = Arc::new(WorkerStates::from_registry(&registry));
        let a = Arc::new(worker("a.one"));
        let b = Arc::new(worker("b.one").panics_at_start());
        let bound = BoundWorkers::bind(&registry, vec![a, b]).expect("valid binding");

        let running = start_workers(&registry, bound, states.clone());

        // `b` panics on its first poll, so its task has already resolved
        // `Err` by the time drain reaches it, well within budget rather
        // than by the drain-cutoff path.
        let outcome = running.drain(Duration::from_secs(30)).await;

        assert!(outcome.timed_out.is_empty());
        assert_eq!(outcome.drained, vec![worker_id("a.one")]);
        assert_eq!(outcome.failed, vec![worker_id("b.one")]);
        assert_eq!(
            states
                .handle(&worker_id("b.one"))
                .expect("handle exists")
                .status()
                .state,
            WorkerState::Failed {
                cause: "worker panicked during drain".to_string()
            }
        );
    }
}
