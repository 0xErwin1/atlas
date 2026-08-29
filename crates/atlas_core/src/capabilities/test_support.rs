//! Test-only helper for driving `#[async_trait]` futures without pulling a
//! runtime crate into `atlas_core`. Every stub in these tests resolves on
//! its first poll, so a single-poll loop with a no-op waker is sufficient.
#![cfg(test)]

use std::future::Future;
use std::task::{Context, Poll, Waker};

/// Polls `future` to completion using a no-op waker, without spawning a
/// runtime. Only valid for futures that never actually suspend.
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
