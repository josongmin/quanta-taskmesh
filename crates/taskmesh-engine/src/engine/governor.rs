//! The governor: the application service at the hexagon's core. It owns the
//! driven [`Clock`] port and the mutex-guarded [`GovernedState`], and delegates
//! every rule to a feature slice. No Tokio, no I/O, no product taxonomy.
//!
//! # Lock discipline
//!
//! The state mutex guards **pure state transitions only**. Two rules keep it
//! that way, and both are load-bearing:
//!
//! 1. *Driven ports are called outside the lock.* The [`Clock`] is sampled
//!    before acquiring it and clamped to a monotonic watermark inside; every
//!    [`PermitWaker`] is notified after releasing it.
//! 2. *Host `Arc`s are destroyed outside the lock.* A waker's destructor is host
//!    code just as much as its `wake()`, and a destructor that re-enters the
//!    governor under the mutex deadlocks the whole runtime. Removed requests
//!    hand their port references to [`TransitionEffects`], which is drained
//!    after the guard is dropped.
//!
//! # Bounded promotion
//!
//! One promotion pass grants at most [`PROMOTION_BUDGET`] permits. If runnable
//! work remains, the pass reports it and the caller re-enters after releasing
//! the lock. Progress is therefore guaranteed *without* an unbounded critical
//! section and without waiting for an external event to nudge the queue.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    CapabilityUsage, CheckpointPolicy, ClassSnapshot, Clock, ExecutionPhase, FairnessPolicy,
    GovernorError, MemoryOvercommitPolicy, MemoryPermitMode, OverflowPolicy, PermitWaker,
    SettlementWaker, Snapshot, SubstrateKind, SubstrateRecord, TaskClass, TaskSpec,
    ValidatedTaskPlan, SNAPSHOT_SCHEMA_VERSION,
};

use crate::engine::state::{
    AbandonOutcome, AdvanceOutcome, CapacityBlock, ClaimOutcome, GovernedState, GrantRequest,
    LeaseToken, ReleaseOutcome, TerminalReason, TicketState, TransitionEffects,
};
use crate::features::memory::{MeasurementSequence, ReconcileOutcome};
use crate::features::{admission, composite, fairness, inventory, memory};
use crate::shared::{
    AdmissionDecision, CapabilityRequirementSet, CapabilityResolutionError, LeakSweepReport,
    PermitId, PolicySet, Provenance, ResolvedCapability, RootAttribution, StageReleaseOutcome,
    Ticket,
};
use crate::sync::{AtomicU64, Mutex, Ordering};

static NEXT_GOVERNOR_AUTHORITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Permits granted in one promotion pass before the lock is released.
///
/// The bound exists so a large freed budget cannot turn one release into an
/// arbitrarily long critical section. It does not cost progress: a pass that
/// stops early says so, and the caller resumes immediately.
pub const PROMOTION_BUDGET: usize = 64;

/// Typed result keeps the promotion loop total and makes "nothing removed"
/// distinct from a truthy continuation convention.
enum CycleTerminalization {
    Removed,
    None,
}

/// The governed execution control point. Cheap to wrap in `Arc` and share.
///
/// `Debug` prints the policy's class set and the id counters — never the
/// guarded state, which would take the mutex inside a formatter (a host
/// `Debug` in a log line must not contend with admission).
pub struct Governor {
    authority: u64,
    policy: PolicySet,
    clock: Arc<dyn Clock>,
    next_permit: AtomicU64,
    next_ticket: AtomicU64,
    next_seq: AtomicU64,
    state: Mutex<GovernedState>,
    settlement_waker: Option<Arc<dyn SettlementWaker>>,
}

mod memory_governance;
mod observation;
mod validation;

