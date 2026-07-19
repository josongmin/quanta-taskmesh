//! `TokioRuntime` — the host adapter implementing the [`Runtime`] driving port.
//!
//! Every path acquires one execution lease: a governor permit plus the physical
//! substrate slot. Async work owns the lease in its caller future; detached
//! synchronous workers own it in their worker closure until actual termination.
//! Governor rejection and task failure stay distinct via [`RunError`]; a
//! `spawn_blocking`/worker join failure is a governor/runtime-side error, never a
//! task error.

use std::collections::BTreeMap;
use std::future::{ready, Future};
use std::panic::{self, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use taskmesh_contract::{
    AdmissionVerdict, CancellationPolicy, CpuExecutor, GovernorError, RunError, Runtime, Snapshot,
    SubstrateHint, SubstrateRecord, TaskClass, TaskSpec, TopologyConfig,
};
use taskmesh_engine::{AdmissionDecision, Governor, PermitId};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use crate::adapters::TokioPermitWaker;
use crate::executor::{SubmissionDeadline, SubmitOptions};
use crate::RuntimeConfig;

type BlockingJobV1<T, E> = Box<dyn FnOnce() -> Result<T, E> + Send + 'static>;
type RequestedStackAsyncFutureV1<T, E> = Pin<Box<dyn Future<Output = Result<T, E>> + 'static>>;
type RequestedStackAsyncFutureFactoryV1<T, E> =
    Box<dyn FnOnce() -> RequestedStackAsyncFutureV1<T, E> + Send + 'static>;
type RuntimeBoxFutureV1<'a, T, E> =
    Pin<Box<dyn Future<Output = Result<T, RunError<E>>> + Send + 'a>>;

#[derive(Clone, Copy)]
enum AbsoluteDeadlineCapabilityV1 {
    CooperativeAsync,
    Unsupported,
}

/// A governed, Tokio-hosted runtime. Cheap to clone (shared `Arc` internals).
#[derive(Clone)]
pub struct TokioRuntime {
    config: RuntimeConfig,
    governor: Arc<Governor>,
    cpu: Arc<dyn CpuExecutor>,
    substrates: Arc<SubstrateGates>,
}

impl TokioRuntime {
    async fn run_async_with_requested_stack_v1<T, E>(
        &self,
        spec: &TaskSpec,
        make_future: RequestedStackAsyncFutureFactoryV1<T, E>,
        cancel: Option<CancellationToken>,
        deadline: Option<SubmissionDeadline>,
        lease: ExecutionLease,
    ) -> Result<T, RunError<E>>
    where
        E: Send + 'static,
        T: Send + 'static,
    {
        enum AsyncThreadOutcome<T, E> {
            Completed(Result<T, RunError<E>>),
            RuntimeInitializationFailed(String),
            Panicked,
        }

        let stack_size_bytes = requested_stack_size_bytes_v1(spec)?;
        let thread_name = requested_stack_thread_name_v1(spec);
        let (tx, rx) = oneshot::channel::<AsyncThreadOutcome<T, E>>();
        std::thread::Builder::new()
            .name(thread_name)
            .stack_size(stack_size_bytes)
            .spawn(move || {
                // The dedicated worker, not the caller future, owns governance
                // until the worker-local runtime and root future have terminated.
                let _lease = lease;
                let mut tx = tx;
                let outcome = match panic::catch_unwind(AssertUnwindSafe(|| {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            return AsyncThreadOutcome::RuntimeInitializationFailed(
                                error.to_string(),
                            );
                        }
                    };
                    AsyncThreadOutcome::Completed(runtime.block_on(async {
                        tokio::select! {
                            biased;
                            () = tx.closed() => {
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
                    }))
                })) {
                    Ok(outcome) => outcome,
                    Err(_) => AsyncThreadOutcome::Panicked,
                };
                let _ = tx.send(outcome);
            })
            .map_err(|error| requested_stack_spawn_error_v1(error.to_string()))?;
        match rx.await {
            Ok(AsyncThreadOutcome::Completed(result)) => result,
            Ok(AsyncThreadOutcome::RuntimeInitializationFailed(message)) => {
                Err(RunError::Governor(GovernorError::PolicyViolation(
                    format!("requested-stack async runtime initialization failed: {message}")
                        .into(),
                )))
            }
            Ok(AsyncThreadOutcome::Panicked) => Err(RunError::Governor(
                GovernorError::PolicyViolation("requested-stack async worker panicked".into()),
            )),
            Err(_) => Err(RunError::Governor(GovernorError::PolicyViolation(
                "requested-stack async worker dropped result".into(),
            ))),
        }
    }

    async fn run_blocking_with_requested_stack_v1<T, E>(
        &self,
        spec: &TaskSpec,
        job: BlockingJobV1<T, E>,
        lease: ExecutionLease,
    ) -> Result<T, RunError<E>>
    where
        E: Send + 'static,
        T: Send + 'static,
    {
        enum BlockingThreadOutcome<T, E> {
            Completed(Result<T, E>),
            Panicked,
        }

        let stack_size_bytes = requested_stack_size_bytes_v1(spec)?;
        let thread_name = requested_stack_thread_name_v1(spec);
        let tokio_handle = tokio::runtime::Handle::try_current().ok();
        let (tx, rx) = oneshot::channel::<BlockingThreadOutcome<T, E>>();
        std::thread::Builder::new()
            .name(thread_name)
            .stack_size(stack_size_bytes)
            .spawn(move || {
                let _lease = lease;
                let _tokio_runtime_context = tokio_handle.as_ref().map(|handle| handle.enter());
                let outcome = match panic::catch_unwind(AssertUnwindSafe(job)) {
                    Ok(result) => BlockingThreadOutcome::Completed(result),
                    Err(_) => BlockingThreadOutcome::Panicked,
                };
                let _ = tx.send(outcome);
            })
            .map_err(|error| requested_stack_spawn_error_v1(error.to_string()))?;
        match rx.await {
            Ok(BlockingThreadOutcome::Completed(Ok(value))) => Ok(value),
            Ok(BlockingThreadOutcome::Completed(Err(error))) => Err(RunError::Task(error)),
            Ok(BlockingThreadOutcome::Panicked) => Err(RunError::Governor(
                GovernorError::PolicyViolation("large-stack worker panicked".into()),
            )),
            Err(_) => Err(RunError::Governor(GovernorError::PolicyViolation(
                "large-stack worker dropped result".into(),
            ))),
        }
    }

    pub(crate) fn new(
        config: RuntimeConfig,
        governor: Arc<Governor>,
        cpu: Arc<dyn CpuExecutor>,
    ) -> Self {
        let substrates = Arc::new(SubstrateGates::from_inventory(
            &config.topology,
            &config.substrates,
        ));
        Self {
            config,
            governor,
            cpu,
            substrates,
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

    // ---- acquire/release core --------------------------------------------

    /// Acquire a permit for `spec`, honoring pre-submit cancel and a bounded
    /// acquire wait. Queued requests park on a [`TokioPermitWaker`] and are woken
    /// on promotion — no polling.
    async fn acquire(
        &self,
        spec: &TaskSpec,
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
        match self.governor.admit_waitable(spec, waker_port) {
            AdmissionDecision::Admitted { permit_id } => Ok(permit_id),
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
                    .await_promotion(ticket, &waker, opts.cancel.as_ref(), acquire_deadline)
                    .await;
                if result.is_ok() {
                    guard.disarm();
                }
                result
            }
        }
    }

    async fn await_promotion(
        &self,
        ticket: u64,
        waker: &TokioPermitWaker,
        cancel: Option<&CancellationToken>,
        acquire_deadline: Option<Instant>,
    ) -> Result<PermitId, GovernorError> {
        loop {
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(GovernorError::Rejected(
                    AdmissionVerdict::CancelledBeforeSubmit,
                ));
            }
            if let Some(permit_id) = self.governor.claim(ticket) {
                return Ok(permit_id);
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
                    // Last-chance claim in case promotion raced the timeout;
                    // otherwise the caller's `TicketGuard` abandons the ticket.
                    if let Some(permit_id) = self.governor.claim(ticket) {
                        return Ok(permit_id);
                    }
                    return Err(GovernorError::Rejected(
                        AdmissionVerdict::PermitAcquireTimedOut {
                            retry_after_ms: None,
                        },
                    ));
                }
            }
        }
    }

    async fn acquire_execution_lease<E>(
        &self,
        spec: &TaskSpec,
        opts: &SubmitOptions,
        absolute_deadline_capability: AbsoluteDeadlineCapabilityV1,
    ) -> Result<ExecutionLease, RunError<E>> {
        if opts.is_cancelled() {
            return Err(RunError::Governor(GovernorError::Rejected(
                AdmissionVerdict::CancelledBeforeSubmit,
            )));
        }
        let absolute_deadline = self.absolute_deadline(&spec.class, opts);
        if absolute_deadline.is_some()
            && matches!(
                absolute_deadline_capability,
                AbsoluteDeadlineCapabilityV1::Unsupported
            )
        {
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
        // Worker governance BEFORE semantic policy (AGENTS rule 1: the two are
        // separate concerns). The substrate capability-pool slot is the physical
        // worker gate (topology-sized; unlimited pools return immediately). We
        // take it FIRST so a task waiting for a worker is NOT counted as a live
        // governor permit — inflight/cpu/memory/root attribution then reflect
        // tasks that are *actually executing*, and substrate backlog (its own
        // `SubstratePoolTimedOut`) never pollutes the semantic queue / retry-after
        // / resource-saturation signals.
        let gate = self
            .substrates
            .acquire(
                spec.primary_substrate_hint(),
                opts.cancel.as_ref(),
                acquire_deadline,
            )
            .await
            .map_err(|error| absolute_acquire_error_v1(absolute_deadline, error))?;
        // Now seek semantic admission. The permit's inflight count begins only
        // once a worker slot is already held.
        let permit = self
            .acquire(spec, opts, acquire_deadline)
            .await
            .map_err(|error| absolute_acquire_error_v1(absolute_deadline, error))?;
        Ok(ExecutionLease {
            _permit: PermitGuard {
                governor: Arc::clone(&self.governor),
                permit,
            },
            _gate: gate,
        })
    }

    /// The single admit→run→release path shared by every substrate whose work
    /// lifetime is identical to the caller future's lifetime.
    async fn with_permit<T, E, Fut, F>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        absolute_deadline_capability: AbsoluteDeadlineCapabilityV1,
        run: F,
    ) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, RunError<E>>>,
    {
        let _lease = self
            .acquire_execution_lease(&spec, &opts, absolute_deadline_capability)
            .await?;
        run().await
    }

    /// Reject a spec whose declared substrate is incompatible with this run path
    /// (substrate classification is authoritative, not advisory metadata).
    fn require_hint(spec: &TaskSpec, allowed: &[SubstrateHint]) -> Result<(), GovernorError> {
        if allowed.contains(&spec.primary_substrate_hint()) {
            Ok(())
        } else {
            Err(GovernorError::Rejected(AdmissionVerdict::SubstrateMismatch))
        }
    }

    /// Mid-run cancellation controls for `class` given the submission `opts`:
    /// a cancel token (for `Cooperative`/`CooperativeWithDeadline`) and a run
    /// deadline (for `CooperativeWithDeadline` only). `PreSubmitOnly` returns
    /// `(None, None)` — its token is honored pre-submit only.
    fn cancel_controls(
        &self,
        class: &TaskClass,
        opts: &SubmitOptions,
    ) -> (Option<CancellationToken>, Option<SubmissionDeadline>) {
        let policy = self
            .config
            .classes
            .get(class)
            .map(|p| p.cancellation_policy);
        let mid_run = matches!(
            policy,
            Some(CancellationPolicy::Cooperative | CancellationPolicy::CooperativeWithDeadline)
        );
        let with_deadline = matches!(policy, Some(CancellationPolicy::CooperativeWithDeadline));
        (
            mid_run.then(|| opts.cancel.clone()).flatten(),
            with_deadline.then_some(opts.deadline).flatten(),
        )
    }

    fn absolute_deadline(
        &self,
        class: &TaskClass,
        opts: &SubmitOptions,
    ) -> Option<std::time::Instant> {
        let with_deadline = self.config.classes.get(class).is_some_and(|policy| {
            policy.cancellation_policy == CancellationPolicy::CooperativeWithDeadline
        });
        with_deadline
            .then_some(opts.deadline)
            .flatten()
            .and_then(|deadline| match deadline {
                SubmissionDeadline::CompleteBy(instant) => Some(instant),
                SubmissionDeadline::RunFor(_) => None,
            })
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
        if let Err(e) = Self::require_hint(&spec, &[SubstrateHint::AsyncIo]) {
            return Err(RunError::Governor(e));
        }
        let (token, deadline) = self.cancel_controls(&spec.class, &opts);
        self.with_permit(
            spec,
            opts,
            AbsoluteDeadlineCapabilityV1::CooperativeAsync,
            move || async move {
                run_cancellable(token, deadline, async { fut.await.map_err(RunError::Task) }).await
            },
        )
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
        if let Err(e) = Self::require_hint(
            &spec,
            &[
                SubstrateHint::BlockingPool,
                SubstrateHint::LargeStackCapability,
                SubstrateHint::BackgroundOnly,
            ],
        ) {
            return Box::pin(ready(Err(RunError::Governor(e))));
        }
        if spec.requested_stack_size_bytes().is_some() {
            let boxed_job: BlockingJobV1<T, E> = Box::new(job);
            return Box::pin(async move {
                let lease = self
                    .acquire_execution_lease(
                        &spec,
                        &opts,
                        AbsoluteDeadlineCapabilityV1::Unsupported,
                    )
                    .await?;
                self.run_blocking_with_requested_stack_v1(&spec, boxed_job, lease)
                    .await
            });
        }
        Box::pin(async move {
            let lease = self
                .acquire_execution_lease(&spec, &opts, AbsoluteDeadlineCapabilityV1::Unsupported)
                .await?;
            match tokio::task::spawn_blocking(move || {
                let _lease = lease;
                job()
            })
            .await
            {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(err)) => Err(RunError::Task(err)),
                Err(_) => Err(RunError::Governor(GovernorError::PolicyViolation(
                    "spawn_blocking join failure".into(),
                ))),
            }
        })
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
        Self::require_hint(&spec, &[SubstrateHint::LargeStackCapability])
            .map_err(RunError::Governor)?;
        if spec.requested_stack_size_bytes().is_none() {
            return Err(RunError::Governor(GovernorError::PolicyViolation(
                "requested stack size missing on async large-stack path".into(),
            )));
        }
        let (cancel, deadline) = self.cancel_controls(&spec.class, &opts);
        let make_future: RequestedStackAsyncFutureFactoryV1<T, E> =
            Box::new(move || Box::pin(make_future()));
        let lease = self
            .acquire_execution_lease(&spec, &opts, AbsoluteDeadlineCapabilityV1::CooperativeAsync)
            .await?;
        self.run_async_with_requested_stack_v1(&spec, make_future, cancel, deadline, lease)
            .await
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
        enum CpuWorkOutcome<T, E> {
            Completed(Result<T, E>),
            Panicked,
        }

        if let Err(e) = Self::require_hint(&spec, &[SubstrateHint::SharedCpuExecutor]) {
            return Err(RunError::Governor(e));
        }
        let (token, deadline) = self.cancel_controls(&spec.class, &opts);
        let cpu = Arc::clone(&self.cpu);
        let lease = self
            .acquire_execution_lease(&spec, &opts, AbsoluteDeadlineCapabilityV1::Unsupported)
            .await?;
        let (tx, rx) = oneshot::channel::<(CpuWorkOutcome<T, E>, ExecutionLease)>();
        let work = Box::new(move || {
            // The host contract requires worker panic to surface as a
            // governor/runtime error instead of aborting the process.
            let outcome = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
                Ok(result) => CpuWorkOutcome::Completed(result),
                Err(_) => CpuWorkOutcome::Panicked,
            };
            // The receiver is gone only if the caller already cancelled; the
            // failed send then drops the lease on this worker. On success,
            // custody transfers with the result so `run_cpu` cannot report
            // completion before governed capacity is released.
            let _delivered = tx.send((outcome, lease));
        });
        if panic::catch_unwind(AssertUnwindSafe(|| cpu.spawn(work))).is_err() {
            return Err(RunError::Governor(GovernorError::PolicyViolation(
                "cpu executor spawn panicked".into(),
            )));
        }
        // The caller can stop awaiting a stalled executor, but the queued or
        // running closure retains its lease through job completion. A completed
        // result carries custody back so the caller drops it before returning.
        let work = async move {
            match rx.await {
                Ok((CpuWorkOutcome::Completed(Ok(value)), _lease)) => Ok(value),
                Ok((CpuWorkOutcome::Completed(Err(err)), _lease)) => Err(RunError::Task(err)),
                Ok((CpuWorkOutcome::Panicked, _lease)) => Err(RunError::Governor(
                    GovernorError::PolicyViolation("cpu worker panicked".into()),
                )),
                Err(_) => Err(RunError::Governor(GovernorError::PolicyViolation(
                    "cpu worker dropped result".into(),
                ))),
            }
        };
        run_cancellable(token, deadline, work).await
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
        if let Err(e) = Self::require_hint(&spec, &[SubstrateHint::LocalRuntime]) {
            return Err(RunError::Governor(e));
        }
        // run_local is async, so (like run_io) it honors mid-run cooperative
        // cancel/deadline for cancellable classes.
        let (token, deadline) = self.cancel_controls(&spec.class, &opts);
        self.with_permit(
            spec,
            opts,
            AbsoluteDeadlineCapabilityV1::CooperativeAsync,
            move || async move {
                let work = async move {
                    tokio::task::LocalSet::new()
                        .run_until(fut)
                        .await
                        .map_err(RunError::Task)
                };
                run_cancellable(token, deadline, work).await
            },
        )
        .await
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

