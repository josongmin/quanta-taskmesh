//! Synchronous blocking and CPU dispatch under response/custody bounds.

use super::{
    await_detached, oneshot, panic, ready, requested_stack_thread_name_v1, run_detached_job,
    worker_unavailable_v1, Arc, AssertUnwindSafe, BlockingJobV1, DispatchKind, ExecutionLease,
    ExecutionPhase, GovernorError, RunError, RuntimeBoxFutureV1, SubmitOptions, SubstrateHint,
    TaskSpec, TokioRuntime, ValidatedDispatchPlan,
};

impl TokioRuntime {
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
        self.run_blocking_with_response_boundary(spec, opts, None, job)
    }

    /// Bound acquisition and the caller's response by one absolute instant.
    /// A started synchronous worker cannot be stopped; it retains capacity
    /// until it terminates, even after this call returns `DeadlineExceeded`.
    /// `SubmitOptions::deadline` must be absent: `CompleteBy` remains reserved
    /// for cooperative async work, and `RunFor` is a separate run-start budget.
    pub fn run_blocking_response_by<T, E, F>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        response_by: std::time::Instant,
        job: F,
    ) -> RuntimeBoxFutureV1<'_, T, E>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        self.run_blocking_with_response_boundary(spec, opts, Some(response_by), job)
    }

    fn run_blocking_with_response_boundary<T, E, F>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        response_by: Option<std::time::Instant>,
        job: F,
    ) -> RuntimeBoxFutureV1<'_, T, E>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        let plan = match self.plan(
            spec,
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
        if response_by.is_some() && opts.deadline.is_some() {
            return Box::pin(ready(Err(RunError::Governor(
                GovernorError::PolicyViolation(
                    "response boundary cannot be combined with a run deadline".into(),
                ),
            ))));
        }
        let boxed_job: BlockingJobV1<T, E> = Box::new(job);
        Box::pin(async move {
            let lease = self
                .acquire_execution_lease_response_by(&opts, &plan, response_by)
                .await?;
            if response_by.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                return Err(RunError::Governor(GovernorError::DeadlineExceeded));
            }
            match plan.dispatch {
                DispatchKind::DedicatedStackThread => {
                    self.run_blocking_on_dedicated_thread(boxed_job, lease, &plan, response_by)
                        .await
                }
                _ => {
                    self.run_blocking_on_pool(boxed_job, lease, &plan, response_by)
                        .await
                }
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
        plan: &ValidatedDispatchPlan,
        response_by: Option<std::time::Instant>,
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
            run_detached_job(
                job,
                lease,
                started_tx,
                done_tx,
                "blocking worker",
                response_by,
            );
        });
        await_detached(
            started_rx,
            done_rx,
            plan.cancel.clone(),
            plan.run_budget(),
            "blocking worker",
            response_by,
        )
        .await
    }

    /// Blocking work on a dedicated OS thread sized by the spec's stack request.
    async fn run_blocking_on_dedicated_thread<T, E>(
        &self,
        job: BlockingJobV1<T, E>,
        lease: ExecutionLease,
        plan: &ValidatedDispatchPlan,
        response_by: Option<std::time::Instant>,
    ) -> Result<T, RunError<E>>
    where
        E: Send + 'static,
        T: Send + 'static,
    {
        let stack_size_bytes = plan
            .stack_size()
            .expect("dedicated stack dispatch is preflight-validated")
            .get();
        let thread_name = requested_stack_thread_name_v1(plan.spec());
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
                run_detached_job(
                    job,
                    lease,
                    started_tx,
                    done_tx,
                    "large-stack worker",
                    response_by,
                );
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
            plan.run_budget(),
            "large-stack worker",
            response_by,
        )
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
        self.run_cpu_with_response_boundary(spec, opts, None, job)
            .await
    }

    /// Bound CPU acquisition and the caller's response by one absolute
    /// instant. A started worker retains its lease until it actually finishes.
    pub async fn run_cpu_response_by<T, E, F>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        response_by: std::time::Instant,
        job: F,
    ) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        self.run_cpu_with_response_boundary(spec, opts, Some(response_by), job)
            .await
    }

    async fn run_cpu_with_response_boundary<T, E, F>(
        &self,
        spec: TaskSpec,
        opts: SubmitOptions,
        response_by: Option<std::time::Instant>,
        job: F,
    ) -> Result<T, RunError<E>>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        E: Send + 'static,
        T: Send + 'static,
    {
        let plan = match self.plan(
            spec,
            &opts,
            &[SubstrateHint::SharedCpuExecutor],
            DispatchKind::CpuExecutor,
        ) {
            Ok(plan) => plan,
            Err(error) => return Err(RunError::Governor(error)),
        };
        if response_by.is_some() && opts.deadline.is_some() {
            return Err(RunError::Governor(GovernorError::PolicyViolation(
                "response boundary cannot be combined with a run deadline".into(),
            )));
        }
        let cpu = Arc::clone(&self.cpu);
        let lease = self
            .acquire_execution_lease_response_by(&opts, &plan, response_by)
            .await?;
        if response_by.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
            return Err(RunError::Governor(GovernorError::DeadlineExceeded));
        }
        let (started_tx, started_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let boxed_job: BlockingJobV1<T, E> = Box::new(job);
        let mut lease = lease;
        lease.advance(ExecutionPhase::Accepted)?;
        let work = Box::new(move || {
            run_detached_job(
                boxed_job,
                lease,
                started_tx,
                done_tx,
                "cpu worker",
                response_by,
            );
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
            plan.run_budget(),
            "cpu worker",
            response_by,
        )
        .await
    }
}
