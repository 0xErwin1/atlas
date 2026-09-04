//! Public in-memory test doubles for workers and diagnostics (E11-S2 design
//! D6), gated behind `test-support` so `atlas_server` (E11-S3b) can depend on
//! them as a `[dev-dependencies]`-only edge without duplicating fakes. Never
//! enabled by a non-dev, non-test build: the feature appears in no
//! `[dependencies]` entry.

#[cfg(test)]
use std::future::Future;
use std::sync::{Arc, Mutex};
#[cfg(test)]
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use async_trait::async_trait;

use crate::capabilities::{
    Doctor, DoctorFinding, Health, HealthStatus, Readiness, ReadinessStatus,
};
use crate::ops::doctor::DoctorDeadline;
use crate::ops::readiness::ReadinessDeadline;
use crate::registry::{ComponentId, Worker, WorkerId};

/// Polls `future` to completion using a no-op waker, without spawning a
/// runtime. Only valid for futures that never actually suspend — every
/// future produced by the fakes in this module resolves on its first poll.
/// Crate-internal only: this slice's own `#[cfg(test)]` modules use it to
/// drive `Readiness`/`Doctor`/`Worker` futures without `tokio`; an external
/// `test-support` consumer (e.g. E11-S3b's `atlas_server`) already owns a
/// runtime and drives these futures with it instead.
#[cfg(test)]
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = Box::pin(future);

    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
    }
}

/// A controllable `Worker`, recording an ordered `(WorkerId, event)` log so
/// a caller (this slice's own tests, or E11-S3b's supervisor tests) can
/// assert start/drain order and outcomes without a real runtime.
pub struct FakeWorker {
    id: WorkerId,
    critical: bool,
    fails_at_start: bool,
    events: Arc<Mutex<Vec<(WorkerId, &'static str)>>>,
}

impl FakeWorker {
    pub fn new(id: WorkerId) -> Self {
        Self {
            id,
            critical: false,
            fails_at_start: false,
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn critical(mut self, critical: bool) -> Self {
        self.critical = critical;
        self
    }

    pub fn fails_at_start(mut self, fails_at_start: bool) -> Self {
        self.fails_at_start = fails_at_start;
        self
    }

    pub fn is_critical(&self) -> bool {
        self.critical
    }

    /// The ordered `(WorkerId, event)` log recorded so far.
    pub fn events(&self) -> Vec<(WorkerId, &'static str)> {
        self.lock_events().clone()
    }

    /// Locks the shared event log, recovering the guard on poison rather
    /// than panicking (mirrors `WorkerStateHandle::lock`).
    fn lock_events(&self) -> std::sync::MutexGuard<'_, Vec<(WorkerId, &'static str)>> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl Worker for FakeWorker {
    fn id(&self) -> &WorkerId {
        &self.id
    }

    async fn start(&self) {
        let event = if self.fails_at_start {
            "start_failed"
        } else {
            "started"
        };
        self.lock_events().push((self.id.clone(), event));
    }

    async fn drain(&self, _remaining_budget: Duration) {
        self.lock_events().push((self.id.clone(), "drained"));
    }
}

/// One struct implementing `Health` + `Readiness` + `Doctor` from
/// constructor-supplied, fixed values — no I/O, no delay.
pub struct FakeDiagnostics {
    health: HealthStatus,
    readiness: ReadinessStatus,
    findings: Vec<DoctorFinding>,
}

impl FakeDiagnostics {
    pub fn new(
        health: HealthStatus,
        readiness: ReadinessStatus,
        findings: Vec<DoctorFinding>,
    ) -> Self {
        Self {
            health,
            readiness,
            findings,
        }
    }
}

impl Health for FakeDiagnostics {
    fn health(&self) -> HealthStatus {
        self.health.clone()
    }
}

#[async_trait]
impl Readiness for FakeDiagnostics {
    async fn readiness(&self) -> ReadinessStatus {
        self.readiness.clone()
    }
}

#[async_trait]
impl Doctor for FakeDiagnostics {
    async fn doctor(&self) -> Vec<DoctorFinding> {
        self.findings.clone()
    }
}

/// A deadline port that never elapses: it always runs the probe to
/// completion and reports its real outcome.
pub struct NeverElapses;

#[async_trait]
impl ReadinessDeadline for NeverElapses {
    async fn bounded(
        &self,
        _component: &ComponentId,
        probe: &dyn Readiness,
    ) -> Option<ReadinessStatus> {
        Some(probe.readiness().await)
    }
}

#[async_trait]
impl DoctorDeadline for NeverElapses {
    async fn bounded(
        &self,
        _component: &ComponentId,
        probe: &dyn Doctor,
    ) -> Option<Vec<DoctorFinding>> {
        Some(probe.doctor().await)
    }
}

/// A deadline port that always reports the bound as elapsed, without
/// running the probe. Used to prove the timeout paths of
/// `aggregate_readiness`/`run_doctor` with an immediate outcome — no real
/// delay, no runtime.
pub struct AlwaysElapses;

#[async_trait]
impl ReadinessDeadline for AlwaysElapses {
    async fn bounded(
        &self,
        _component: &ComponentId,
        _probe: &dyn Readiness,
    ) -> Option<ReadinessStatus> {
        None
    }
}

#[async_trait]
impl DoctorDeadline for AlwaysElapses {
    async fn bounded(
        &self,
        _component: &ComponentId,
        _probe: &dyn Doctor,
    ) -> Option<Vec<DoctorFinding>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_worker_records_started_then_drained_in_order() {
        let worker = FakeWorker::new(WorkerId::new("acta.reindex").expect("valid worker id"));

        block_on(worker.start());
        block_on(worker.drain(Duration::from_secs(1)));

        assert_eq!(
            worker.events(),
            vec![
                (
                    WorkerId::new("acta.reindex").expect("valid worker id"),
                    "started"
                ),
                (
                    WorkerId::new("acta.reindex").expect("valid worker id"),
                    "drained"
                ),
            ]
        );
    }

    #[test]
    fn fake_worker_records_start_failed_when_configured() {
        let worker = FakeWorker::new(WorkerId::new("acta.reindex").expect("valid worker id"))
            .fails_at_start(true);

        block_on(worker.start());

        assert_eq!(
            worker.events(),
            vec![(
                WorkerId::new("acta.reindex").expect("valid worker id"),
                "start_failed"
            )]
        );
    }
}
