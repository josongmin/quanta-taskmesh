//! Resolved execution plans (H16-008).
//!
//! Every run path answers the same four questions — which class governs this
//! work, which capability pool will it *actually* occupy, how may it be
//! cancelled, and by when must it finish — and before this module they were
//! answered separately at five call sites. Resolving once and freezing the
//! answer is what stops the two most damaging disagreements:
//!
//! * **Declared hint vs. real dispatch.** A blocking spec that carries a stack
//!   request does not run on the blocking pool; it runs on a dedicated
//!   large-stack worker. Gating it on the pool its hint names leaves the pool it
//!   actually uses ungoverned, so `large_stack_slots = 1` does not bound it.
//! * **Intake class vs. promotion class.** A resolved plan is frozen onto the
//!   pending record at intake, so a request cannot queue needing a scarce
//!   capability and be promoted against a cheaper one.
//!
//! The *declared* class stays on the caller's `TaskSpec`; the *effective* class
//! a memory fallback re-accounted the work under is readable from the engine's
//! permit ledger. Keeping a third copy here would be one more thing to drift.

use taskmesh_contract::{
    AdmissionVerdict, CancellationPolicy, ClassPolicy, GovernorError, SubstrateHint, TaskSpec,
    ValidatedTaskPlan,
};
use taskmesh_engine::{CapabilityRequirementSet, PolicySet, ResolvedCapability};
use tokio_util::sync::CancellationToken;

use crate::executor::{SubmissionDeadline, SubmitOptions};
use taskmesh_contract::{PHYSICAL_DEDICATED, PHYSICAL_SHARED_BLOCKING};

/// The protocol upper bound for an explicitly requested worker stack: 16 GiB.
///
/// This bound is checked before the platform-width conversion. On a target
/// whose `usize` cannot represent 16 GiB, the conversion is a distinct typed
/// preflight failure rather than an OS-thread construction attempt.
pub const MAX_REQUESTED_STACK_BYTES: u64 = 1 << 34;

/// A nonzero, bounded stack size that fits the current target's `usize`.
///
/// Only dispatch preflight constructs this value. Worker code consumes it and
/// never re-reads or re-validates `TaskSpec::stack_size_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValidatedStackSize(usize);

impl ValidatedStackSize {
    pub fn get(self) -> usize {
        self.0
    }

    fn validate(requested: u64) -> Result<Self, GovernorError> {
        validate_stack_size_for_usize(requested, usize_max_as_u64())
    }
}

fn usize_max_as_u64() -> u64 {
    u64::try_from(usize::MAX).unwrap_or(u64::MAX)
}

/// Injectable target-width seam used by the 32-bit conversion oracle.
fn validate_stack_size_for_usize(
    requested: u64,
    usize_max: u64,
) -> Result<ValidatedStackSize, GovernorError> {
    if requested == 0 {
        return Err(GovernorError::PolicyViolation(
            "requested stack size must be nonzero".into(),
        ));
    }
    if requested > MAX_REQUESTED_STACK_BYTES {
        return Err(GovernorError::PolicyViolation(
            format!(
                "requested stack size {requested} bytes exceeds the supported maximum \
                 {MAX_REQUESTED_STACK_BYTES}"
            )
            .into(),
        ));
    }
    if requested > usize_max {
        return Err(GovernorError::PolicyViolation(
            "requested stack size does not fit usize".into(),
        ));
    }
    usize::try_from(requested)
        .map(ValidatedStackSize)
        .map_err(|_too_large| {
            GovernorError::PolicyViolation("requested stack size does not fit usize".into())
        })
}

/// How the host will physically run this work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchKind {
    /// Awaited on the caller's own task (async I/O, local runtime).
    CallerFuture,
    /// Tokio's blocking pool.
    BlockingPool,
    /// The pluggable `CpuExecutor` port.
    CpuExecutor,
    /// A dedicated OS thread sized by the spec's stack request, running a
    /// *synchronous* job (`run_blocking` with a stack request). Cannot be
    /// stopped mid-run.
    DedicatedStackThread,
    /// A dedicated OS thread sized by the spec's stack request that hosts an
    /// owned current-thread runtime for a cooperative future
    /// (`run_async_with_requested_stack`). Stoppable at poll boundaries like a
    /// caller future.
    DedicatedStackRuntime,
}

