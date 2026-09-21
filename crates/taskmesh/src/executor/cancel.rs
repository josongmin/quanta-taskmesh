//! Cancellation and deadline adapter (T07). Timeout/cancel are host concerns,
//! kept out of the engine entirely.

use std::time::Duration;

use taskmesh_contract::{AdmissionVerdict, GovernorError};
use tokio_util::sync::CancellationToken;

/// Mutually exclusive deadline authority for one submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionDeadline {
    /// Relative budget that starts when governed work begins running.
    RunFor(Duration),
    /// Absolute boundary covering substrate wait, admission, and execution.
    CompleteBy(std::time::Instant),
}

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
    /// Maximum time to wait to *acquire* the execution lease: the whole
    /// admission wait, including time blocked on the admission mutex and time
    /// queued for class capacity or for a capability pool (one decision, one
    /// queue — there is no separate host pool wait). `None` waits indefinitely;
    /// `Some(Duration::ZERO)` means "try, do not wait". On expiry the verdict
    /// names what the request was waiting on: `PermitAcquireTimedOut` for class
    /// capacity, `SubstratePoolTimedOut` for a capability pool. A permit that
    /// arrives after the budget expired is returned unstarted (D09).
    pub acquire_timeout: Option<Duration>,
    /// Deadline authority for a `CooperativeWithDeadline` class; any other
    /// class refuses a deadline with `GovernorError::DeadlineUnsupported`.
    /// `RunFor` is anchored to the worker's own start (not to admission or to
    /// `spawn` returning); `CompleteBy` is one instant across queue wait and
    /// execution. For synchronous dispatches (`run_blocking`, `run_cpu`) a
    /// `RunFor` bounds the caller's wait while the job stays charged.
    pub deadline: Option<SubmissionDeadline>,
}

impl SubmitOptions {
    /// No cancellation, wait indefinitely for promotion.
    // This named constructor is intentionally identical to `Default`; exclude
    // only that mechanically equivalent whole-function replacement.
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
        self.deadline = Some(SubmissionDeadline::RunFor(deadline));
        self
    }

    /// Set one absolute deadline across substrate wait, admission, and execution.
    pub fn with_absolute_deadline(mut self, deadline: std::time::Instant) -> Self {
        self.deadline = Some(SubmissionDeadline::CompleteBy(deadline));
        self
    }
}

/// Relative acquisition authority without collapsing it into an absolute task
/// completion deadline. `TryOnce` is intentionally distinct from `At(now)`: an
/// uncontended immediate admit succeeds, while the first queued observation
/// expires without parking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelativeAcquisitionBudget {
    Unbounded,
    TryOnce,
    At(std::time::Instant),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcquisitionRejection {
    Cancelled,
    AbsoluteDeadline,
    RelativeTimeout,
}

/// Single owner for pre-dispatch cancel/deadline/budget precedence.
///
/// The three inputs remain independent until the final lease handoff. Their
/// fixed precedence is cancel, absolute completion deadline, then relative
/// acquisition timeout; equality is expired for both clocks.
#[derive(Debug, Clone)]
pub struct AcquisitionArbiter {
    cancel: Option<CancellationToken>,
    absolute_deadline: Option<std::time::Instant>,
    relative: RelativeAcquisitionBudget,
}

impl AcquisitionArbiter {
    pub(crate) fn new(
        opts: &SubmitOptions,
        absolute_deadline: Option<std::time::Instant>,
    ) -> Result<Self, GovernorError> {
        let now = std::time::Instant::now();
        let relative = match opts.acquire_timeout {
            None => RelativeAcquisitionBudget::Unbounded,
            Some(Duration::ZERO) => RelativeAcquisitionBudget::TryOnce,
            Some(timeout) => {
                RelativeAcquisitionBudget::At(now.checked_add(timeout).ok_or_else(|| {
                    GovernorError::PolicyViolation("acquire timeout exceeds Instant range".into())
                })?)
            }
        };
        Ok(Self {
            cancel: opts.cancel.clone(),
            absolute_deadline,
            relative,
        })
    }

    pub(crate) fn cancel_token(&self) -> Option<&CancellationToken> {
        self.cancel.as_ref()
    }

