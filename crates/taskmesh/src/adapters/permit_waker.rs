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

    pub async fn notified(&self) {
        self.notify.notified().await;
    }
}

impl PermitWaker for TokioPermitWaker {
    fn wake(&self) {
        self.notify.notify_one();
    }
}
