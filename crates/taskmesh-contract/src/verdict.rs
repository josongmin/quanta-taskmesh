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
    /// The task spec is structurally invalid (e.g. a fan-out stage missing its
    /// deterministic reduce policy, or a substrate-hint/run-path mismatch).
    MalformedTask,
}

impl AdmissionVerdict {
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            Self::QueueFull { retry_after_ms }
            | Self::CpuSaturated { retry_after_ms }
            | Self::MemorySaturated { retry_after_ms }
            | Self::PermitAcquireTimedOut { retry_after_ms } => *retry_after_ms,
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
}

impl fmt::Display for GovernorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(verdict) => write!(f, "admission rejected: {verdict}"),
            Self::LocalRuntimeUnavailable => f.write_str("local runtime substrate unavailable"),
            Self::PolicyViolation(message) => write!(f, "policy violation: {message}"),
            Self::Cancelled => f.write_str("work cooperatively cancelled"),
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
