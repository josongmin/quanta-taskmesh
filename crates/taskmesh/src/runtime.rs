//! `TokioRuntime` — the host adapter implementing the [`Runtime`] driving port.
//!
//! Every path funnels through `with_permit`: acquire a governor permit (waiting,
//! bounded, on a queue if needed), run the work on the right substrate, then
//! release — which promotes queued work. Governor rejection and task failure stay
//! distinct via [`RunError`]; a `spawn_blocking`/worker join failure is a
//! governor/runtime-side error, never a task error.

use std::future::Future;
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, CpuExecutor, GovernorError, RunError, Runtime, Snapshot, SubstrateHint,
    TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PermitId, RequestKey};
use tokio::sync::oneshot;
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
        let key = RequestKey::new(spec.root_operation_id.clone());
        match self.governor.admit_waitable(spec, key, waker.clone()) {
            AdmissionDecision::Admitted { permit_id } => Ok(permit_id),
            AdmissionDecision::Rejected(verdict) => Err(GovernorError::Rejected(verdict)),
            AdmissionDecision::Queued { ticket } => {
                // While parked on the queue this future owns the ticket. If it is
                // dropped (external cancellation) or times out, the guard abandons
                // the ticket — removing it from the engine queue, or releasing it
                // if it was already promoted — so a cancelled-while-queued request
                // can never linger or strand a promoted-but-unclaimed permit.
                let mut guard = TicketGuard {
                    governor: self.governor.clone(),
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
            governor: self.governor.clone(),
            permit,
        };
        run().await
        // `_guard` drops here (success) or on cancellation (future dropped),
        // releasing the permit exactly once.
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
        self.with_permit(
            spec,
            opts,
            || async move { fut.await.map_err(RunError::Task) },
        )
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
        let cpu = self.cpu.clone();
        self.with_permit(spec, opts, || async move {
            let (tx, rx) = oneshot::channel();
            cpu.spawn(Box::new(move || {
                let _ = tx.send(job());
            }));
            match rx.await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(err)) => Err(RunError::Task(err)),
                Err(_) => Err(RunError::Governor(GovernorError::PolicyViolation(
                    "cpu worker dropped result".into(),
                ))),
            }
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
        if spec.primary_substrate_hint() != SubstrateHint::LocalRuntime {
            return Err(RunError::Governor(GovernorError::LocalRuntimeUnavailable));
        }
        self.with_permit(spec, opts, || async move {
            tokio::task::LocalSet::new()
                .run_until(fut)
                .await
                .map_err(RunError::Task)
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
