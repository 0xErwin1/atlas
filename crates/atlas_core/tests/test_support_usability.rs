//! Proves `atlas_core::ops::test_support::FakeWorker` is usable from outside
//! the crate, with only the `test-support` feature enabled (spec Scenario
//! "The fake worker is usable from outside `atlas_core`") — standing in for
//! what E11-S3b's `atlas_server` will do as a `[dev-dependencies]`-only
//! consumer. Requires no crate-internal (crate-private) API: this file
//! polls the returned futures with its own no-op waker, exactly as a real
//! `atlas_server` dev-dependency consumer without a runtime would have to.
//!
//! The feature is turned on for this crate's own test targets by the
//! `atlas_core` self dev-dependency in `Cargo.toml`, so a plain
//! `cargo test -p atlas_core` compiles and runs this file.

#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::future::Future;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use atlas_core::ops::test_support::FakeWorker;
use atlas_core::registry::{Worker, WorkerId};

/// Polls `future` to completion using a no-op waker, without a runtime.
/// Only valid for futures that never actually suspend — a default
/// `FakeWorker` resolves on its first poll; a stalling one must not be
/// driven through this helper.
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

#[test]
fn fake_worker_compiles_and_runs_from_outside_the_crate() {
    let id = WorkerId::new("acta.reindex").expect("valid worker id");
    let worker = FakeWorker::new(id.clone()).critical(true);

    assert!(worker.is_critical());
    assert_eq!(worker.id(), &id);

    block_on(worker.start());
    block_on(worker.drain(Duration::from_secs(1)));

    assert_eq!(
        worker.events(),
        vec![(id.clone(), "started"), (id, "drained")]
    );
}
