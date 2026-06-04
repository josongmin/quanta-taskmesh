//! `TokioRuntime` — the host adapter implementing the [`Runtime`] driving port.
//!
//! Every path funnels through `with_permit`: acquire a governor permit (waiting,
//! bounded, on a queue if needed), run the work on the right substrate, then
//! release — which promotes queued work. Governor rejection and task failure stay
//! distinct via [`RunError`]; a `spawn_blocking`/worker join failure is a
//! governor/runtime-side error, never a task error.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use taskmesh_contract::{
    AdmissionVerdict, CancellationPolicy, CpuExecutor, GovernorError, RunError, Runtime, Snapshot,
    SubstrateHint, TaskClass, TaskSpec, TopologyConfig,
};
use taskmesh_engine::{AdmissionDecision, Governor, PermitId, RequestKey};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use tokio::time::{timeout_at, Instant};

use crate::adapters::TokioPermitWaker;
use crate::executor::SubmitOptions;
use crate::RuntimeConfig;

/// A governed, Tokio-hosted runtime. Cheap to clone (shared `Arc` internals).
#[derive(Clone)]
pub struct TokioRuntime {
    config: RuntimeConfig,
    governor: Arc<Governor>,
    cpu: Arc<dyn CpuExecutor>,
    substrates: Arc<SubstrateGates>,
}