/// The admission-ready, host-validated answer for one submission.
///
/// Construction consumes C01's [`ValidatedTaskPlan`] boundary first, then
/// validates host dispatch semantics. Admission and every worker path receive
/// this value; no later path interprets raw stack bytes.
pub struct ValidatedDispatchPlan {
    task: ValidatedTaskPlan,
    /// Canonical authority-backed role + physical-domain capabilities charged
    /// atomically by admission. Raw names do not survive preflight.
    pub(crate) requirements: CapabilityRequirementSet,
    pub(crate) dispatch: DispatchKind,
    stack_size: Option<ValidatedStackSize>,
    /// Mid-run cancel token, if the declared class's policy supports it.
    pub(crate) cancel: Option<CancellationToken>,
    /// Run deadline, if the declared class's policy supports one.
    pub(crate) deadline: Option<SubmissionDeadline>,
}

impl ValidatedDispatchPlan {
    /// Resolve `spec` + `opts` against the class policy for one run path.
    ///
    /// `allowed` is the set of substrate hints this run path accepts;
    /// `dispatch` is what the path will do once admitted. Both are properties of
    /// the entry point, not of the spec, so they are supplied by the caller.
    pub(crate) fn preflight(
        spec: TaskSpec,
        opts: &SubmitOptions,
        policy: Option<&ClassPolicy>,
        allowed: &[SubstrateHint],
        dispatch: DispatchKind,
        authority: &PolicySet,
        cpu_physical_domain: &'static str,
    ) -> Result<Self, GovernorError> {
        // Contract shape is authoritative and has precedence over every
        // host-specific dispatch check. A malformed task never reaches the
        // governor, so it allocates no ticket or permit.
        let task = ValidatedTaskPlan::try_from(spec)
            .map_err(|_error| GovernorError::Rejected(AdmissionVerdict::MalformedTask))?;
        let spec = task.as_spec();
        let hint = spec.primary_substrate_hint();
        if !allowed.contains(&hint) {
            return Err(GovernorError::Rejected(
                taskmesh_contract::AdmissionVerdict::SubstrateMismatch,
            ));
        }

        let requested_stack = spec.requested_stack_size_bytes();
        let wants_dedicated_thread = requested_stack.is_some()
            && matches!(
                hint,
                SubstrateHint::BlockingPool
                    | SubstrateHint::LargeStackCapability
                    | SubstrateHint::BackgroundOnly
            );
        // A stack request moves a *synchronous* blocking-family job onto a
        // dedicated thread. The async requested-stack entry point already
        // names its own dispatch (`DedicatedStackRuntime`) and keeps it: the
        // two differ in what a deadline can do to them.
        let dispatch = if wants_dedicated_thread && dispatch == DispatchKind::BlockingPool {
            DispatchKind::DedicatedStackThread
        } else {
            dispatch
        };

        let stack_size = match (dispatch, requested_stack) {
            (
                DispatchKind::DedicatedStackThread | DispatchKind::DedicatedStackRuntime,
                Some(requested),
            ) => Some(ValidatedStackSize::validate(requested)?),
            (DispatchKind::DedicatedStackRuntime, None) => {
                return Err(GovernorError::PolicyViolation(
                    "requested stack size missing on async large-stack path".into(),
                ));
            }
            (DispatchKind::CallerFuture | DispatchKind::CpuExecutor, Some(_requested)) => {
                return Err(GovernorError::PolicyViolation(
                    "requested stack size is not supported by this dispatch path".into(),
                ));
            }
            (DispatchKind::BlockingPool, Some(_)) => {
                return Err(GovernorError::PolicyViolation(
                    "requested stack size cannot use pooled blocking dispatch".into(),
                ));
            }
            (_, None) => None,
        };

        // D04: a stack request consumes the large-stack capability whichever
        // blocking-family hint carried it. Charging it to `blocking` (or
        // `maintenance`) would let `large_stack_slots` be bypassed by spelling
        // the hint differently.
        //
        // Without a stack request the hint alone decides the pool. That is the
        // documented model for the blocking family (`SubstrateHint` docs):
        // `blocking`, `large_stack`, and `maintenance` are three concurrency
        // gates over one shared blocking executor, so a large-stack-*classified*
        // job that asked for no particular stack runs on that executor and holds
        // a `large_stack` slot, exactly as a `BackgroundOnly` job holds
        // `maintenance`.
        let capability = if wants_dedicated_thread {
            SubstrateHint::LargeStackCapability.capability_pool()
        } else {
            hint.capability_pool()
        };

        let cancellation_policy = policy.map(|p| p.cancellation_policy);
        let mid_run = matches!(
            cancellation_policy,
            Some(CancellationPolicy::Cooperative | CancellationPolicy::CooperativeWithDeadline)
        );
        let with_deadline = matches!(
            cancellation_policy,
            Some(CancellationPolicy::CooperativeWithDeadline)
        );
        // A deadline the class cannot enforce is refused, not dropped. The
        // caller asked for a bound; running without one and returning `Ok` is
        // the silent kind of failure this host does not produce. (A cancel token
        // on a `PreSubmitOnly` class is different: it *is* honored, before
        // submit, which is exactly what that policy promises.) An unknown class
        // is left for admission to reject with its own verdict.
        if let (Some(policy), Some(_deadline), false) =
            (cancellation_policy, opts.deadline, with_deadline)
        {
            return Err(GovernorError::DeadlineUnsupported {
                class: spec.class.clone(),
                policy,
            });
        }

        let physical_domain = match dispatch {
            DispatchKind::BlockingPool => Some(PHYSICAL_SHARED_BLOCKING),
            DispatchKind::CpuExecutor => Some(cpu_physical_domain),
            DispatchKind::DedicatedStackThread | DispatchKind::DedicatedStackRuntime => {
                Some(PHYSICAL_DEDICATED)
            }
            DispatchKind::CallerFuture => None,
        };
        let resolved = [capability, physical_domain]
            .into_iter()
            .flatten()
            .map(|name| authority.resolve_capability(name))
            .collect::<Result<Vec<ResolvedCapability>, _>>()
            .map_err(|error| {
                GovernorError::PolicyViolation(
                    format!("dispatch capability resolution failed: {error}").into(),
                )
            })?;
        let requirements = CapabilityRequirementSet::from_resolved(resolved).map_err(|error| {
            GovernorError::PolicyViolation(
                format!("dispatch capability requirement failed: {error}").into(),
            )
        })?;

        Ok(Self {
            task,
            requirements,
            dispatch,
            stack_size,
            cancel: mid_run.then(|| opts.cancel.clone()).flatten(),
            deadline: with_deadline.then_some(opts.deadline).flatten(),
        })
    }