impl std::fmt::Debug for Governor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Governor")
            .field("classes", &self.policy.classes.keys().collect::<Vec<_>>())
            .field("next_permit", &self.next_permit.load(Ordering::Relaxed))
            .field("next_ticket", &self.next_ticket.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Governor {
    /// Construct a governor, **validating the policy fail-closed** first. Any
    /// impossible budget, invalid memory scaling, mixed-tier fairness, misused
    /// degrade/queue policy, or incomplete substrate registry is rejected here —
    /// a direct engine embedder (via `taskmesh::ext`) gets the same
    /// construction-time rejection the host `Builder` enforces, so contradictory
    /// policy can never boot.
    pub fn new(policy: PolicySet, clock: Arc<dyn Clock>) -> Result<Self, GovernorError> {
        Self::validate_policy(&policy)?;
        Self::construct(policy, clock, None)
    }

    /// Construct a validated governor with a host settlement observer. The
    /// observer runs after the state lock is released whenever a transition may
    /// have reduced queued or inflight custody. A wake is only a hint: the host
    /// must re-read [`Self::snapshot`].
    pub fn new_with_settlement_waker(
        policy: PolicySet,
        clock: Arc<dyn Clock>,
        settlement_waker: Arc<dyn SettlementWaker>,
    ) -> Result<Self, GovernorError> {
        Self::validate_policy(&policy)?;
        Self::construct(policy, clock, Some(settlement_waker))
    }

    /// Raw constructor (no validation). Private: the validated [`Governor::new`]
    /// and the `test-util`-gated [`Governor::new_unchecked`] both route through it.
    fn construct(
        policy: PolicySet,
        clock: Arc<dyn Clock>,
        settlement_waker: Option<Arc<dyn SettlementWaker>>,
    ) -> Result<Self, GovernorError> {
        let authority = Self::take_sequence(&NEXT_GOVERNOR_AUTHORITY)
            .ok_or(GovernorError::IdentityAuthorityExhausted)?;
        Ok(Self {
            authority,
            policy,
            clock,
            next_permit: AtomicU64::new(1),
            next_ticket: AtomicU64::new(1),
            next_seq: AtomicU64::new(1),
            state: Mutex::new(GovernedState::default()),
            settlement_waker,
        })
    }

    /// Construct a governor **without** validating the policy — the caller
    /// asserts the policy was already validated, or is intentionally exercising
    /// raw scheduler behavior in tests.
    ///
    /// Gated behind the off-by-default `test-util` feature so it is absent from
    /// the public surface a downstream embedder sees: there is no peer-level
    /// public escape hatch around the fail-closed [`Governor::new`]. Prefer
    /// [`Governor::new`] everywhere else.
    #[cfg(feature = "test-util")]
    #[doc(hidden)]
    pub fn new_unchecked(policy: PolicySet, clock: Arc<dyn Clock>) -> Self {
        Self::construct(policy, clock, None)
            .expect("test process exhausted the governor authority identity space")
    }

    pub fn policy(&self) -> &PolicySet {
        &self.policy
    }

    /// Sample the driven clock. Called **before** the state lock is taken; the
    /// value is clamped to the monotonic commit watermark inside the transition.
    fn sample_clock_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    fn take_sequence(counter: &AtomicU64) -> Option<u64> {
        let mut current = counter.load(Ordering::Relaxed);
        loop {
            let next = current.checked_add(1)?;
            match counter.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(previous) => return Some(previous),
                Err(observed) => current = observed,
            }
        }
    }

    fn fresh_ids(&self) -> Option<admission::Ids> {
        let permit = Self::take_sequence(&self.next_permit)?;
        let ticket = Self::take_sequence(&self.next_ticket)?;
        let seq_no = Self::take_sequence(&self.next_seq)?;
        Some(admission::Ids {
            permit_id: PermitId::new(self.authority, permit),
            ticket: Ticket::new(self.authority, ticket),
            seq_no,
        })
    }

    /// Set the next local identity sequences for exhaustion-path tests.
    #[cfg(feature = "test-util")]
    #[doc(hidden)]
    pub fn set_identity_counters_for_test(&self, permit: u64, ticket: u64, seq: u64) {
        self.next_permit.store(permit, Ordering::Relaxed);
        self.next_ticket.store(ticket, Ordering::Relaxed);
        self.next_seq.store(seq, Ordering::Relaxed);
    }

    /// The capability pool a spec resolves to by its declared substrate hint.
    /// Hosts that resolve an execution plan (e.g. a stack request that must run
    /// on a dedicated worker) pass the resolved capability explicitly instead.
    fn declared_requirements(&self, spec: &TaskSpec) -> CapabilityRequirementSet {
        CapabilityRequirementSet::from_resolved(spec.stages.iter().map(|stage| {
            stage
                .substrate_hint
                .capability_pool()
                .map_or(ResolvedCapability::Ungated, |pool| {
                    self.policy
                        .resolve_capability(pool)
                        .expect("built-in substrate capabilities are validated at construction")
                })
        }))
        .expect("unique built-in hint pools are below the capability requirement bound")
    }

    // ---- admission --------------------------------------------------------

    /// Admit a request without a promotion waker (caller does not wait on a
    /// queue). The admission key is derived authoritatively from
    /// `spec.root_operation_id`; there is no caller-supplied key.
    pub fn admit(&self, spec: &TaskSpec) -> AdmissionDecision {
        if spec.validate_borrowed().is_err() {
            return AdmissionDecision::Rejected(taskmesh_contract::AdmissionVerdict::MalformedTask);
        }
        let requirements = self.declared_requirements(spec);
        self.admit_inner(spec, requirements, None)
    }

    pub fn admit_validated(&self, plan: &ValidatedTaskPlan) -> AdmissionDecision {
        let requirements = self.declared_requirements(plan.as_spec());
        self.admit_inner(plan.as_spec(), requirements, None)
    }

    /// Admit a request, registering a waker that fires if the request is queued
    /// and later promoted.
    pub fn admit_waitable(
        &self,
        spec: &TaskSpec,
        waker: Arc<dyn PermitWaker>,
    ) -> AdmissionDecision {
        if spec.validate_borrowed().is_err() {
            drop(waker);
            return AdmissionDecision::Rejected(taskmesh_contract::AdmissionVerdict::MalformedTask);
        }
        let requirements = self.declared_requirements(spec);
        self.admit_inner(spec, requirements, Some(waker))
    }

    /// Admit a request against an explicitly resolved capability pool.
    ///
    /// The host resolves which pool the work will *actually* occupy — which is
    /// not always the one the declared hint names — and admission reserves that
    /// pool. Passing the real capability is what stops a stack request from
    /// being gated by the blocking pool while it runs on a dedicated worker.
    pub fn admit_resolved(
        &self,
        spec: &TaskSpec,
        capability: ResolvedCapability,
        waker: Option<Arc<dyn PermitWaker>>,
    ) -> Result<AdmissionDecision, CapabilityResolutionError> {
        if let Err(error) = self.policy.validate_resolved_capability(&capability) {
            drop(waker);
            return Err(error);
        }
        if spec.validate_borrowed().is_err() {
            drop(waker);
            return Ok(AdmissionDecision::Rejected(
                taskmesh_contract::AdmissionVerdict::MalformedTask,
            ));
        }
        let requirements = CapabilityRequirementSet::from_resolved([capability])?;
        Ok(self.admit_inner(spec, requirements, waker))
    }

    pub fn admit_validated_requirements(
        &self,
        plan: &ValidatedTaskPlan,
        requirements: CapabilityRequirementSet,
        waker: Option<Arc<dyn PermitWaker>>,
    ) -> Result<AdmissionDecision, CapabilityResolutionError> {
        if let Err(error) = self.policy.validate_requirements(&requirements) {
            drop(waker);
            return Err(error);
        }
        Ok(self.admit_inner(plan.as_spec(), requirements, waker))
    }

    fn admit_inner(
        &self,
        spec: &TaskSpec,
        capabilities: CapabilityRequirementSet,
        waker: Option<Arc<dyn PermitWaker>>,
    ) -> AdmissionDecision {
        let sampled = self.sample_clock_ms();
        let Some(ids) = self.fresh_ids() else {
            drop(waker);
            return AdmissionDecision::Rejected(
                taskmesh_contract::AdmissionVerdict::IdentityExhausted,
            );
        };
        let request = admission::AdmissionRequest { spec, capabilities };
        let mut effects = TransitionEffects::default();
        let decision = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            admission::admit(
                &mut state,
                &self.policy,
                now,
                &request,
                waker,
                ids,
                &mut effects,
            )
        };
        if let Some(payload) = self.apply_effects(effects) {
            // The final notifier may have panicked in Drop after an immediate
            // grant. The caller has not received the permit ID, so returning
            // this panic without compensation would orphan every charge.
            if let AdmissionDecision::Admitted { permit_id } = decision {
                let mut compensation = TransitionEffects::default();
                {
                    let mut state = self.state.lock();
                    let now = state.commit_time(sampled);
                    // A reentrant host callback may already have advanced the
                    // permit and taken its lease. Only undispatched custody can
                    // be compensated here; never refund a running worker.
                    if state.release_with_proof(permit_id, None, &mut compensation)
                        == ReleaseOutcome::Released
                    {
                        compensation.absorb(self.promote(&mut state, now));
                    }
                }
                // Finish all follow-up notifications; retain the original host
                // panic even if a later callback also fails.
                let secondary = self.drain_effects_captured(compensation, sampled);
                drop(secondary);
            }
            std::panic::resume_unwind(payload);
        }
        decision
    }

    /// Claim a permit for a previously-queued ticket.
    ///
    /// The four outcomes are genuinely different states and the caller must be
    /// able to tell them apart. A bare `Option` cannot: "not promoted yet" and
    /// "the permit this ticket held was reclaimed" both read as `None`, and a
    /// waiter that treats the second as the first waits forever for a promotion
    /// that already happened and was undone.
    ///
    /// A successful claim is lease activity: the waiter that was parked while
    /// the permit sat promoted-but-unclaimed has just proven itself alive, so
    /// the leak sweep's staleness clock restarts here rather than at promotion.
    pub fn claim(&self, ticket: Ticket) -> ClaimOutcome {
        let sampled = self.sample_clock_ms();
        let (permit_id, effects) = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            match state.tickets.get(&ticket).cloned() {
                Some(TicketState::Granted(permit_id)) => {
                    let Some(record) = state.permits.get_mut(&permit_id) else {
                        return ClaimOutcome::Terminal(TerminalReason::Released);
                    };
                    // The claimant is alive at this transition. Refresh the
                    // lease before retiring the final host waker outside the
                    // lock: its Drop may re-enter a leak sweep before claim
                    // returns to take the second lock.
                    record.ledger.last_touched_ms = record.ledger.last_touched_ms.max(now);
                    let mut effects = TransitionEffects::default();
                    if let Some(waker) = record.claim_waker.take() {
                        effects.retire.push(waker);
                    }
                    state
                        .tickets
                        .insert(ticket, TicketState::Claiming(permit_id));
                    (permit_id, effects)
                }
                Some(TicketState::Queued { .. } | TicketState::Claiming(_)) => {
                    return ClaimOutcome::Pending;
                }
                Some(TicketState::Terminal(reason)) => {
                    state.forget_terminal_ticket(ticket);
                    return ClaimOutcome::Terminal(reason);
                }
                Some(TicketState::Faulted) => {
                    state.forget_terminal_ticket(ticket);
                    return ClaimOutcome::Terminal(TerminalReason::AccountingFault);
                }
                None => return ClaimOutcome::Invalid,
            }
        };

        if let Some(payload) = self.apply_effects(effects) {
            // The caller never received custody. Refund only a permit still
            // reserved for dispatch: a reentrant host callback may already
            // have leased it to a worker before panicking.
            let mut compensation = TransitionEffects::default();
            {
                let mut state = self.state.lock();
                let now = state.commit_time(sampled);
                // Ticket identifiers are monotonic and never rebound. If this
                // claim is still in the transient state, it necessarily owns
                // `permit_id`; abandon/reap remove or terminalize it instead.
                if matches!(state.tickets.get(&ticket), Some(TicketState::Claiming(_))) {
                    match state.release_with_proof_reason(
                        permit_id,
                        None,
                        TerminalReason::ClaimDeliveryFailed,
                        &mut compensation,
                    ) {
                        ReleaseOutcome::Released => {
                            compensation.absorb(self.promote(&mut state, now));
                        }
                        ReleaseOutcome::HeldByLease { .. } => {
                            // The worker has physical custody. Retire only the
                            // failed caller ticket; keep every capacity charge
                            // until that worker releases its lease token.
                            if let Some(record) = state.permits.get_mut(&permit_id) {
                                record.pending_ticket = None;
                            }
                            state.record_terminal_ticket(
                                ticket,
                                TerminalReason::ClaimDeliveryFailed,
                            );
                        }
                        ReleaseOutcome::UnknownPermit => {
                            // A reentrant callback may have completed the
                            // worker and removed the permit already.
                            state.record_terminal_ticket(ticket, TerminalReason::Released);
                        }
                    }
                }
            }
            let compensation_panic = self.drain_effects_captured(compensation, sampled);
            drop(compensation_panic);
            std::panic::resume_unwind(payload);
        }

        let mut state = self.state.lock();
        let now = state.commit_time(sampled);
        match state.tickets.get(&ticket).cloned() {
            // A ticket is never rebound, so the authoritative post-effect
            // permit identity is the one stored in its transient state.
            Some(TicketState::Claiming(claimed_permit)) => {
                let Some(record) = state.permits.get_mut(&claimed_permit) else {
                    state.record_terminal_ticket(ticket, TerminalReason::Released);
                    return ClaimOutcome::Terminal(TerminalReason::Released);
                };
                record.pending_ticket = None;
                record.ledger.last_touched_ms = record.ledger.last_touched_ms.max(now);
                state.tickets.remove(&ticket);
                ClaimOutcome::Ready(claimed_permit)
            }
            Some(TicketState::Terminal(reason)) => {
                state.forget_terminal_ticket(ticket);
                ClaimOutcome::Terminal(reason)
            }
            Some(TicketState::Faulted) => {
                state.forget_terminal_ticket(ticket);
                ClaimOutcome::Terminal(TerminalReason::AccountingFault)
            }
            Some(TicketState::Granted(_) | TicketState::Queued { .. }) => ClaimOutcome::Pending,
            None => ClaimOutcome::Invalid,
        }
    }

    /// The current lifecycle position of a ticket, **without** consuming it.
    ///
    /// [`Self::claim`] transfers ownership and is therefore not repeatable;
    /// this is the read-only view for diagnostics and for tests that need to
    /// observe a promotion without taking it. A `Ready` here is a statement
    /// about this instant only — the permit behind it can still be reclaimed
    /// before anyone claims it, which is exactly the race `claim` reports.
    pub fn ticket_status(&self, ticket: Ticket) -> ClaimOutcome {
        let state = self.state.lock();
        match state.tickets.get(&ticket) {
            Some(TicketState::Granted(permit_id)) => ClaimOutcome::Ready(*permit_id),
            Some(TicketState::Queued { .. } | TicketState::Claiming(_)) => ClaimOutcome::Pending,
            Some(TicketState::Terminal(reason)) => ClaimOutcome::Terminal(reason.clone()),
            Some(TicketState::Faulted) => ClaimOutcome::Terminal(TerminalReason::AccountingFault),
            None => ClaimOutcome::Invalid,
        }
    }

    /// Abandon a queued/promoted ticket (e.g. on acquire timeout). A promoted
    /// permit is released only while dispatch still owns it; an already leased
    /// worker retains its capacity and reports [`AbandonOutcome::HeldByLease`].
    pub fn abandon(&self, ticket: Ticket) -> AbandonOutcome {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        let outcome = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            match state.tickets.get(&ticket).cloned() {
                Some(TicketState::Granted(permit_id) | TicketState::Claiming(permit_id)) => {
                    match state.release_with_proof_reason(
                        permit_id,
                        None,
                        TerminalReason::Abandoned,
                        &mut effects,
                    ) {
                        ReleaseOutcome::Released => {
                            // The abandoning waiter is the one that asked; it
                            // does not need to be told, and it will not claim.
                            state.forget_terminal_ticket(ticket);
                            effects.absorb(self.promote(&mut state, now));
                            AbandonOutcome::Abandoned
                        }
                        ReleaseOutcome::HeldByLease { phase } => {
                            AbandonOutcome::HeldByLease { phase }
                        }
                        ReleaseOutcome::UnknownPermit => {
                            state.tickets.remove(&ticket);
                            AbandonOutcome::Invalid
                        }
                    }
                }
                Some(TicketState::Queued { class }) => {
                    let mut released_guard = None;
                    let position = state.classes.get(&class).and_then(|cstate| {
                        cstate
                            .queue
                            .iter()
                            .position(|request| request.ticket == ticket)
                    });
                    if let Some(pos) = position {
                        let request = state
                            .remove_pending(&class, pos)
                            .expect("position just found");
                        if matches!(request.scope, taskmesh_contract::TaskScope::Child { .. }) {
                            released_guard = Some((
                                request.root_operation_id.clone(),
                                request.target_stage.clone(),
                            ));
                        }
                        // The removed request owns a host `Arc`. Hand it to
                        // the effect list; dropping it here would run host
                        // code under this mutex.
                        if let Some(waker) = request.waker {
                            effects.retire.push(waker);
                        }
                    }
                    state.tickets.remove(&ticket);
                    // A dequeued child no longer occupies the recursion guard.
                    if let Some((root, stage)) = released_guard {
                        state.vacate_recursion_guard(&root, &stage);
                    }
                    // A cancelled request never received service, so it leaves no
                    // virtual-time debt and no idle DRR credit behind it.
                    let policy = self
                        .policy
                        .class(&class)
                        .expect("queued class belongs to the validated policy");
                    fairness::on_unserved_removed(&mut state, &class, policy);
                    let pass = self.promote(&mut state, now);
                    effects.absorb(pass);
                    AbandonOutcome::Abandoned
                }
                Some(TicketState::Terminal(_) | TicketState::Faulted) => {
                    state.forget_terminal_ticket(ticket);
                    AbandonOutcome::TerminalDiscarded
                }
                None => AbandonOutcome::Invalid,
            }
        };
        self.drain_effects(effects, sampled);
        outcome
    }

    // ---- permit lifecycle -------------------------------------------------

    /// Release an **undispatched** permit, returning its resources and
    /// promoting queued work.
    ///
    /// This is the release for a permit that never left `DispatchReserved`: a
    /// direct `admit` an embedder owns outright, or a reservation a host
    /// returns unstarted. A permit that has been advanced past that phase
    /// belongs to the [`LeaseToken`] minted by [`Self::advance_phase`], and this
    /// call answers [`ReleaseOutcome::HeldByLease`] without touching it — a
    /// running job's capacity cannot be refunded under it by anyone who merely
    /// knows its (sequential) id. Use [`Self::release_leased`] with the token.
    ///
    /// The outcome is never a no-op to shrug at: `UnknownPermit` is either a
    /// double release or a lease the sweep already reclaimed, and the holder
    /// has to know which path it is on (see [`ReleaseOutcome`]).
    pub fn release(&self, permit_id: PermitId) -> ReleaseOutcome {
        self.release_with_proof(permit_id, None)
    }

    /// Release a **dispatched** permit by spending its lease.
    ///
    /// Releases exactly when the token is the live lease of the permit it
    /// names: the ledger's minted nonce must equal the token's. No live permit
    /// with that id answers `UnknownPermit` (the lease was already spent, or
    /// the id was never granted here); a live permit whose lease is a
    /// different token answers `HeldByLease` and stays charged (see
    /// [`ReleaseOutcome::HeldByLease`] for when that can happen). The token is
    /// consumed either way — it is not `Clone`, so a lease cannot be spent
    /// twice by construction.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "spending the lease consumes the token: a value that could be reused is the defect"
    )]
    pub fn release_leased(&self, token: LeaseToken) -> ReleaseOutcome {
        self.release_with_proof(token.permit_id, Some(token.nonce))
    }

    fn release_with_proof(&self, permit_id: PermitId, proof: Option<u64>) -> ReleaseOutcome {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        let outcome = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            let outcome = state.release_with_proof(permit_id, proof, &mut effects);
            if outcome == ReleaseOutcome::Released {
                let pass = self.promote(&mut state, now);
                effects.absorb(pass);
            }
            outcome
        };
        self.drain_effects(effects, sampled);
        outcome
    }

    /// Advance a live permit's ownership phase (`DispatchReserved` → `Accepted`
    /// → `Running` → `CleanupPending`).
    ///
    /// Phases are what keep a caller's answer separate from the work's custody:
    /// a timed-out caller has its result while the permit is still `Running`, and
    /// the gauges say so instead of reporting the worker as gone. Monotonic —
    /// a backwards or repeated declaration is `Refused` and changes nothing.
    ///
    /// The first advance out of `DispatchReserved` **transfers custody**: it
    /// answers [`AdvanceOutcome::Leased`] with the permit's one
    /// [`LeaseToken`], and from then on only [`Self::release_leased`] with that
    /// token ends the permit — [`Self::release`] answers `HeldByLease`. Keep
    /// the token with the work. Later advances answer
    /// [`AdvanceOutcome::Advanced`].
    ///
    /// An embedder that drives the engine directly (via `taskmesh::ext`) owes
    /// these transitions: the leak sweep reclaims only `DispatchReserved`
    /// permits, so work that is never advanced to `Running` is *reclaimable*
    /// while it runs, and `started_total`/the phase gauges only mean something
    /// if the phases are declared.
    pub fn advance_phase(&self, permit_id: PermitId, phase: ExecutionPhase) -> AdvanceOutcome {
        self.state.lock().advance_phase(permit_id, phase)
    }

    /// The current ownership phase of a live permit.
    pub fn phase(&self, permit_id: PermitId) -> Option<ExecutionPhase> {
        self.state.lock().permits.get(&permit_id).map(|r| r.phase)
    }

    /// Promote queued work into freed capacity, fairly, up to the promotion
    /// budget. Returns the effects to apply *after* the lock is released,
    /// including whether runnable work remained.
    #[must_use]
    fn promote(&self, state: &mut GovernedState, now: u64) -> TransitionEffects {
        let mut effects = TransitionEffects::default();
        let removed_before_grants = matches!(
            self.terminalize_wait_cycle(state, &mut effects),
            CycleTerminalization::Removed
        );
        let mut pending_state_changed = removed_before_grants;
        let mut budget_exhausted = false;
        let mut grant_slots = [(); PROMOTION_BUDGET].into_iter();
        loop {
            let Some(()) = grant_slots.next() else {
                budget_exhausted = true;
                break;
            };
            let Some(selection) = fairness::select(state, &self.policy) else {
                break;
            };
            let class = selection.class;
            let Some(head) = state.remove_pending(&class, selection.queue_index) else {
                break;
            };
            let policy = self
                .policy
                .class(&class)
                .expect("queued class belongs to the validated policy");
            fairness::on_request_served(
                state,
                &class,
                policy,
                selection.queue_index,
                selection.service_finish_tag,
            );
            state.tickets.remove(&head.ticket);
            let permit_id = head.permit_id;
            let parent_permit_id = head
                .parent_permit_id
                .or_else(|| state.parent_permit_for_scope(&head.root_operation_id, &head.scope));
            state.grant(GrantRequest {
                permit_id,
                class: &class,
                operation: &head.operation,
                root_operation_id: &head.root_operation_id,
                scope: head.scope.clone(),
                parent_permit_id,
                target_stage: head.target_stage.clone(),
                provenance: head.provenance,
                capabilities: head.capabilities.clone(),
                cost: head.cost,
                reserved_units: head.cost.memory_units,
                now_ms: now,
                pending_ticket: Some(head.ticket),
                // Retained until the claim so a permit that dies first can hand
                // its waiter a terminal answer instead of leaving it parked.
                claim_waker: head.waker.clone(),
            });
            if let Some(waker) = head.waker {
                effects.wake.push(waker);
            }
            pending_state_changed = true;
        }
        // A removed cycle or a grant changes the pending-state assessment. Scan
        // once more, never once per removal: repeated full-queue scans under one
        // mutex make N cycle requests O(N^2). If this removes another request,
        // the continuation flag below releases the lock before scanning again.
        let removed_after_change = pending_state_changed
            && matches!(
                self.terminalize_wait_cycle(state, &mut effects),
                CycleTerminalization::Removed
            );
        // Ask without selecting only after the bounded pass spent every grant
        // slot. If the selector and the pure runnable probe ever disagree, a
        // continuation must fail closed instead of spinning forever under a
        // host transition. Removing a cycle is separate progress and earns one
        // fresh pass because it may expose a newly runnable head.
        effects.more_runnable = removed_after_change
            || (budget_exhausted && fairness::has_runnable(state, &self.policy));
        // Every pass records whether a continuation is owed. A pass that drained
        // the runnable set (or found nothing) clears the flag, whichever
        // transition ran it.
        state.promotion_pending = effects.more_runnable;
        effects
    }

    /// Fail-stop a queue after an accounting fault. Preserve exact ticket
    /// answers until each waiter claims or abandons, even beyond the ordinary
    /// terminal-ring limit, and retire host references outside the lock.
    fn terminalize_queued_for_accounting_fault(
        &self,
        state: &mut GovernedState,
        effects: &mut TransitionEffects,
    ) {
        let classes: Vec<TaskClass> = state.classes.keys().cloned().collect();
        for class in classes {
            let mut removed_any = false;
            while let Some(request) = state.remove_pending(&class, 0) {
                removed_any = true;
                if matches!(request.scope, taskmesh_contract::TaskScope::Child { .. }) {
                    state.vacate_recursion_guard(&request.root_operation_id, &request.target_stage);
                }
                state.tickets.insert(request.ticket, TicketState::Faulted);
                if let Some(waker) = request.waker {
                    effects.wake.push(waker);
                }
            }
            if removed_any {
                let policy = self
                    .policy
                    .class(&class)
                    .expect("queued class belongs to the validated policy");
                fairness::on_unserved_removed(state, &class, policy);
            }
        }
        state.promotion_pending = false;
    }

    fn terminalize_wait_cycle(
        &self,
        state: &mut GovernedState,
        effects: &mut TransitionEffects,
    ) -> CycleTerminalization {
        let candidate = state.classes.iter().find_map(|(class, cstate)| {
            if cstate.queued_awaited_children.is_empty() {
                return None;
            }
            let policy = self.policy.class(class)?;
            cstate
                .queue
                .iter()
                .enumerate()
                .filter(|(_, request)| cstate.queued_awaited_children.contains(&request.seq_no))
                .find_map(|(index, request)| {
                    match admission::pending::assess_pending(
                        state,
                        &self.policy,
                        request,
                        policy.max_inflight,
                        false,
                    ) {
                        admission::pending::CapacityAssessment::IrreversibleWaitCycle(witness) => {
                            Some((class.clone(), index, witness))
                        }
                        _ => None,
                    }
                })
        });
        let Some((class, index, witness)) = candidate else {
            return CycleTerminalization::None;
        };
        let Some(request) = state.remove_pending(&class, index) else {
            return CycleTerminalization::None;
        };
        let policy = self
            .policy
            .class(&class)
            .expect("queued class belongs to the validated policy");
        fairness::on_unserved_removed(state, &class, policy);
        state.vacate_recursion_guard(&request.root_operation_id, &request.target_stage);
        let held_by_parent = witness
            .held
            .into_iter()
            .next()
            .expect("cycle witness has held capacity");
        state.record_terminal_ticket(
            request.ticket,
            TerminalReason::IrreversibleWaitCycle { held_by_parent },
        );
        if let Some(waker) = request.waker {
            effects.wake.push(waker);
        }
        CycleTerminalization::Removed
    }

    /// Fire and retire host references collected by a transition, then resume
    /// promotion if the budget cut a pass short.
    ///
    /// A panicking host port is a contract fault and is propagated — but only
    /// after the whole drain has finished: every waiter in every continuation
    /// pass is notified first. Aborting the drain on the first panic would leave
    /// runnable work queued with no event left to promote it.
    fn drain_effects(&self, effects: TransitionEffects, sampled: u64) {
        if let Some(payload) = self.drain_effects_captured(effects, sampled) {
            std::panic::resume_unwind(payload);
        }
    }

    fn drain_effects_captured(
        &self,
        mut effects: TransitionEffects,
        sampled: u64,
    ) -> Option<Box<dyn std::any::Any + Send>> {
        let mut first_panic: Option<Box<dyn std::any::Any + Send>> = None;
        loop {
            let more = effects.more_runnable;
            if let Some(payload) = self.apply_effects(std::mem::take(&mut effects)) {
                first_panic.get_or_insert(payload);
            }
            if more {
                let mut state = self.state.lock();
                let now = state.commit_time(sampled);
                effects = self.promote(&mut state, now);
            } else {
                break;
            }
        }
        if let Some(waker) = &self.settlement_waker {
            if let Err(payload) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| waker.wake()))
            {
                first_panic.get_or_insert(payload);
            }
        }
        first_panic
    }

    /// Run the host-visible part of a transition with the mutex released.
    ///
    /// Returns the first panic payload raised by a host port instead of
    /// propagating it, so the caller can finish notifying everyone else first.
    /// `wake()` and the destructor are both host code and both are contained.
    fn apply_effects(&self, effects: TransitionEffects) -> Option<Box<dyn std::any::Any + Send>> {
        let TransitionEffects { wake, retire, .. } = effects;
        let mut first_panic: Option<Box<dyn std::any::Any + Send>> = None;
        for waker in wake {
            if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                waker.wake();
            })) {
                first_panic.get_or_insert(payload);
            }
            if let Err(payload) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(waker)))
            {
                first_panic.get_or_insert(payload);
            }
        }
        // Explicit: these exist so their destructors run *here*, not under the
        // state mutex.
        for waker in retire {
            if let Err(payload) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                    drop(waker);
                }))
            {
                first_panic.get_or_insert(payload);
            }
        }
        first_panic
    }

    // ---- composite (T06) --------------------------------------------------

    /// Validate that every fan-out stage carries a complete reduce policy.
    pub fn validate_reduce(spec: &TaskSpec) -> Result<(), GovernorError> {
        composite::reduce::validate_spec(spec)
    }

    /// Aggregated accounting for a root operation and its children, or `None`
    /// if no children are currently attributed to it.
    pub fn root_attribution(&self, root_operation_id: &str) -> Option<RootAttribution> {
        let state = self.state.lock();
        state.roots.get(root_operation_id).map(|r| RootAttribution {
            child_inflight: r.child_inflight,
            cpu_units: r.cpu_units,
            memory_units: r.memory_units,
            active_stages: r.active_stages.len(),
        })
    }

    /// Classification provenance (`source`/`reason`) of a live permit — "why this
    /// class" stays auditable in runtime state after submit, not just on the
    /// inbound `TaskSpec`.
    pub fn permit_provenance(&self, permit_id: PermitId) -> Option<Provenance> {
        self.state
            .lock()
            .permits
            .get(&permit_id)
            .map(|r| r.provenance.clone())
    }

    /// The checkpoint policy declared for a class.
    pub fn checkpoint_policy(&self, class: &TaskClass) -> Option<CheckpointPolicy> {
        self.policy.class(class).map(|p| p.checkpoint_policy)
    }

    // ---- inventory (T08) --------------------------------------------------

    /// The substrate registry snapshot, ordered by name.
    pub fn substrates(&self) -> Vec<SubstrateRecord> {
        inventory::snapshot(self.policy.substrates())
    }
}

