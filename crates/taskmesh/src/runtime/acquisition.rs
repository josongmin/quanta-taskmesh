//! Permit acquisition, admission wait and host preflight.

use super::{
    plan_supports_absolute_deadline, require_tokio_context_v1, AcquisitionArbiter,
    AdmissionDecision, Arc, CapacityBlock, ClaimOutcome, DispatchKind, ExecutionLease,
    GovernorError, Instant, PermitId, RunError, SubmitOptions, SubstrateHint, TaskSpec, Ticket,
    TicketGuard, TokioPermitWaker, TokioRuntime, TopologyError, ValidatedDispatchPlan,
};

impl TokioRuntime {
    // ---- acquire/release core --------------------------------------------

    /// Acquire a governor permit that reserves `capability`.
    ///
    /// The capability is an explicit argument rather than something re-derived
    /// here: the pool a submission *reserves* must be the pool it will *occupy*,
    /// and only the resolved execution plan knows which that is.
    ///
    /// Honors pre-submit cancel and a bounded acquire wait. Queued requests park
    /// on a [`TokioPermitWaker`] and are woken on promotion — no polling.
    pub(super) async fn acquire(
        &self,
        plan: &ValidatedDispatchPlan,
        arbiter: &AcquisitionArbiter,
    ) -> Result<PermitId, GovernorError> {
        if let Some(rejection) = arbiter.rejection(std::time::Instant::now(), false) {
            return Err(AcquisitionArbiter::to_governor_error(rejection, false));
        }

        let waker = TokioPermitWaker::new();
        // `Arc::clone` cannot unsize a concrete `Arc<T>` to `Arc<dyn _>`; the
        // method form performs the clone-and-unsize coercion in one step.
        #[allow(
            clippy::clone_on_ref_ptr,
            reason = "Arc::clone cannot unsize concrete Arc<T> to Arc<dyn _>"
        )]
        let waker_port: Arc<dyn taskmesh_contract::PermitWaker> = waker.clone();
        match self.governor.admit_validated_requirements(
            plan.task(),
            plan.requirements.clone(),
            Some(waker_port),
        ) {
            Ok(AdmissionDecision::Admitted { permit_id }) => {
                self.finalize_acquired(permit_id, arbiter, false, None)
            }
            Ok(AdmissionDecision::Rejected(verdict)) => Err(GovernorError::Rejected(verdict)),
            Ok(AdmissionDecision::Queued { ticket }) => {
                // While parked on the queue this future owns the ticket. If it is
                // dropped (external cancellation) or times out, the guard abandons
                // the ticket — removing it from the engine queue, or releasing it
                // if it was already promoted — so a cancelled-while-queued request
                // can never linger or strand a promoted-but-unclaimed permit.
                let mut guard = TicketGuard {
                    governor: Arc::clone(&self.governor),
                    ticket: Some(ticket),
                };
                let result = self.await_promotion(ticket, &waker, arbiter).await;
                if matches!(
                    self.governor.ticket_status(ticket),
                    ClaimOutcome::Terminal(_) | ClaimOutcome::Invalid
                ) {
                    guard.disarm();
                }
                result
            }
            Err(error) => Err(GovernorError::PolicyViolation(
                format!("validated dispatch requirements rejected: {error}").into(),
            )),
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
    pub(super) async fn await_promotion(
        &self,
        ticket: Ticket,
        waker: &TokioPermitWaker,
        arbiter: &AcquisitionArbiter,
    ) -> Result<PermitId, GovernorError> {
        loop {
            // The diagnostic is sampled immediately before the ownership
            // decision. Intake-time causes are never retained or projected.
            let current_blocker = self.governor.pending_block_reason(ticket);
            if let Some(rejection) = arbiter.rejection(std::time::Instant::now(), true) {
                return match self.governor.claim(ticket) {
                    ClaimOutcome::Ready(permit_id) => {
                        self.finalize_acquired(permit_id, arbiter, true, current_blocker)
                    }
                    ClaimOutcome::Pending => Err(AcquisitionArbiter::to_governor_error(
                        rejection,
                        matches!(current_blocker, Some(CapacityBlock::Capability)),
                    )),
                    ClaimOutcome::Terminal(reason) => Err(GovernorError::TicketClaimTerminated {
                        ticket: ticket.sequence(),
                        reason: reason.into(),
                    }),
                    ClaimOutcome::Invalid => Err(GovernorError::InvalidTicketClaim {
                        ticket: ticket.sequence(),
                    }),
                };
            }
            match self.governor.claim(ticket) {
                ClaimOutcome::Ready(permit_id) => {
                    return self.finalize_acquired(permit_id, arbiter, true, current_blocker);
                }
                ClaimOutcome::Pending => {}
                ClaimOutcome::Terminal(reason) => {
                    return Err(GovernorError::TicketClaimTerminated {
                        ticket: ticket.sequence(),
                        reason: reason.into(),
                    });
                }
                ClaimOutcome::Invalid => {
                    return Err(GovernorError::InvalidTicketClaim {
                        ticket: ticket.sequence(),
                    });
                }
            }
            let cancel = arbiter.cancel_token();
            let wake_deadline = arbiter.wake_deadline().map(Instant::from_std);
            tokio::select! {
                biased;
                () = async {
                    match cancel {
                        Some(token) => token.cancelled().await,
                        None => std::future::pending::<()>().await,
                    }
                }, if cancel.is_some() => {}
                () = waker.notified() => {}
                () = async {
                    match wake_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => std::future::pending::<()>().await,
                    }
                }, if wake_deadline.is_some() => {}
            }
        }
    }

    /// The one permit-to-execution linearization point shared by immediate and
    /// promoted admission. A rejected handoff returns the unstarted permit
    /// exactly once and cannot create a worker, thread, or user closure effect.
    pub(super) fn finalize_acquired(
        &self,
        permit_id: PermitId,
        arbiter: &AcquisitionArbiter,
        queued: bool,
        current_blocker: Option<CapacityBlock>,
    ) -> Result<PermitId, GovernorError> {
        if let Some(rejection) = arbiter.rejection(std::time::Instant::now(), queued) {
            self.return_unstarted(permit_id);
            return Err(AcquisitionArbiter::to_governor_error(
                rejection,
                matches!(current_blocker, Some(CapacityBlock::Capability)),
            ));
        }
        Ok(permit_id)
    }

    /// Return a permit whose work will not start (budget expired after grant).
    ///
    /// The permit is still `DispatchReserved`, so this is exactly what dropping
    /// an undispatched [`ExecutionLease`] does — and it is done *by* one, so the
    /// pre-dispatch release rules live in a single place.
    pub(super) fn return_unstarted(&self, permit_id: PermitId) {
        drop(ExecutionLease::reserved(
            Arc::clone(&self.governor),
            permit_id,
        ));
    }

    /// Acquire the execution lease for a resolved plan.
    pub(super) async fn acquire_execution_lease<E>(
        &self,
        opts: &SubmitOptions,
        plan: &ValidatedDispatchPlan,
    ) -> Result<ExecutionLease, RunError<E>> {
        self.acquire_execution_lease_response_by(opts, plan, None)
            .await
    }

    pub(super) async fn acquire_execution_lease_response_by<E>(
        &self,
        opts: &SubmitOptions,
        plan: &ValidatedDispatchPlan,
        response_by: Option<std::time::Instant>,
    ) -> Result<ExecutionLease, RunError<E>> {
        let absolute_deadline = plan.absolute_deadline();
        if absolute_deadline.is_some() && !plan_supports_absolute_deadline(plan) {
            return Err(RunError::Governor(GovernorError::PolicyViolation(
                "absolute completion deadline requires cooperative async work".into(),
            )));
        }
        let arbiter = AcquisitionArbiter::new(opts, response_by.or(absolute_deadline))
            .map_err(RunError::Governor)?;
        // Contract verdicts that are already decided at poll time keep their
        // documented precedence over host prerequisites. In particular, a
        // pre-cancelled submission stays CancelledBeforeSubmit even when the
        // caller is outside Tokio.
        if let Some(rejection) = arbiter.rejection(std::time::Instant::now(), false) {
            return Err(RunError::Governor(AcquisitionArbiter::to_governor_error(
                rejection, false,
            )));
        }
        self.preflight_runtime_context(opts, plan, response_by)
            .map_err(RunError::Governor)?;
        let permit = self
            .acquire(plan, &arbiter)
            .await
            .map_err(RunError::Governor)?;
        Ok(ExecutionLease::reserved(Arc::clone(&self.governor), permit))
    }

    /// Validate caller-runtime services before a permit or ticket can exist.
    /// Plain I/O futures and custom executors remain usable on other executors;
    /// only paths that actually need Tokio workers, timers, or a LocalSet are
    /// rejected.
    pub(super) fn preflight_runtime_context(
        &self,
        opts: &SubmitOptions,
        plan: &ValidatedDispatchPlan,
        response_by: Option<std::time::Instant>,
    ) -> Result<(), GovernorError> {
        if plan.spec().primary_substrate_hint() == SubstrateHint::LocalRuntime
            && tokio::runtime::Handle::try_current().is_err()
        {
            return Err(GovernorError::LocalRuntimeUnavailable);
        }

        let timer_required = response_by.is_some()
            || opts.acquire_timeout.is_some_and(|budget| !budget.is_zero())
            || match plan.dispatch {
                DispatchKind::CallerFuture => plan.deadline.is_some(),
                DispatchKind::BlockingPool
                | DispatchKind::CpuExecutor
                | DispatchKind::DedicatedStackThread => plan.run_budget().is_some(),
                DispatchKind::DedicatedStackRuntime => plan.absolute_deadline().is_some(),
            };
        let context = match plan.dispatch {
            DispatchKind::BlockingPool => Some("blocking worker"),
            DispatchKind::CpuExecutor if self.cpu_capabilities.requires_tokio_context => {
                Some("cpu executor")
            }
            _ if timer_required => Some("submission timer"),
            _ => None,
        };
        match context {
            Some(context) => require_tokio_context_v1(context),
            None => Ok(()),
        }
    }

    /// Resolve the plan for one submission against its class policy.
    pub(super) fn plan(
        &self,
        spec: TaskSpec,
        opts: &SubmitOptions,
        allowed: &[SubstrateHint],
        dispatch: DispatchKind,
    ) -> Result<ValidatedDispatchPlan, GovernorError> {
        let policy = self.config.classes.get(&spec.class);
        let cpu_physical_domain =
            self.cpu_capabilities
                .physical_domain
                .ok_or(GovernorError::InvalidTopology(
                    TopologyError::ExecutorPhysicalDomainUnknown,
                ))?;
        ValidatedDispatchPlan::preflight(
            spec,
            opts,
            policy,
            allowed,
            dispatch,
            self.governor.policy(),
            cpu_physical_domain,
        )
    }
}
