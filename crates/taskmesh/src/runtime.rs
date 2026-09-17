//! `TokioRuntime` — the host adapter implementing the [`Runtime`] driving port.
//!
//! # One authority, one queue
//!
//! A submission needs two things to run: semantic permission from its class
//! policy, and a slot in the capability pool it will physically occupy. Those
//! used to be two separate gates, taken in sequence, and that arrangement had
//! three consequences that no amount of tuning fixes:
//!
//! * the physical gate had **no depth limit**, so `OverflowPolicy::Reject` and
//!   `max_queue_depth` applied to a queue that requests reached second;
//! * a request could **hold a worker slot while waiting** for class capacity,
//!   idling a worker that another class was runnable on; and
//! * the physical gate is **FIFO**, so arrival order arbitrated before class
//!   fairness ever saw the request.
//!
//! Both capacities are now decided by the same admission transition in the
//! engine. There is no host-side semaphore, so `queued` counts everything that
//! is waiting, `Reject` rejects immediately, and fairness is not pre-empted by a
//! queue in front of it.
//!
//! # Response is not custody
//!
//! A caller's answer and the work's lifetime are separate events. A deadline
//! reply, a cancel, or a dropped caller future ends the *caller's* wait; the
//! worker keeps the execution lease until it has actually terminated — including
//! any owned runtime it must tear down. Capacity is therefore never reported
//! free while something is still using it, and a completed result is never
//! delivered before its capacity is released.

