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

use taskmesh_contract::{CancellationPolicy, ClassPolicy, GovernorError, SubstrateHint, TaskSpec};
use tokio_util::sync::CancellationToken;

use crate::executor::{SubmissionDeadline, SubmitOptions};

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

/// The frozen answer for one submission.
pub struct ResolvedExecutionPlan {
    /// The capability pool the work will occupy while it runs. `None` for an
    /// ungated substrate. A `&'static str` from the built-in hint table: the
    /// engine interns it, so resolving a plan allocates nothing.
    pub capability: Option<&'static str>,
    pub dispatch: DispatchKind,
    /// Mid-run cancel token, if the declared class's policy supports it.
    pub cancel: Option<CancellationToken>,
    /// Run deadline, if the declared class's policy supports one.
    pub deadline: Option<SubmissionDeadline>,
}

impl ResolvedExecutionPlan {
    /// Resolve `spec` + `opts` against the class policy for one run path.
    ///
    /// `allowed` is the set of substrate hints this run path accepts;
    /// `dispatch` is what the path will do once admitted. Both are properties of
    /// the entry point, not of the spec, so they are supplied by the caller.
    pub fn resolve(
        spec: &TaskSpec,
        opts: &SubmitOptions,
        policy: Option<&ClassPolicy>,
        allowed: &[SubstrateHint],
        dispatch: DispatchKind,
    ) -> Result<Self, GovernorError> {
        let hint = spec.primary_substrate_hint();
        if !allowed.contains(&hint) {
            return Err(GovernorError::Rejected(
                taskmesh_contract::AdmissionVerdict::SubstrateMismatch,
            ));
        }

        let wants_dedicated_thread = spec.requested_stack_size_bytes().is_some()
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

        Ok(Self {
            capability,
            dispatch,
            cancel: mid_run.then(|| opts.cancel.clone()).flatten(),
            deadline: with_deadline.then_some(opts.deadline).flatten(),
        })
    }

    /// The absolute completion instant this plan must respect, if any. `RunFor`
    /// is relative to worker start and is therefore not known here.
    pub fn absolute_deadline(&self) -> Option<std::time::Instant> {
        match self.deadline {
            Some(SubmissionDeadline::CompleteBy(instant)) => Some(instant),
            _ => None,
        }
    }
}
