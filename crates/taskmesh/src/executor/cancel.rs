//! Cancellation and deadline adapter (T07). Timeout/cancel are host concerns,
//! kept out of the engine entirely.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

/// Per-submission control: pre-submit cancellation and a bounded acquire wait.
///
/// The plain `Runtime` trait methods submit with [`SubmitOptions::unbounded`];
/// the `*_with` inherent methods expose this for callers that need cancel or a
/// timed acquire.
#[derive(Debug, Default, Clone)]
pub struct SubmitOptions {
    /// If set and already cancelled at submit time, admission is rejected with
    /// `CancelledBeforeSubmit` before any permit is sought.
    pub cancel: Option<CancellationToken>,
    /// Maximum time to wait for a queued request to be promoted. `None` waits
    /// until promotion; `Some(Duration::ZERO)` means "do not wait".
    pub acquire_timeout: Option<Duration>,
}

impl SubmitOptions {
    /// No cancellation, wait indefinitely for promotion.
    pub fn unbounded() -> Self {
        Self::default()
    }

    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    pub fn with_acquire_timeout(mut self, timeout: Duration) -> Self {
        self.acquire_timeout = Some(timeout);
        self
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(|t| t.is_cancelled())
    }
}