use std::future::{ready, Future};
use std::panic::{self, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use taskmesh_contract::{
    AdmissionVerdict, CpuExecutor, ExecutionPhase, GovernorError, RunError, Runtime, Snapshot,
    SubstrateHint, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, CapacityBlock, ClaimOutcome, Governor, PermitId, ReleaseOutcome,
};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::adapters::TokioPermitWaker;
use crate::execution_plan::{DispatchKind, ResolvedExecutionPlan};
use crate::executor::{SubmissionDeadline, SubmitOptions};
use crate::RuntimeConfig;

#[cfg(test)]
mod claim_acquisition_tests;

type BlockingJobV1<T, E> = Box<dyn FnOnce() -> Result<T, E> + Send + 'static>;
type RequestedStackAsyncFutureV1<T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + 'static>>;
type RequestedStackAsyncFutureFactoryV1<T, E> =
    Box<dyn FnOnce() -> RequestedStackAsyncFutureV1<T, E> + Send + 'static>;
type RuntimeBoxFutureV1<'a, T, E> =
    Pin<Box<dyn Future<Output = Result<T, RunError<E>>> + Send + 'a>>;

/// Longest OS thread label the host will construct.
const MAX_THREAD_NAME_BYTES: usize = 48;

/// A governed, Tokio-hosted runtime. Cheap to clone (shared `Arc` internals).
#[derive(Clone)]
pub struct TokioRuntime {
    pub(crate) config: RuntimeConfig,
    pub(crate) governor: Arc<Governor>,
    cpu: Arc<dyn CpuExecutor>,
}

impl TokioRuntime {
    pub(crate) fn new(
        config: RuntimeConfig,
        governor: Arc<Governor>,
        cpu: Arc<dyn CpuExecutor>,
    ) -> Self {
        Self {
            config,
            governor,
            cpu,
        }
    }

    /// The validated configuration this runtime was built from.
    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Direct access to the governor for advanced governance (memory reconcile,
    /// leak sweep, substrate inventory).
    pub fn governor(&self) -> &Governor {
        &self.governor
    }

    /// What the installed CPU executor declared about itself (D05). The builder
    /// already refused a `cpu` gate wider than `declared_workers`; the rest is
    /// surfaced so an operator can see whether the pool is exclusive and whether
    /// submission is non-blocking, instead of assuming either.
    pub fn executor_capabilities(&self) -> taskmesh_contract::ExecutorCapabilities {
        self.cpu.capabilities()
    }

    // ---- acquire/release core --------------------------------------------

    /// Acquire a governor permit that reserves `capability`.
    ///
    /// The capability is an explicit argument rather than something re-derived
    /// here: the pool a submission *reserves* must be the pool it will *occupy*,
    /// and only the resolved execution plan knows which that is.
    ///
    /// Honors pre-submit cancel and a bounded acquire wait. Queued requests park
    /// on a [`TokioPermitWaker`] and are woken on promotion — no polling.
    async fn acquire(
        &self,
        spec: &TaskSpec,
        capability: Option<&str>,
        opts: &SubmitOptions,
        acquire_deadline: Option<Instant>,
    ) -> Result<PermitId, GovernorError> {
        if opts.is_cancelled() {
            return Err(GovernorError::Rejected(
                AdmissionVerdict::CancelledBeforeSubmit,
            ));
        }

        let waker = TokioPermitWaker::new();
        // `Arc::clone` cannot unsize a concrete `Arc<T>` to `Arc<dyn _>`; the
        // method form performs the clone-and-unsize coercion in one step.
        #[allow(
            clippy::clone_on_ref_ptr,
            reason = "Arc::clone cannot unsize concrete Arc<T> to Arc<dyn _>"
        )]
        let waker_port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
        match self
            .governor
            .admit_resolved(spec, capability, Some(waker_port))
        {
            AdmissionDecision::Admitted { permit_id } => {
                // The acquisition budget covers *every* wait on the way in,
                // including the time this thread spent blocked on the admission
                // mutex. An immediate `Admitted` that arrives after the budget
                // expired is still a budget overrun, and starting the caller's
                // work anyway is a side effect they asked not to have.
                if acquisition_budget_expired(opts, acquire_deadline) {
                    self.return_unstarted(permit_id);
                    return Err(GovernorError::Rejected(
                        AdmissionVerdict::PermitAcquireTimedOut {
                            retry_after_ms: None,
                        },
                    ));
                }
                Ok(permit_id)
            }
            AdmissionDecision::Rejected(verdict) => Err(GovernorError::Rejected(verdict)),
            AdmissionDecision::Queued { ticket } => {
                // While parked on the queue this future owns the ticket. If it is
                // dropped (external cancellation) or times out, the guard abandons
                // the ticket — removing it from the engine queue, or releasing it
                // if it was already promoted — so a cancelled-while-queued request
                // can never linger or strand a promoted-but-unclaimed permit.
                let mut guard = TicketGuard {
                    governor: Arc::clone(&self.governor),
                    ticket: Some(ticket),
                };
                let result = self
                    .await_promotion(ticket, &waker, opts, acquire_deadline)
                    .await;
                if result.is_ok() {
                    guard.disarm();
                }
                result
            }
        }
    }

    /// Wait for a queued ticket to be promoted, under the same acquisition
    /// budget the immediate path enforces (D09).
    ///
    /// A promotion that lands after the budget expired — including one that
    /// races the timeout itself — is unwound, not started: the caller asked for
    /// work to begin within a bound, and a permit that arrives outside it is a
    /// side effect they declined. The last look at the ticket on timeout exists
    /// to report *why* the wait ended when the engine already knows (reclaimed,
    /// invalid), never to sneak a late permit through.
    async fn await_promotion(
        &self,
        ticket: u64,
        waker: &TokioPermitWaker,
        opts: &SubmitOptions,
        acquire_deadline: Option<Instant>,
    ) -> Result<PermitId, GovernorError> {
        let cancel = opts.cancel.as_ref();
        loop {
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(GovernorError::Rejected(
                    AdmissionVerdict::CancelledBeforeSubmit,
                ));
            }
            // Read *before* claiming: a timeout must be able to say what the
            // request was waiting on, and once it is dequeued that is gone.
            let blocked_on = self.governor.pending_block_reason(ticket);
            match self.governor.claim(ticket) {
                ClaimOutcome::Ready(permit_id) => {
                    if acquisition_budget_expired(opts, acquire_deadline) {
                        self.return_unstarted(permit_id);
                        return Err(GovernorError::Rejected(acquire_timeout_verdict(blocked_on)));
                    }
                    return Ok(permit_id);
                }
                ClaimOutcome::Pending => {}
                ClaimOutcome::Terminal(reason) => {
                    return Err(GovernorError::TicketClaimTerminated { ticket, reason });
                }
                ClaimOutcome::Invalid => return Err(GovernorError::InvalidTicketClaim { ticket }),
            }
            tokio::select! {
                biased;
                () = cancellation_requested_v1(cancel), if cancel.is_some() => {
                    return Err(GovernorError::Rejected(
                        AdmissionVerdict::CancelledBeforeSubmit,
                    ));
                }
                () = waker.notified() => {}
                () = acquisition_deadline_reached_v1(acquire_deadline), if acquire_deadline.is_some() => {
                    if cancel.is_some_and(CancellationToken::is_cancelled) {
                        return Err(GovernorError::Rejected(
                            AdmissionVerdict::CancelledBeforeSubmit,
                        ));
                    }
                    // The budget is spent. A promotion that raced the timer is
                    // returned unstarted (D09); a ticket the engine already
                    // ended reports the engine's reason instead of a generic
                    // timeout, because that reason is the more useful truth.
                    match self.governor.claim(ticket) {
                        ClaimOutcome::Ready(permit_id) => self.return_unstarted(permit_id),
                        ClaimOutcome::Pending => {}
                        ClaimOutcome::Terminal(reason) => {
                            return Err(GovernorError::TicketClaimTerminated { ticket, reason });
                        }
                        ClaimOutcome::Invalid => {
                            return Err(GovernorError::InvalidTicketClaim { ticket });
                        }
                    }
                    return Err(GovernorError::Rejected(acquire_timeout_verdict(blocked_on)));
                }
            }
        }
    }

    /// Return a permit whose work will not start (budget expired after grant).
    ///
    /// Both outcomes are terminal for the caller and neither is a fault: the
    /// leak sweep may reclaim an unstarted (`DispatchReserved`) lease at any
    /// moment, and if it reached this permit first there is simply nothing left
    /// to return.
    fn return_unstarted(&self, permit_id: PermitId) {
        match self.governor.release(permit_id) {
            ReleaseOutcome::Released | ReleaseOutcome::UnknownPermit => {}
        }
    }

    /// Acquire the execution lease for a resolved plan.
    async fn acquire_execution_lease<E>(
        &self,
        spec: &TaskSpec,
        opts: &SubmitOptions,
        plan: &ResolvedExecutionPlan,
    ) -> Result<ExecutionLease, RunError<E>> {
        if opts.is_cancelled() {
            return Err(RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::CancelledBeforeSubmit,
            )));
        }
        let absolute_deadline = plan.absolute_deadline();
        if absolute_deadline.is_some() && !plan_supports_absolute_deadline(plan) {
            return Err(RunError::Governor(GovernorError::PolicyViolation(
                "absolute completion deadline requires cooperative async work".into(),
            )));
        }
        if absolute_deadline.is_some_and(|deadline| deadline <= std::time::Instant::now()) {
            return Err(RunError::Governor(GovernorError::DeadlineExceeded));
        }
        let relative_acquire_deadline = match opts.acquire_timeout {
            Some(timeout) => Some(Instant::now().checked_add(timeout).ok_or_else(|| {
                RunError::Governor(GovernorError::PolicyViolation(
                    "acquire timeout exceeds Instant range".into(),
                ))
            })?),
            None => None,
        };
        let absolute_acquire_deadline = absolute_deadline.map(Instant::from_std);
        let acquire_deadline = match (relative_acquire_deadline, absolute_acquire_deadline) {
            (Some(relative), Some(absolute)) => Some(relative.min(absolute)),
            (Some(relative), None) => Some(relative),
            (None, Some(absolute)) => Some(absolute),
            (None, None) => None,
        };
        let permit = self
            .acquire(spec, plan.capability, opts, acquire_deadline)
            .await
            .map_err(|error| absolute_acquire_error_v1(absolute_deadline, error))?;
        Ok(ExecutionLease {
            governor: Arc::clone(&self.governor),
            permit_id: permit,
            permit: Some(permit),
            dispatched: false,
        })
    }

    /// Resolve the plan for one submission against its class policy.
    fn plan(
        &self,
        spec: &TaskSpec,
        opts: &SubmitOptions,
        allowed: &[SubstrateHint],
        dispatch: DispatchKind,
    ) -> Result<ResolvedExecutionPlan, GovernorError> {
        ResolvedExecutionPlan::resolve(
            spec,
            opts,
            self.config.classes.get(&spec.class),
            allowed,
            dispatch,
        )
    }

    // ---- substrate entry points (with options) ----------------------------

    pub async fn run_io_with<T, E, Fut>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        fut: Fut,
    ) -> Result<T, RunError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        let plan = match self.plan(
            &spec,
            &opts,
            &[SubstrateHint::AsyncIo],
            DispatchKind::CallerFuture,
        ) {
            Ok(plan) => plan,
            Err(error) => return Err(RunError::Governor(error)),
        };
        let mut lease = self.acquire_execution_lease(&spec, &opts, &plan).await?;
        lease.advance(ExecutionPhase::Running)?;
        run_cancellable(plan.cancel.clone(), plan.deadline, async {
            fut.await.map_err(RunError::Task)
        })
        .await
    }

    pub fn run_blocking_with<T, E, F>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        job: F,
    ) -> RuntimeBoxFutureV1<'_, T, E>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        let plan = match self.plan(
            &spec,
            &opts,
            &[
                SubstrateHint::BlockingPool,
                SubstrateHint::LargeStackCapability,
                SubstrateHint::BackgroundOnly,
            ],
            DispatchKind::BlockingPool,
        ) {
            Ok(plan) => plan,
            Err(error) => return Box::pin(ready(Err(RunError::Governor(error)))),
        };
        let boxed_job: BlockingJobV1<T, E> = Box::new(job);
        Box::pin(async move {
            let lease = self.acquire_execution_lease(&spec, &opts, &plan).await?;
            match plan.dispatch {
                DispatchKind::DedicatedStackThread => {
                    self.run_blocking_on_dedicated_thread(&spec, boxed_job, lease, &plan)
                        .await
                }
                _ => self.run_blocking_on_pool(boxed_job, lease, &plan).await,
            }
        })
    }

    /// Blocking work on Tokio's blocking pool.
    ///
    /// A started `spawn_blocking` task cannot be aborted, so a run deadline here
    /// bounds the **caller's wait**, not the work. The worker keeps the lease
    /// until it actually returns: telling the caller "deadline exceeded" while
    /// silently freeing capacity a live thread still occupies would be a lie the
    /// next admission pays for.
    async fn run_blocking_on_pool<T, E>(
        &self,
        job: BlockingJobV1<T, E>,
        lease: ExecutionLease,
        plan: &ResolvedExecutionPlan,
    ) -> Result<T, RunError<E>>
    where
        E: Send + 'static,
        T: Send + 'static,
    {
        let (started_tx, started_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let mut lease = lease;
        lease.advance(ExecutionPhase::Accepted)?;
        tokio::task::spawn_blocking(move || {
            run_detached_job(job, lease, started_tx, done_tx, "blocking worker");
        });
        await_detached(
            started_rx,
            done_rx,
            plan.cancel.clone(),
            plan.deadline,
            "blocking worker",
        )
        .await
    }

    /// Blocking work on a dedicated OS thread sized by the spec's stack request.
    async fn run_blocking_on_dedicated_thread<T, E>(
        &self,
        spec: &TaskSpec,
        job: BlockingJobV1<T, E>,
        lease: ExecutionLease,
        plan: &ResolvedExecutionPlan,
    ) -> Result<T, RunError<E>>
    where
        E: Send + 'static,
        T: Send + 'static,
    {
        let stack_size_bytes = requested_stack_size_bytes_v1(spec)?;
        let thread_name = requested_stack_thread_name_v1(spec);
        let tokio_handle = tokio::runtime::Handle::try_current().ok();
        let (started_tx, started_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let mut lease = lease;
        lease.advance(ExecutionPhase::Accepted)?;
        let spawned = std::thread::Builder::new()
            .name(thread_name)
            .stack_size(stack_size_bytes)
            .spawn(move || {
                let _tokio_runtime_context = tokio_handle.as_ref().map(|handle| handle.enter());
                run_detached_job(job, lease, started_tx, done_tx, "large-stack worker");
            });
        // Setup failed before the worker existed, so the lease inside the
        // closure was never handed over: it is dropped with the closure and the
        // permit returns. Nothing started, nothing to reconcile.
        if let Err(error) = spawned {
            return Err(worker_unavailable_v1("large-stack worker", &error));
        }
        await_detached(
            started_rx,
            done_rx,
            plan.cancel.clone(),
            plan.deadline,
            "large-stack worker",
        )
        .await
    }

    /// Run an async future on an owned current-thread Tokio runtime hosted by
    /// the requested-stack worker. The factory is invoked on that worker so the
    /// root future and Tokio children never begin on the caller runtime.
    pub async fn run_async_with_requested_stack<T, E, F, Fut>(
        &self,
        spec: TaskSpec,
        make_future: F,
    ) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        self.run_async_with_requested_stack_with(spec, SubmitOptions::unbounded(), make_future)
            .await
    }

    /// [`Self::run_async_with_requested_stack`] with explicit admission and
    /// cooperative cancellation controls.
    pub async fn run_async_with_requested_stack_with<T, E, F, Fut>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        make_future: F,
    ) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        let plan = self
            .plan(
                &spec,
                &opts,
                &[SubstrateHint::LargeStackCapability],
                DispatchKind::DedicatedStackRuntime,
            )
            .map_err(RunError::Governor)?;
        if spec.requested_stack_size_bytes().is_none() {
            return Err(RunError::Governor(GovernorError::PolicyViolation(
                "requested stack size missing on async large-stack path".into(),
            )));
        }
        let make_future: RequestedStackAsyncFutureFactoryV1<T, E> =
            Box::new(move || Box::pin(make_future()));
        let lease = self.acquire_execution_lease(&spec, &opts, &plan).await?;
        self.run_async_with_requested_stack_v1(&spec, make_future, &plan, lease)
            .await
    }

    async fn run_async_with_requested_stack_v1<T, E>(
        &self,
        spec: &TaskSpec,
        make_future: RequestedStackAsyncFutureFactoryV1<T, E>,
        plan: &ResolvedExecutionPlan,
        lease: ExecutionLease,
    ) -> Result<T, RunError<E>>
    where
        E: Send + 'static,
        T: Send + 'static,
    {
        /// What the worker sends back.
        ///
        /// The two variants exist because the success path and the terminal path
        /// want opposite orderings, and collapsing them breaks one of them:
        ///
        /// * `Completed` is sent **after** the owned runtime is torn down, and
        ///   carries the lease, so a caller that sees `Ok` sees capacity already
        ///   released.
        /// * `Terminated` is sent **before** teardown and carries no lease, so a
        ///   deadline reply — or a panic report, or a worker that could not be
        ///   set up — is not held hostage by a blocking child the owned runtime
        ///   must wait for, while that child keeps the work charged.
        enum AsyncThreadOutcome<T, E> {
            Completed(Result<T, RunError<E>>, ExecutionLease),
            Terminated(RunError<E>),
        }

        let stack_size_bytes = requested_stack_size_bytes_v1(spec)?;
        let thread_name = requested_stack_thread_name_v1(spec);
        let cancel = plan.cancel.clone();
        let deadline = plan.deadline;
        let (tx, rx) = oneshot::channel::<AsyncThreadOutcome<T, E>>();
        let mut lease = lease;
        lease.advance(ExecutionPhase::Accepted)?;
        let spawned = std::thread::Builder::new()
            .name(thread_name)
            .stack_size(stack_size_bytes)
            .spawn(move || {
                // The dedicated worker, not the caller future, owns governance
                // until the worker-local runtime and root future have terminated.
                let mut lease = lease;
                let mut tx = Some(tx);

                // The runtime is built *outside* the panic boundary and owned by
                // this frame. Were it a local of the guarded closure, a panic in
                // the root future would tear it down during the unwind — and
                // that teardown blocks on any child the runtime cannot abort —
                // before the caller heard `Panicked`. Every terminal reply below
                // is sent first and teardown happens after, under the lease.
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        // A failed send here means the caller already gave up,
                        // which is an expected delivery outcome, not a loss.
                        let _undelivered = tx.take().map(|tx| {
                            tx.send(AsyncThreadOutcome::Terminated(RunError::Governor(
                                GovernorError::WorkerUnavailable {
                                    context: "requested-stack async worker".into(),
                                    detail: error.to_string(),
                                },
                            )))
                        });
                        return;
                    }
                };
                if let Err(error) = lease.advance(ExecutionPhase::Running) {
                    let _undelivered = tx.take().map(|tx| {
                        tx.send(AsyncThreadOutcome::Terminated(RunError::Governor(error)))
                    });
                    drop(runtime);
                    return;
                }

                let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(async {
                        tokio::select! {
                            biased;
                            () = closed(tx.as_mut()) => {
                                Err(RunError::Governor(GovernorError::Cancelled))
                            }
                            result = async move {
                                run_cancellable(
                                    cancel,
                                    deadline,
                                    async move { make_future().await.map_err(RunError::Task) },
                                )
                                .await
                            } => result,
                        }
                    })
                }));

                let result = match outcome {
                    Ok(result) => result,
                    Err(_panic) => {
                        // Reply first; the teardown below may block on children
                        // the runtime cannot abort, and the caller's answer must
                        // not wait for them. The work stays charged meanwhile.
                        let _undelivered = tx.take().map(|tx| {
                            tx.send(AsyncThreadOutcome::Terminated(RunError::Governor(
                                GovernorError::WorkerPanicked {
                                    context: "requested-stack async worker".into(),
                                },
                            )))
                        });
                        drop(runtime);
                        drop(lease);
                        return;
                    }
                };

                if let Err(error) = lease.advance(ExecutionPhase::CleanupPending) {
                    let _undelivered = tx.take().map(|tx| {
                        tx.send(AsyncThreadOutcome::Terminated(RunError::Governor(error)))
                    });
                    drop(runtime);
                    drop(lease);
                    return;
                }

                // A terminal governor outcome (deadline, cancel) is the caller's
                // answer, and it must not wait for teardown: dropping the owned
                // runtime blocks on any child it cannot abort. Teardown still
                // happens, still under this lease, so the capacity stays charged
                // for exactly as long as the work really occupies it.
                match result {
                    Err(
                        error @ RunError::Governor(
                            GovernorError::Cancelled | GovernorError::DeadlineExceeded,
                        ),
                    ) => {
                        let _undelivered = tx
                            .take()
                            .map(|tx| tx.send(AsyncThreadOutcome::Terminated(error)));
                        drop(runtime);
                        drop(lease);
                    }
                    completed => {
                        // Normal completion: tear down first, then hand the
                        // result and the lease over together, so a caller that
                        // can see the value sees released capacity.
                        drop(runtime);
                        let _undelivered = tx
                            .take()
                            .map(|tx| tx.send(AsyncThreadOutcome::Completed(completed, lease)));
                    }
                }
            });
        if let Err(error) = spawned {
            return Err(worker_unavailable_v1(
                "requested-stack async worker",
                &error,
            ));
        }
        match rx.await {
            Ok(AsyncThreadOutcome::Completed(result, lease)) => {
                // Release before returning: a result the caller can see is a
                // result whose capacity is already back in the pool.
                drop(lease);
                result
            }
            Ok(AsyncThreadOutcome::Terminated(error)) => Err(error),
            Err(_) => Err(RunError::Governor(GovernorError::JobAbandoned {
                context: "requested-stack async worker".into(),
            })),
        }
    }

    pub async fn run_cpu_with<T, E, F>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        job: F,
    ) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        let plan = match self.plan(
            &spec,
            &opts,
            &[SubstrateHint::SharedCpuExecutor],
            DispatchKind::CpuExecutor,
        ) {
            Ok(plan) => plan,
            Err(error) => return Err(RunError::Governor(error)),
        };
        let cpu = Arc::clone(&self.cpu);
        let lease = self.acquire_execution_lease(&spec, &opts, &plan).await?;
        let (started_tx, started_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let boxed_job: BlockingJobV1<T, E> = Box::new(job);
        let mut lease = lease;
        lease.advance(ExecutionPhase::Accepted)?;
        let work = Box::new(move || {
            run_detached_job(boxed_job, lease, started_tx, done_tx, "cpu worker");
        });
        if panic::catch_unwind(AssertUnwindSafe(|| cpu.spawn(work))).is_err() {
            // The adapter took ownership of the closure before unwinding, so the
            // lease went with it: either the closure is dropped by the unwind
            // (permit returns) or it is still queued and will run under its
            // lease. Re-submitting could execute the caller's job twice, so this
            // path reports and stops.
            return Err(RunError::Governor(GovernorError::WorkerPanicked {
                context: "cpu executor spawn".into(),
            }));
        }
        await_detached(
            started_rx,
            done_rx,
            plan.cancel.clone(),
            plan.deadline,
            "cpu worker",
        )
        .await
    }

    pub async fn run_local_with<T, E, Fut>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        fut: Fut,
    ) -> Result<T, RunError<E>>
    where
        Fut: Future<Output = Result<T, E>> + 'static,
        T: 'static,
    {
        // `run_local` is the local-runtime exception, not a general async path.
        let plan = match self.plan(
            &spec,
            &opts,
            &[SubstrateHint::LocalRuntime],
            DispatchKind::CallerFuture,
        ) {
            Ok(plan) => plan,
            Err(error) => return Err(RunError::Governor(error)),
        };
        // The non-`Send` future never leaves this task: only the permit metadata
        // is governed centrally, so a local payload keeps its caller affinity.
        let mut lease = self.acquire_execution_lease(&spec, &opts, &plan).await?;
        lease.advance(ExecutionPhase::Running)?;
        let work = async move {
            tokio::task::LocalSet::new()
                .run_until(fut)
                .await
                .map_err(RunError::Task)
        };
        run_cancellable(plan.cancel.clone(), plan.deadline, work).await
    }
}

