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
    /// `CancelledBeforeSubmit` before any permit is sought. For a class whose
    /// `cancellation_policy` is `Cooperative`/`CooperativeWithDeadline`, firing
    /// this token mid-run also cancels in-flight async work
    /// (→ `GovernorError::Cancelled`).
    pub cancel: Option<CancellationToken>,
    /// Maximum time to wait to *acquire* a slot — covering BOTH the governor
    /// admission-queue wait AND the host substrate capability-pool wait. `None`
    /// waits indefinitely; `Some(Duration::ZERO)` means "do not wait". A queue
    /// timeout yields `PermitAcquireTimedOut`; a substrate-pool timeout yields
    /// `SubstratePoolTimedOut`, so the two causes stay distinguishable.
    pub acquire_timeout: Option<Duration>,
    /// Wall-clock budget for the *running* work, honored only for a
    /// `CooperativeWithDeadline` class. When exceeded mid-run, the work is
    /// cancelled with `GovernorError::DeadlineExceeded`. `None` = no run deadline.
    pub deadline: Option<Duration>,
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

    /// Set the run deadline (honored for `CooperativeWithDeadline` classes).
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }
}
