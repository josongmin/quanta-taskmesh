//! The `PermitWaker` port backed by `tokio::sync::Notify` (T07). The engine calls
//! `wake()` on promotion; the host awaits `notified()` to learn its queued
//! request became runnable, without polling.

use std::sync::Arc;

use taskmesh_contract::PermitWaker;
use tokio::sync::Notify;

/// A one-shot-ish promotion notifier. `Notify` stores a permit if `wake()` races
/// ahead of the waiter, so no wakeup is lost.
#[derive(Debug, Default)]
pub struct TokioPermitWaker {
    notify: Notify,
}

impl TokioPermitWaker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn notified(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.notify.notified()
    }
}

impl PermitWaker for TokioPermitWaker {
    fn wake(&self) {
        self.notify.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn notification_is_pending_until_wake_and_then_completes() {
        let waker = TokioPermitWaker::new();
        assert!(
            tokio::time::timeout(Duration::from_millis(1), waker.notified())
                .await
                .is_err(),
            "a notifier must not manufacture a permit"
        );
        waker.wake();
        tokio::time::timeout(Duration::from_secs(1), waker.notified())
            .await
            .expect("stored notification is delivered");
    }
}