/// Whether a plan's dispatch can honor an absolute completion deadline.
///
/// Only cooperatively-polled async work can be stopped mid-run. Accepting a
/// `CompleteBy` on a synchronous path would promise an abort that cannot happen.
fn plan_supports_absolute_deadline(plan: &ResolvedExecutionPlan) -> bool {
    match plan.dispatch {
        // The requested-stack *async* path runs a cooperative future on its own
        // runtime, so it can be stopped mid-run exactly like a caller future.
        // Every synchronous dispatch — the blocking pool, the CPU executor, and
        // a dedicated thread running a blocking job — cannot, and must not
        // accept a promise to.
        DispatchKind::CallerFuture | DispatchKind::DedicatedStackRuntime => true,
        DispatchKind::BlockingPool
        | DispatchKind::CpuExecutor
        | DispatchKind::DedicatedStackThread => false,
    }
}

/// Whether the acquisition budget is already spent.
///
/// `Some(Duration::ZERO)` keeps its published meaning — *try, do not wait* — so
/// an uncontended immediate acquisition still succeeds. Any positive budget is
/// enforced against the clock, which is what makes it cover a blocking wait on
/// the admission mutex rather than only the asynchronous queue wait.
fn acquisition_budget_expired(opts: &SubmitOptions, acquire_deadline: Option<Instant>) -> bool {
    if opts.acquire_timeout == Some(Duration::ZERO) {
        return false;
    }
    acquire_deadline.is_some_and(|deadline| Instant::now() >= deadline)
}