impl TokioRuntime {
    pub(crate) fn new(
        config: RuntimeConfig,
        governor: Arc<Governor>,
        cpu: Arc<dyn CpuExecutor>,
    ) -> Self {
        let substrates = Arc::new(SubstrateGates::from_topology(&config.topology));
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
        let key = RequestKey::new(spec.root_operation_id.clone());
        match self.governor.admit_waitable(spec, key, waker_port) {
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
                let result = self.await_promotion(ticket, &waker, opts).await;
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
        opts: &SubmitOptions,
    ) -> Result<PermitId, GovernorError> {
        let until = opts.acquire_timeout.map(|d| Instant::now() + d);
        loop {
            if let Some(permit_id) = self.governor.claim(ticket) {
                return Ok(permit_id);
            }
            match until {
                None => waker.notified().await,
                Some(deadline) => {
                    if timeout_at(deadline, waker.notified()).await.is_err() {
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
    }

    /// The single admit→run→release path shared by every substrate.
    ///
    /// The acquired permit is held by a [`PermitGuard`] whose `Drop` releases it.
    /// This makes release cancellation-safe: if the returned future is dropped
    /// while the work is in flight (timeout wrapper, `select!`, task abort), the
    /// permit is still returned and queued work is still promoted — no leak.
    async fn with_permit<T, E, Fut, F>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        run: F,
    ) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, RunError<E>>>,
    {
        let permit = self
            .acquire(&spec, &opts)
            .await
            .map_err(RunError::Governor)?;
        let _guard = PermitGuard {
            governor: Arc::clone(&self.governor),
            permit,
        };
        // Acquire a slot in the substrate's capability pool (topology-sized). Held
        // for the duration of the work, then dropped. Unlimited pools (slots == 0)
        // return immediately. On a bounded-wait timeout the `_guard` above still
        // releases the governor permit.
        let _gate = self
            .substrates
            .acquire(spec.primary_substrate_hint(), &opts)
            .await
            .map_err(RunError::Governor)?;
        run().await
        // `_guard`/`_gate` drop here (success) or on cancellation (future dropped),
        // releasing the governor permit and substrate slot exactly once.
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
    ) -> (Option<CancellationToken>, Option<Duration>) {
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
        self.with_permit(spec, opts, move || async move {
            run_cancellable(token, deadline, async { fut.await.map_err(RunError::Task) }).await
        })
        .await
    }

    pub async fn run_blocking_with<T, E, F>(
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
        if let Err(e) = Self::require_hint(
            &spec,
            &[
                SubstrateHint::BlockingPool,
                SubstrateHint::LargeStackCapability,
                SubstrateHint::BackgroundOnly,
            ],
        ) {
            return Err(RunError::Governor(e));
        }
        self.with_permit(spec, opts, || async move {
            match tokio::task::spawn_blocking(job).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(err)) => Err(RunError::Task(err)),
                Err(_) => Err(RunError::Governor(GovernorError::PolicyViolation(
                    "spawn_blocking join failure".into(),
                ))),
            }
        })
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
        if let Err(e) = Self::require_hint(&spec, &[SubstrateHint::SharedCpuExecutor]) {
            return Err(RunError::Governor(e));
        }
        let (token, deadline) = self.cancel_controls(&spec.class, &opts);
        let cpu = Arc::clone(&self.cpu);
        self.with_permit(spec, opts, move || async move {
            let (tx, rx) = oneshot::channel();
            cpu.spawn(Box::new(move || {
                // The receiver is gone only if the caller already cancelled; the
                // result is then intentionally not delivered.
                let _delivered = tx.send(job());
            }));
            // Wrap the result-await in cooperative cancel/deadline so a cancellable
            // class can escape even a stalled custom CpuExecutor (no liveness
            // guarantee from the port) instead of hanging on `rx` forever.
            let work = async move {
                match rx.await {
                    Ok(Ok(value)) => Ok(value),
                    Ok(Err(err)) => Err(RunError::Task(err)),
                    Err(_) => Err(RunError::Governor(GovernorError::PolicyViolation(
                        "cpu worker dropped result".into(),
                    ))),
                }
            };
            run_cancellable(token, deadline, work).await
        })
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
        if let Err(e) = Self::require_hint(&spec, &[SubstrateHint::LocalRuntime]) {
            return Err(RunError::Governor(e));
        }
        // run_local is async, so (like run_io) it honors mid-run cooperative
        // cancel/deadline for cancellable classes.
        let (token, deadline) = self.cancel_controls(&spec.class, &opts);
        self.with_permit(spec, opts, move || async move {
            let work = async move {
                tokio::task::LocalSet::new()
                    .run_until(fut)
                    .await
                    .map_err(RunError::Task)
            };
            run_cancellable(token, deadline, work).await
        })
        .await
    }
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
    deadline: Option<Duration>,
    work: impl Future<Output = Result<T, RunError<E>>>,
) -> Result<T, RunError<E>> {
    if token.is_none() && deadline.is_none() {
        return work.await;
    }
    let cancelled = async {
        match &token {
            Some(t) => t.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    let timed = async {
        match deadline {
            Some(d) => tokio::time::sleep(d).await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(work);
    tokio::select! {
        biased;
        () = cancelled, if token.is_some() => Err(RunError::Governor(GovernorError::Cancelled)),
        () = timed, if deadline.is_some() => Err(RunError::Governor(GovernorError::DeadlineExceeded)),
        out = &mut work => out,
    }
}

/// Releases a held permit on drop. Held across the work future so a cancelled
/// (dropped) submission still returns its permit and promotes queued work.
struct PermitGuard {
    governor: Arc<Governor>,
    permit: PermitId,
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
    cpu: Arc<Semaphore>,
    blocking: Option<Arc<Semaphore>>,
    large_stack: Option<Arc<Semaphore>>,
    local_runtime: Option<Arc<Semaphore>>,
    maintenance: Option<Arc<Semaphore>>,
}

impl SubstrateGates {
    fn from_topology(topology: &TopologyConfig) -> Self {
        let gate = |n: usize| (n > 0).then(|| Arc::new(Semaphore::new(n)));
        let available = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let cpu_workers = topology.resolved_cpu_workers(available);
        Self {
            cpu: Arc::new(Semaphore::new(cpu_workers)),
            blocking: gate(topology.blocking_threads),
            large_stack: gate(topology.large_stack_slots),
            local_runtime: gate(topology.local_runtime_slots),
            maintenance: gate(topology.maintenance_workers),
        }
    }

    fn gate_for(&self, hint: SubstrateHint) -> Option<&Arc<Semaphore>> {
        match hint {
            SubstrateHint::SharedCpuExecutor => Some(&self.cpu),
            SubstrateHint::BlockingPool => self.blocking.as_ref(),
            SubstrateHint::LargeStackCapability => self.large_stack.as_ref(),
            SubstrateHint::LocalRuntime => self.local_runtime.as_ref(),
            SubstrateHint::BackgroundOnly => self.maintenance.as_ref(),
            // AsyncIo is ungated (async is unbounded by design).
            // `#[non_exhaustive]`: future substrates are ungated until wired.
            _ => None,
        }
    }

    /// Acquire a slot for `hint`, held for the work's duration. `None` when the
    /// substrate is unlimited. Honors the bounded acquire wait in `opts`.
    async fn acquire(
        &self,
        hint: SubstrateHint,
        opts: &SubmitOptions,
    ) -> Result<Option<OwnedSemaphorePermit>, GovernorError> {
        let Some(sem) = self.gate_for(hint) else {
            return Ok(None);
        };
        let sem = Arc::clone(sem);
        match opts.acquire_timeout {
            None => Ok(Some(
                sem.acquire_owned().await.expect("substrate semaphore open"),
            )),
            Some(deadline) => {
                match timeout_at(Instant::now() + deadline, sem.acquire_owned()).await {
                    Ok(permit) => Ok(Some(permit.expect("substrate semaphore open"))),
                    // Distinct from the governor queue-wait timeout so the two
                    // backpressure causes stay separable.
                    Err(_) => Err(GovernorError::Rejected(
                        AdmissionVerdict::SubstratePoolTimedOut {
                            retry_after_ms: None,
                        },
                    )),
                }
            }
        }
    }
}
