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
    /// The substrate capability pool the request resolved to is at capacity and
    /// the class does not queue. Distinct from [`Self::SubstratePoolTimedOut`]:
    /// this is an *immediate*, non-waiting shed under the class's
    /// [`crate::OverflowPolicy`], not the expiry of a bounded wait.
    SubstrateSaturated {
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
            | Self::SubstratePoolTimedOut { retry_after_ms }
            | Self::SubstrateSaturated { retry_after_ms } => *retry_after_ms,
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
            Self::SubstrateSaturated { .. } => f.write_str("substrate capability pool saturated"),
        }
    }
}

/// Why a queued ticket ended without its waiter receiving ownership.
///
/// A waiter must be able to tell these apart from "not promoted yet". Collapsing
/// them into one absent value is what turns a reclaimed reservation into a
/// waiter parked forever on a promotion that already happened and was undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TerminalReason {
    /// The leak sweep reclaimed the promoted-but-unclaimed permit.
    Reclaimed,
    /// The permit was released before the waiter claimed it.
    Released,
    /// The waiter itself abandoned the ticket.
    Abandoned,
}

impl fmt::Display for TerminalReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Reclaimed => "reclaimed by the leak sweep",
            Self::Released => "released before the claim",
            Self::Abandoned => "abandoned by the waiter",
        })
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
    /// An issued ticket ended without transferring permit ownership.
    TicketClaimTerminated {
        ticket: u64,
        reason: TerminalReason,
    },
    /// The ticket is unknown, already claimed, or outside terminal retention.
    /// It cannot become ready and must not be treated as pending.
    InvalidTicketClaim {
        ticket: u64,
    },
    LocalRuntimeUnavailable,
    PolicyViolation(Cow<'static, str>),
    /// The worker topology is structurally impossible (inverted worker bounds,
    /// an unrepresentable slot count, a zero fixed CPU pool). Reported as a typed
    /// value rather than a panic inside a resolver.
    InvalidTopology(crate::topology::TopologyError),
    /// The in-flight work was cooperatively cancelled (the class's
    /// [`crate::CancellationPolicy`] permits mid-run cancellation and the
    /// submission's cancel token fired). Distinct from a task error.
    Cancelled,
    /// The in-flight work exceeded its run deadline (a `CooperativeWithDeadline`
    /// class with `SubmitOptions::deadline` set). Distinct from a task error.
    DeadlineExceeded,
    /// A run deadline was requested for a class whose
    /// [`crate::CancellationPolicy`] cannot enforce one. The deadline would have
    /// been silently dropped; refusing the submission is the only honest answer.
    DeadlineUnsupported {
        class: TaskClass,
        policy: crate::CancellationPolicy,
    },
    /// The permit behind an execution was no longer live when the host declared
    /// its next ownership phase. A granted permit disappears under its holder in
    /// exactly one way: the leak sweep reclaimed a `DispatchReserved` lease that
    /// went stale before dispatch. The work is refused rather than run on
    /// capacity the engine has already handed to someone else.
    LeaseReclaimed {
        permit_id: u64,
    },
    /// The host could not create the worker the plan needed (a dedicated thread
    /// or a worker-local runtime). The job never started.
    WorkerUnavailable {
        context: Cow<'static, str>,
        detail: String,
    },
    /// A host worker — or the executor adapter's own `spawn` — panicked. The job
    /// may have started; its side effects are unknown.
    WorkerPanicked {
        context: Cow<'static, str>,
    },
    /// The worker that owned the job went away without reporting a result: an
    /// executor adapter dropped the closure without running it, or the worker
    /// thread died. The job's side effects are unknown.
    JobAbandoned {
        context: Cow<'static, str>,
    },
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
            Self::TicketClaimTerminated { ticket, reason } => {
                write!(f, "queued ticket {ticket} terminated: {reason}")
            }
            Self::InvalidTicketClaim { ticket } => {
                write!(f, "queued ticket {ticket} is unknown to the governor")
            }
            Self::LocalRuntimeUnavailable => f.write_str("local runtime substrate unavailable"),
            Self::PolicyViolation(message) => write!(f, "policy violation: {message}"),
            Self::InvalidTopology(error) => write!(f, "invalid topology: {error}"),
            Self::Cancelled => f.write_str("work cooperatively cancelled"),
            Self::DeadlineExceeded => f.write_str("work exceeded its run deadline"),
            Self::DeadlineUnsupported { class, policy } => write!(
                f,
                "class {class} (cancellation policy {policy:?}) cannot enforce a run deadline"
            ),
            Self::LeaseReclaimed { permit_id } => write!(
                f,
                "permit {permit_id} was reclaimed before the work was dispatched"
            ),
            Self::WorkerUnavailable { context, detail } => {
                write!(f, "{context}: worker could not be created: {detail}")
            }
            Self::WorkerPanicked { context } => write!(f, "{context} panicked"),
            Self::JobAbandoned { context } => {
                write!(f, "{context} went away without reporting a result")
            }
        }
    }
}

impl From<crate::topology::TopologyError> for GovernorError {
    fn from(error: crate::topology::TopologyError) -> Self {
        Self::InvalidTopology(error)
    }
}

impl std::error::Error for GovernorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidTopology(error) => Some(error),
            _ => None,
        }
    }
}

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