/// The timeout verdict matching what the request was actually waiting on.
fn acquire_timeout_verdict(blocked_on: Option<CapacityBlock>) -> AdmissionVerdict {
    match blocked_on {
        Some(CapacityBlock::Capability) => AdmissionVerdict::SubstratePoolTimedOut {
            retry_after_ms: None,
        },
        _ => AdmissionVerdict::PermitAcquireTimedOut {
            retry_after_ms: None,
        },
    }
}

/// What a detached worker sends back when it finishes.
struct DetachedOutcome<T, E> {
    result: Result<T, RunError<E>>,
    started_at: std::time::Instant,
    completed_at: std::time::Instant,
    /// Custody travels with the result, so a caller that observes completion has
    /// already released the capacity.
    lease: ExecutionLease,
}

/// The shared worker body for every detached synchronous dispatch.
///
/// One wrapper so the start timestamp, the phase transitions, the panic
/// boundary, and the lease handoff are identical on the blocking pool, the CPU
/// executor port, and a dedicated stack thread. The differences between those
/// three are *where* the closure runs, not what governance it owes.
fn run_detached_job<T, E>(
    job: BlockingJobV1<T, E>,
    lease: ExecutionLease,
    started_tx: oneshot::Sender<std::time::Instant>,
    done_tx: oneshot::Sender<DetachedOutcome<T, E>>,
    context: &'static str,
) {
    // The worker's own clock is the authority for a relative run budget. Timing
    // from submission instead would charge the job for queue time it did not
    // spend running — and would miss the case where the adapter runs it inline,
    // where "submitted" and "finished" are the same instant.
    let started_at = std::time::Instant::now();
    let mut lease = lease;
    // A lease the engine no longer backs must not run: the sweep reclaimed it
    // while the job sat in the executor's queue. Report, do not execute.
    let result = if let Err(error) = lease.advance(ExecutionPhase::Running) {
        Err(RunError::Governor(error))
    } else {
        // The caller may already have stopped waiting; that is expected, and
        // the completion message below is what actually carries custody back.
        let _undelivered = started_tx.send(started_at);
        let outcome = panic::catch_unwind(AssertUnwindSafe(job));
        match (outcome, lease.advance(ExecutionPhase::CleanupPending)) {
            (Ok(Ok(value)), Ok(())) => Ok(value),
            (Ok(Err(error)), Ok(())) => Err(RunError::Task(error)),
            (Err(_panic), Ok(())) => Err(RunError::Governor(GovernorError::WorkerPanicked {
                context: context.into(),
            })),
            // Unreachable once `Running` was accepted (nothing but this lease
            // can end the permit), reported anyway rather than pretending the
            // gauges are right.
            (_, Err(error)) => Err(RunError::Governor(error)),
        }
    };
    let completed_at = std::time::Instant::now();
    // A failed send means the caller stopped waiting (timeout, cancel, drop).
    // That is a normal, expected delivery outcome — not a lost message — and
    // this worker is then the owner that releases the lease, which happens when
    // the returned payload is dropped here.
    let _undelivered = done_tx.send(DetachedOutcome {
        result,
        started_at,
        completed_at,
        lease,
    });
}