    /// Earliest instant that can wake a queued acquisition. The verdict is not
    /// chosen from this merged wait key; [`Self::rejection`] re-evaluates the
    /// original authorities in precedence order after every wake.
    pub(crate) fn wake_deadline(&self) -> Option<std::time::Instant> {
        let relative = match self.relative {
            RelativeAcquisitionBudget::At(deadline) => Some(deadline),
            RelativeAcquisitionBudget::TryOnce | RelativeAcquisitionBudget::Unbounded => None,
        };
        match (self.absolute_deadline, relative) {
            (Some(absolute), Some(relative)) => Some(absolute.min(relative)),
            (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
            (None, None) => None,
        }
    }

    pub(crate) fn rejection(
        &self,
        now: std::time::Instant,
        queued: bool,
    ) -> Option<AcquisitionRejection> {
        if self
            .cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Some(AcquisitionRejection::Cancelled);
        }
        if self
            .absolute_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            return Some(AcquisitionRejection::AbsoluteDeadline);
        }
        match self.relative {
            RelativeAcquisitionBudget::TryOnce if queued => {
                Some(AcquisitionRejection::RelativeTimeout)
            }
            RelativeAcquisitionBudget::At(deadline) if now >= deadline => {
                Some(AcquisitionRejection::RelativeTimeout)
            }
            RelativeAcquisitionBudget::Unbounded
            | RelativeAcquisitionBudget::TryOnce
            | RelativeAcquisitionBudget::At(_) => None,
        }
    }

    pub(crate) fn to_governor_error(
        rejection: AcquisitionRejection,
        capability_blocked: bool,
    ) -> GovernorError {
        match rejection {
            AcquisitionRejection::Cancelled => {
                GovernorError::Rejected(AdmissionVerdict::CancelledBeforeSubmit)
            }
            AcquisitionRejection::AbsoluteDeadline => GovernorError::DeadlineExceeded,
            AcquisitionRejection::RelativeTimeout if capability_blocked => {
                GovernorError::Rejected(AdmissionVerdict::SubstratePoolTimedOut {
                    retry_after_ms: None,
                })
            }
            AcquisitionRejection::RelativeTimeout => {
                GovernorError::Rejected(AdmissionVerdict::PermitAcquireTimedOut {
                    retry_after_ms: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_wins_three_way_tie() {
        let token = CancellationToken::new();
        token.cancel();
        let now = std::time::Instant::now();
        let arbiter = AcquisitionArbiter {
            cancel: Some(token),
            absolute_deadline: Some(now),
            relative: RelativeAcquisitionBudget::At(now),
        };
        assert_eq!(
            arbiter.rejection(now, true),
            Some(AcquisitionRejection::Cancelled)
        );
    }

    #[test]
    fn absolute_deadline_wins_relative_timeout_tie_at_equality() {
        let now = std::time::Instant::now();
        let arbiter = AcquisitionArbiter {
            cancel: None,
            absolute_deadline: Some(now),
            relative: RelativeAcquisitionBudget::At(now),
        };
        assert_eq!(
            arbiter.rejection(now, true),
            Some(AcquisitionRejection::AbsoluteDeadline)
        );
    }

    #[test]
    fn zero_budget_is_try_once_without_suppressing_absolute_deadline() {
        let now = std::time::Instant::now();
        let live = AcquisitionArbiter {
            cancel: None,
            absolute_deadline: None,
            relative: RelativeAcquisitionBudget::TryOnce,
        };
        assert_eq!(live.rejection(now, false), None);
        assert_eq!(
            live.rejection(now, true),
            Some(AcquisitionRejection::RelativeTimeout)
        );

        let expired = AcquisitionArbiter {
            cancel: None,
            absolute_deadline: Some(now),
            relative: RelativeAcquisitionBudget::TryOnce,
        };
        assert_eq!(
            expired.rejection(now, false),
            Some(AcquisitionRejection::AbsoluteDeadline)
        );
    }

    #[test]
    fn submission_option_builders_preserve_each_authority() {
        let token = CancellationToken::new();
        let absolute = std::time::Instant::now() + Duration::from_secs(10);
        let options = SubmitOptions::unbounded()
            .with_cancel(token.clone())
            .with_acquire_timeout(Duration::from_millis(7))
            .with_deadline(Duration::from_millis(11));
        token.cancel();
        assert!(options
            .cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled));
        assert_eq!(options.acquire_timeout, Some(Duration::from_millis(7)));
        assert_eq!(
            options.deadline,
            Some(SubmissionDeadline::RunFor(Duration::from_millis(11)))
        );

        let absolute_options = SubmitOptions::unbounded().with_absolute_deadline(absolute);
        assert_eq!(
            absolute_options.deadline,
            Some(SubmissionDeadline::CompleteBy(absolute))
        );
    }
}
