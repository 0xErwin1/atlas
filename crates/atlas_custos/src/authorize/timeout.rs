//! A provider call raced against the injected sleeper. `atlas_custos` stays
//! runtime-free, so the timer comes from the caller through [`Sleeper`].
//!
//! [`Sleeper`]: super::Sleeper

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub(super) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Resolves to the work's output, or `None` when the timer fires first. The
/// work is polled first, so an answer ready together with the timer wins.
pub(super) async fn within<'a, T>(work: BoxFuture<'a, T>, timer: BoxFuture<'a, ()>) -> Option<T> {
    Race { work, timer }.await
}

struct Race<'a, T> {
    work: BoxFuture<'a, T>,
    timer: BoxFuture<'a, ()>,
}

impl<T> Future for Race<'_, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let race = self.get_mut();

        if let Poll::Ready(output) = race.work.as_mut().poll(cx) {
            return Poll::Ready(Some(output));
        }

        match race.timer.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