/// Await a detached worker, racing its completion against cancellation and the
/// run deadline.
///
/// The deadline for `RunFor` is anchored to the worker's **own** start, reported
/// by the worker itself. A completion that lands past that anchor is a deadline
/// miss even if the timer has not fired yet, so an inline executor cannot return
/// a late success as `Ok`. Ties go to the deadline.
async fn await_detached<T, E>(
    started_rx: oneshot::Receiver<std::time::Instant>,
    done_rx: oneshot::Receiver<DetachedOutcome<T, E>>,
    cancel: Option<CancellationToken>,
    deadline: Option<SubmissionDeadline>,
    context: &'static str,
) -> Result<T, RunError<E>> {
    let cancelled = async {
        match &cancel {
            Some(token) => token.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    let expiry = async {
        match deadline {
            None => std::future::pending::<()>().await,
            Some(SubmissionDeadline::CompleteBy(instant)) => {
                tokio::time::sleep_until(Instant::from_std(instant)).await;
            }
            Some(SubmissionDeadline::RunFor(budget)) => match started_rx.await {
                // The budget starts when the work does.
                Ok(started_at) => match started_at.checked_add(budget) {
                    Some(expires_at) => {
                        tokio::time::sleep_until(Instant::from_std(expires_at)).await;
                    }
                    None => std::future::pending::<()>().await,
                },
                // The worker never started and never will; completion decides.
                Err(_) => std::future::pending::<()>().await,
            },
        }
    };
    let mut done_rx = done_rx;
    tokio::select! {
        biased;
        () = cancelled, if cancel.is_some() => Err(RunError::Governor(GovernorError::Cancelled)),
        () = expiry, if deadline.is_some() => Err(RunError::Governor(GovernorError::DeadlineExceeded)),
        received = &mut done_rx => match received {
            Ok(outcome) => {
                let expired = match deadline {
                    Some(SubmissionDeadline::RunFor(budget)) => outcome
                        .started_at
                        .checked_add(budget)
                        .is_some_and(|expires_at| outcome.completed_at >= expires_at),
                    Some(SubmissionDeadline::CompleteBy(instant)) => {
                        outcome.completed_at >= instant
                    }
                    None => false,
                };
                let DetachedOutcome { result, lease, .. } = outcome;
                // Release fence: drop custody before the caller is answered.
                drop(lease);
                if expired {
                    Err(RunError::Governor(GovernorError::DeadlineExceeded))
                } else {
                    result
                }
            }
            Err(_) => Err(RunError::Governor(GovernorError::JobAbandoned {
                context: context.into(),
            })),
        },
    }
}

/// Resolve when the result receiver is gone (the caller stopped waiting).
async fn closed<T>(tx: Option<&mut oneshot::Sender<T>>) {
    match tx {
        Some(tx) => tx.closed().await,
        None => std::future::pending::<()>().await,
    }
}

fn absolute_acquire_error_v1<E>(
    absolute_deadline: Option<std::time::Instant>,
    error: GovernorError,
) -> RunError<E> {
    let acquisition_timed_out = matches!(
        &error,
        GovernorError::Rejected(
            AdmissionVerdict::PermitAcquireTimedOut { .. }
                | AdmissionVerdict::SubstratePoolTimedOut { .. }
        )
    );
    if acquisition_timed_out
        && absolute_deadline.is_some_and(|deadline| deadline <= std::time::Instant::now())
    {
        RunError::Governor(GovernorError::DeadlineExceeded)
    } else {
        RunError::Governor(error)
    }
}

/// The largest stack a submission may request: 16 GiB.
///
/// `std::thread::Builder::stack_size` is handed to the OS after `std` rounds it
/// up to a page boundary; near `usize::MAX` that arithmetic overflows, and what
/// happens next depends on how `std` was built (a release `std` wraps and the
/// OS refuses the mangled size; a debug `std`, as under `-Zbuild-std` with a
/// sanitizer, panics inside the spawn). A request that large is a caller error
/// in every case, so the host refuses it here, deterministically, before `std`
/// is involved. Real OS refusals of *sane* sizes remain `WorkerUnavailable`.
pub const MAX_REQUESTED_STACK_BYTES: u64 = 1 << 34;

fn requested_stack_size_bytes_v1<E>(spec: &TaskSpec) -> Result<usize, RunError<E>> {
    let requested = spec.requested_stack_size_bytes().ok_or_else(|| {
        RunError::Governor(GovernorError::PolicyViolation(
            "requested stack size missing on large-stack path".into(),
        ))
    })?;
    if requested > MAX_REQUESTED_STACK_BYTES {
        return Err(RunError::Governor(GovernorError::PolicyViolation(
            format!(
                "requested stack size {requested} bytes exceeds the supported maximum \
                 {MAX_REQUESTED_STACK_BYTES}"
            )
            .into(),
        )));
    }
    usize::try_from(requested).map_err(|_too_large| {
        RunError::Governor(GovernorError::PolicyViolation(
            "requested stack size does not fit usize".into(),
        ))
    })
}

/// Build the OS thread label for a dedicated worker.
///
/// A class name is a *semantic* identifier chosen by the caller; an OS thread
/// name is a constrained C string. Interpolating one into the other unchecked
/// turns an accepted class like `"a\0b"` into a panic inside `thread::Builder`
/// — outside the worker's own unwind boundary, so it surfaces as a caller-task
/// panic rather than a typed error. The label is derived, bounded, and lossy on
/// purpose; class identity itself is never modified.
fn requested_stack_thread_name_v1(spec: &TaskSpec) -> String {
    const PREFIX: &str = "taskmesh.large_stack:";
    let mut name = String::with_capacity(MAX_THREAD_NAME_BYTES);
    name.push_str(PREFIX);
    for ch in spec.class.as_str().chars() {
        if name.len() >= MAX_THREAD_NAME_BYTES {
            break;
        }
        // Keep the label ASCII-safe and NUL-free: everything else becomes '_',
        // so a label is always constructible from any accepted class.
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            name.push(ch);
        } else {
            name.push('_');
        }
    }
    name
}

fn worker_unavailable_v1<E>(context: &'static str, error: &std::io::Error) -> RunError<E> {
    RunError::Governor(GovernorError::WorkerUnavailable {
        context: context.into(),
        detail: error.to_string(),
    })
}

impl Runtime for TokioRuntime {
    async fn run_io<T, E, Fut>(&self, spec: TaskSpec, fut: Fut) -> Result<T, RunError<E>>
    where
        Fut: Future<Output = Result<T, E>> + Send,
    {
        self.run_io_with(spec, SubmitOptions::unbounded(), fut)
            .await
    }

    async fn run_blocking<T, E, F>(&self, spec: TaskSpec, job: F) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        self.run_blocking_with(spec, SubmitOptions::unbounded(), job)
            .await
    }

    async fn run_cpu<T, E, F>(&self, spec: TaskSpec, job: F) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        self.run_cpu_with(spec, SubmitOptions::unbounded(), job)
            .await
    }

    async fn run_local<T, E, Fut>(&self, spec: TaskSpec, fut: Fut) -> Result<T, RunError<E>>
    where
        Fut: Future<Output = Result<T, E>> + 'static,
        T: 'static,
    {
        self.run_local_with(spec, SubmitOptions::unbounded(), fut)
            .await
    }

    fn snapshot(&self) -> Snapshot {
        self.governor.snapshot()
    }
}