fn requested_stack_size_bytes_v1<E>(spec: &TaskSpec) -> Result<usize, RunError<E>> {
    spec.requested_stack_size_bytes()
        .ok_or_else(|| {
            RunError::Governor(GovernorError::PolicyViolation(
                "requested stack size missing on large-stack path".into(),
            ))
        })?
        .try_into()
        .map_err(|_| {
            RunError::Governor(GovernorError::PolicyViolation(
                "requested stack size does not fit usize".into(),
            ))
        })
}

fn requested_stack_thread_name_v1(spec: &TaskSpec) -> String {
    format!("taskmesh.large_stack:{}", spec.class.as_str())
}

fn requested_stack_spawn_error_v1<E>(message: String) -> RunError<E> {
    RunError::Governor(GovernorError::PolicyViolation(
        format!("large-stack thread spawn failed: {message}").into(),
    ))
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
/// and the `run_cpu` result-await), so cancellation/deadline semantics are
/// identical everywhere. Sync blocking work has no awaitable cancel point and is
/// handled separately (pre-submit cancel only).
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

/// Releases a held permit on drop. Held across the work future so a cancelled
/// (dropped) submission still returns its permit and promotes queued work.
struct PermitGuard {
    governor: Arc<Governor>,
    permit: PermitId,
}

/// Owns the semantic permit and physical substrate slot for one execution.
/// Detached workers take this value so caller cancellation cannot report
/// capacity as free before the worker has actually terminated.
struct ExecutionLease {
    _permit: PermitGuard,
    _gate: Option<OwnedSemaphorePermit>,
}

impl Drop for PermitGuard {
    fn drop(&mut self) {
        self.governor.release(self.permit);
    }
}

/// Owns a queued ticket while the caller waits for promotion. On drop (caller
/// cancelled / timed out) it abandons the ticket: removing it from the engine
/// queue, or releasing it if it was already promoted. Disarmed once the permit
/// is successfully claimed (ownership then passes to a [`PermitGuard`]).
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

/// Per-substrate capability pools sized from [`TopologyConfig`]. Each pool caps
/// concurrent work on a substrate; a slot count of `0` means *unlimited* (no
/// gate), consistent with the `0 == no limit` convention used by resource
/// budgets. `AsyncIo` is intentionally ungated (async is unbounded by design).
///
/// `SharedCpuExecutor` is **always** gated to the resolved CPU worker count, so
/// CPU topology is authoritative on *both* host paths (Tokio blocking-pool and
/// Rayon): a `run_cpu` submission must hold a CPU slot before it reaches the
/// executor, so work cannot pile up unbounded behind held permits in an
/// executor's internal queue. This enforces the "no unbounded competing queue"
/// rule for the CPU substrate.
struct SubstrateGates {
    /// Worker gates keyed by capability-pool name, derived from the registered
    /// substrate inventory (which pools exist) × topology (how many slots each).
    pools: BTreeMap<String, Arc<Semaphore>>,
}

impl SubstrateGates {
    /// Build the worker gates from the inventory ∩ topology: a gate exists for a
    /// capability pool iff that substrate is **registered** (inventory is
    /// authoritative for which pools exist) and topology sizes it `> 0`. The
    /// `cpu` pool is always gated to the resolved CPU worker count. This makes the
    /// substrate inventory back the runtime, not just snapshot metadata.
    fn from_inventory(topology: &TopologyConfig, substrates: &[SubstrateRecord]) -> Self {
        let available = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let cpu_workers = topology.resolved_cpu_workers(available);
        let slots_for = |pool: &str| match pool {
            "cpu" => cpu_workers,
            "blocking" => topology.blocking_threads,
            "large_stack" => topology.large_stack_slots,
            "local_runtime" => topology.local_runtime_slots,
            "maintenance" => topology.maintenance_workers,
            _ => 0,
        };
        let mut pools = BTreeMap::new();
        for record in substrates {
            let Some(pool) = record.capability_pool.as_deref() else {
                continue;
            };
            let slots = slots_for(pool);
            if slots > 0 {
                pools.insert(pool.to_owned(), Arc::new(Semaphore::new(slots)));
            }
        }
        Self { pools }
    }

    fn gate_for(&self, hint: SubstrateHint) -> Option<&Arc<Semaphore>> {
        // Authoritative hint→pool mapping lives in the contract; a gate exists
        // only for a registered, topology-sized pool.
        self.pools.get(hint.capability_pool()?)
    }

    /// Acquire a slot for `hint`, held for the work's duration. `None` when the
    /// substrate is unlimited. Honors the bounded acquire wait in `opts`.
    async fn acquire(
        &self,
        hint: SubstrateHint,
        cancel: Option<&CancellationToken>,
        acquire_deadline: Option<Instant>,
    ) -> Result<Option<OwnedSemaphorePermit>, GovernorError> {
        let Some(sem) = self.gate_for(hint) else {
            return Ok(None);
        };
        let sem = Arc::clone(sem);
        tokio::select! {
            biased;
            () = cancellation_requested_v1(cancel), if cancel.is_some() => {
                Err(GovernorError::Rejected(
                    AdmissionVerdict::CancelledBeforeSubmit,
                ))
            }
            permit = sem.acquire_owned() => {
                Ok(Some(permit.expect("substrate semaphore open")))
            }
            () = acquisition_deadline_reached_v1(acquire_deadline), if acquire_deadline.is_some() => {
                // Distinct from the governor queue-wait timeout so the two
                // backpressure causes stay separable.
                Err(GovernorError::Rejected(
                    AdmissionVerdict::SubstratePoolTimedOut {
                        retry_after_ms: None,
                    },
                ))
            }
        }
    }
}
