//! Admission verdicts and the governor/task error split.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::task::TaskClass;

/// Outcome of an admission decision as seen by callers. Each backpressure
/// variant carries an optional retry-after hint.
///
/// `#[non_exhaustive]`: new backpressure reasons may be added in a minor
/// release, so downstream matches must carry a wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AdmissionVerdict {
    Admitted,
    QueueFull {
        retry_after_ms: Option<u64>,
    },
    CpuSaturated {
        retry_after_ms: Option<u64>,
    },
    MemorySaturated {
        retry_after_ms: Option<u64>,
    },
    ClassDisabled,
    UnknownClass {
        class: TaskClass,
    },
    RuntimeUnavailable,
    ClassificationFailed,
    PermitAcquireTimedOut {
        retry_after_ms: Option<u64>,
    },
    CancelledBeforeSubmit,
    DeadlineExpiredBeforeSubmit,
    RecursiveAdmission,
    /// The task spec is structurally invalid: zero stages, an inconsistent
    /// per-stage class, or a fan-out stage missing its deterministic reduce
    /// policy.
    MalformedTask,
    /// The spec's declared substrate is incompatible with the chosen run path
    /// (e.g. an `AsyncIo` spec submitted via `run_blocking`).
    SubstrateMismatch,
    /// Timed out waiting for a host substrate capability-pool slot (distinct from
    /// `PermitAcquireTimedOut`, which is the governor admission-queue wait).
    SubstratePoolTimedOut {
        retry_after_ms: Option<u64>,
    },
}

impl AdmissionVerdict {
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            Self::QueueFull { retry_after_ms }
            | Self::CpuSaturated { retry_after_ms }
            | Self::MemorySaturated { retry_after_ms }
            | Self::PermitAcquireTimedOut { retry_after_ms }
            | Self::SubstratePoolTimedOut { retry_after_ms } => *retry_after_ms,
            _ => None,
        }
    }

    pub fn is_admitted(&self) -> bool {
        matches!(self, Self::Admitted)
    }
}

impl fmt::Display for AdmissionVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admitted => f.write_str("admitted"),
            Self::QueueFull { .. } => f.write_str("queue full"),
            Self::CpuSaturated { .. } => f.write_str("cpu saturated"),
            Self::MemorySaturated { .. } => f.write_str("memory saturated"),
            Self::ClassDisabled => f.write_str("class disabled"),
            Self::UnknownClass { class } => write!(f, "unknown class: {class}"),
            Self::RuntimeUnavailable => f.write_str("runtime unavailable"),
            Self::ClassificationFailed => f.write_str("classification failed"),
            Self::PermitAcquireTimedOut { .. } => f.write_str("permit acquire timed out"),
            Self::CancelledBeforeSubmit => f.write_str("cancelled before submit"),
            Self::DeadlineExpiredBeforeSubmit => f.write_str("deadline expired before submit"),
            Self::RecursiveAdmission => f.write_str("recursive admission rejected"),
            Self::MalformedTask => f.write_str("malformed task spec"),
            Self::SubstrateMismatch => f.write_str("substrate hint / run-path mismatch"),
            Self::SubstratePoolTimedOut { .. } => f.write_str("substrate pool acquire timed out"),
        }
    }
}

/// Errors raised by the governor itself, distinct from task failure.
/// Runtime-only: intentionally not serializable.
///
/// `#[non_exhaustive]`: governor-side failure modes may grow in a minor release.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GovernorError {
    Rejected(AdmissionVerdict),
    LocalRuntimeUnavailable,
    PolicyViolation(Cow<'static, str>),
    /// The in-flight work was cooperatively cancelled (the class's
    /// [`crate::CancellationPolicy`] permits mid-run cancellation and the
    /// submission's cancel token fired). Distinct from a task error.
    Cancelled,
    /// The in-flight work exceeded its run deadline (a `CooperativeWithDeadline`
    /// class with `SubmitOptions::deadline` set). Distinct from a task error.
    DeadlineExceeded,
}

impl GovernorError {
    /// The backpressure retry-after hint, if this is a rejection that carries one.
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            Self::Rejected(verdict) => verdict.retry_after_ms(),
            _ => None,
        }
    }

    /// The underlying admission verdict, if this error is a rejection.
    pub fn as_verdict(&self) -> Option<&AdmissionVerdict> {
        match self {
            Self::Rejected(verdict) => Some(verdict),
            _ => None,
        }
    }
}

impl fmt::Display for GovernorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(verdict) => write!(f, "admission rejected: {verdict}"),
            Self::LocalRuntimeUnavailable => f.write_str("local runtime substrate unavailable"),
            Self::PolicyViolation(message) => write!(f, "policy violation: {message}"),
            Self::Cancelled => f.write_str("work cooperatively cancelled"),
            Self::DeadlineExceeded => f.write_str("work exceeded its run deadline"),
        }
    }
}

impl std::error::Error for GovernorError {}

/// The type-level split between a governor-side failure and the task's own error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError<E> {
    Governor(GovernorError),
    Task(E),
}

impl<E> RunError<E> {
    pub fn is_governor(&self) -> bool {
        matches!(self, Self::Governor(_))
    }

    pub fn is_task(&self) -> bool {
        matches!(self, Self::Task(_))
    }

    /// The governor-side error, if this is one.
    pub fn governor(&self) -> Option<&GovernorError> {
        match self {
            Self::Governor(err) => Some(err),
            Self::Task(_) => None,
        }
    }

    /// The task error, if this is one.
    pub fn task(&self) -> Option<&E> {
        match self {
            Self::Task(err) => Some(err),
            Self::Governor(_) => None,
        }
    }

    /// The admission verdict, if this is a governor rejection — the common
    /// backpressure path, reachable without two-level destructuring.
    pub fn as_verdict(&self) -> Option<&AdmissionVerdict> {
        self.governor().and_then(GovernorError::as_verdict)
    }

    /// The backpressure retry-after hint, if any.
    pub fn retry_after_ms(&self) -> Option<u64> {
        self.governor().and_then(GovernorError::retry_after_ms)
    }
}

impl<E> From<GovernorError> for RunError<E> {
    fn from(err: GovernorError) -> Self {
        Self::Governor(err)
    }
}

impl<E: fmt::Display> fmt::Display for RunError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Governor(err) => write!(f, "governor error: {err}"),
            Self::Task(err) => write!(f, "task error: {err}"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for RunError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Governor(err) => Some(err),
            Self::Task(err) => Some(err),
        }
    }
}