/// Run `work`, racing it against an optional cooperative cancel token and an
/// optional run deadline. Shared by every async run path (`run_io`, `run_local`,
/// and the requested-stack async worker), so cancellation/deadline semantics are
/// identical everywhere.
async fn run_cancellable<T, E>(
    token: Option<CancellationToken>,
    deadline: Option<SubmissionDeadline>,
    work: impl Future<Output = Result<T, RunError<E>>>,
) -> Result<T, RunError<E>> {
    if token.is_none() && deadline.is_none() {
        return work.await;
    }
    let completion_deadline = match deadline {
        Some(SubmissionDeadline::RunFor(duration)) => Some(
            std::time::Instant::now()
                .checked_add(duration)
                .ok_or_else(|| {
                    RunError::Governor(GovernorError::PolicyViolation(
                        "relative run deadline exceeds Instant range".into(),
                    ))
                })?,
        ),
        Some(SubmissionDeadline::CompleteBy(instant)) => Some(instant),
        None => None,
    };
    let cancelled = async {
        match &token {
            Some(t) => t.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    let timed = async {
        match completion_deadline {
            Some(instant) => tokio::time::sleep_until(Instant::from_std(instant)).await,
            None => std::future::pending::<()>().await,
        }
    };
    let timed_work = async move {
        let outcome = work.await;
        (std::time::Instant::now(), outcome)
    };
    tokio::pin!(timed_work);
    tokio::select! {
        biased;
        () = cancelled, if token.is_some() => Err(RunError::Governor(GovernorError::Cancelled)),
        () = timed, if completion_deadline.is_some() => Err(RunError::Governor(GovernorError::DeadlineExceeded)),
        (completed_at, outcome) = &mut timed_work => {
            if completion_deadline.is_some_and(|deadline| completed_at >= deadline) {
                Err(RunError::Governor(GovernorError::DeadlineExceeded))
            } else {
                outcome
            }
        },
    }
}

async fn cancellation_requested_v1(token: Option<&CancellationToken>) {
    match token {
        Some(token) => token.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

async fn acquisition_deadline_reached_v1(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

/// Owns one execution's governed capacity: the semantic permit and, through it,
/// the capability-pool slot the engine reserved in the same transition.
///
/// Whoever holds this value is the execution's owner. Detached workers take it
/// so caller cancellation cannot report capacity as free before the worker has
/// actually terminated.
struct ExecutionLease {
    governor: Arc<Governor>,
    permit_id: PermitId,
    /// `None` once the permit has been returned — or once the engine reported
    /// it gone, so a later drop does not release it a second time.
    permit: Option<PermitId>,
    /// Set once the engine accepted a phase past `DispatchReserved`. From then
    /// on the leak sweep leaves the permit alone, so the only way its release
    /// can find nothing is a double release.
    dispatched: bool,
}

impl ExecutionLease {
    /// Declare the execution's next ownership phase.
    ///
    /// Fails, and disarms the lease, when the engine no longer holds a live
    /// permit for it: the leak sweep reclaimed the lease before dispatch. The
    /// work must not start on capacity the engine has already reassigned, and
    /// the failure says so instead of letting the phase gauges quietly diverge
    /// from a worker that ran anyway.
    fn advance(&mut self, phase: ExecutionPhase) -> Result<(), GovernorError> {
        let Some(permit) = self.permit else {
            return Err(GovernorError::LeaseReclaimed {
                permit_id: self.permit_id,
            });
        };
        if self.governor.advance_phase(permit, phase) {
            self.dispatched |= phase != ExecutionPhase::DispatchReserved;
            return Ok(());
        }
        if self.governor.phase(permit).is_none() {
            self.permit = None;
            return Err(GovernorError::LeaseReclaimed { permit_id: permit });
        }
        // The permit is live and refused the move: the host declared a phase
        // that is not later than the current one. The host's sequence is fixed
        // (`Accepted` → `Running` → `CleanupPending`), so this is a programming
        // error, reported rather than ignored.
        Err(GovernorError::PolicyViolation(
            format!("execution phase {phase:?} does not advance permit {permit}").into(),
        ))
    }
}

impl Drop for ExecutionLease {
    fn drop(&mut self) {
        if let Some(permit) = self.permit.take() {
            match self.governor.release(permit) {
                ReleaseOutcome::Released => {}
                ReleaseOutcome::UnknownPermit => {
                    // Legitimate only before dispatch, where the leak sweep can
                    // reclaim a stale `DispatchReserved` lease under us. After
                    // dispatch nothing but this lease can end the permit, so a
                    // miss here is a double release — an invariant the test
                    // suite must see. (Not asserted while unwinding: a panic in
                    // a destructor during a panic aborts the process.)
                    if !std::thread::panicking() {
                        debug_assert!(
                            !self.dispatched,
                            "permit {permit} released twice: a dispatched lease found no permit"
                        );
                    }
                }
            }
        }
    }
}

/// Owns a queued ticket while the caller waits for promotion. On drop (caller
/// cancelled / timed out) it abandons the ticket: removing it from the engine
/// queue, or releasing it if it was already promoted. Disarmed once the permit
/// is successfully claimed (ownership then passes to an [`ExecutionLease`]).
struct TicketGuard {
    governor: Arc<Governor>,
    ticket: Option<u64>,
}

impl TicketGuard {
    fn disarm(&mut self) {
        self.ticket = None;
    }
}

impl Drop for TicketGuard {
    fn drop(&mut self) {
        if let Some(ticket) = self.ticket {
            self.governor.abandon(ticket);
        }
    }
}