fn violation(message: String) -> GovernorError {
    GovernorError::PolicyViolation(message.into())
}

/// Diagnostic projection of one queued request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingView {
    pub class: TaskClass,
    pub request_key: crate::shared::RequestKey,
    /// The limit that forced this request to queue.
    pub blocked_on: CapacityBlock,
    pub assessment: admission::pending::CapacityAssessment,
    /// Logical commit time at which the request entered the queue.
    pub enqueued_at_ms: u64,
    /// Logical time spent queued so far.
    pub queue_wait_ms: u64,
    /// The capability pool frozen at intake.
    pub capabilities: CapabilityRequirementSet,
}

/// Diagnostic projection of one live permit ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermitLedgerView {
    pub permit_id: PermitId,
    pub class: TaskClass,
    pub capabilities: CapabilityRequirementSet,
    pub phase: ExecutionPhase,
    pub cpu_units: u32,
    /// What the class policy originally reserved. Provenance, not a live claim.
    pub original_estimate_units: u32,
    /// The part of that reservation still held.
    pub remaining_reservation_units: u32,
    /// What the ledger currently charges.
    pub effective_units: u32,
    pub measured_bytes: u64,
    pub measurement_epoch: u64,
    /// Logical commit time the lease was created at.
    pub leased_at_ms: u64,
    /// Logical commit time of the most recent accepted activity.
    pub last_touched_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_allocation_rejects_exhaustion_without_wrapping() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(Governor::take_sequence(&counter), Some(u64::MAX - 1));
        assert_eq!(Governor::take_sequence(&counter), None);
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn accounting_fault_accessor_reports_the_sticky_state_reason() {
        let governor = Governor::new(
            PolicySet::new(taskmesh_contract::ResourceBudget::new(), BTreeMap::new()),
            Arc::new(taskmesh_contract::ManualClock::new(0)),
        )
        .expect("empty policy is valid");

        governor
            .state
            .lock()
            .record_accounting_fault("test overflow");

        assert_eq!(governor.accounting_fault(), Some("test overflow"));
    }
}