    pub fn task(&self) -> &ValidatedTaskPlan {
        &self.task
    }

    pub fn spec(&self) -> &TaskSpec {
        self.task.as_spec()
    }

    pub fn stack_size(&self) -> Option<ValidatedStackSize> {
        self.stack_size
    }

    /// The absolute completion instant this plan must respect, if any. `RunFor`
    /// is relative to worker start and is therefore not known here.
    pub fn absolute_deadline(&self) -> Option<std::time::Instant> {
        match self.deadline {
            Some(SubmissionDeadline::CompleteBy(instant)) => Some(instant),
            _ => None,
        }
    }

    /// The relative run budget (`RunFor`), if any: the only deadline a
    /// synchronous (detached) dispatch can carry. An absolute `CompleteBy` on
    /// such a dispatch is refused before a lease exists
    /// (`plan_supports_absolute_deadline`), so a detached worker never sees one
    /// and needs no branch for it.
    pub fn run_budget(&self) -> Option<std::time::Duration> {
        match self.deadline {
            Some(SubmissionDeadline::RunFor(budget)) => Some(budget),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_validator_has_an_injectable_32_bit_conversion_boundary() {
        let maximum_32_bit = u64::from(u32::MAX);
        assert_eq!(
            validate_stack_size_for_usize(maximum_32_bit, maximum_32_bit)
                .expect("32-bit maximum is representable")
                .get() as u64,
            maximum_32_bit
        );
        assert_eq!(
            validate_stack_size_for_usize(maximum_32_bit + 1, maximum_32_bit),
            Err(GovernorError::PolicyViolation(
                "requested stack size does not fit usize".into()
            ))
        );
    }
}
