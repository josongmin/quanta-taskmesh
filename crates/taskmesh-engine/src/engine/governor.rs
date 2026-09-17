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
    GovernorError, MemoryOvercommitPolicy, MemoryPermitMode, OverflowPolicy, PermitWaker, Snapshot,
    SubstrateKind, SubstrateRecord, TaskClass, TaskSpec, SNAPSHOT_SCHEMA_VERSION,
};

use crate::engine::state::{
    CapacityBlock, ClaimOutcome, GovernedState, GrantRequest, ReleaseOutcome, TerminalReason,
    TicketState, TransitionEffects,
};
use crate::features::memory::ReconcileOutcome;
use crate::features::{admission, composite, fairness, inventory, memory};
use crate::shared::{
    AdmissionDecision, CapabilityName, LeakSweepReport, PermitId, PolicySet, Provenance,
    RootAttribution, StageReleaseOutcome, Ticket,
};
use crate::sync::{AtomicU64, Mutex, Ordering};

/// Permits granted in one promotion pass before the lock is released.
///
/// The bound exists so a large freed budget cannot turn one release into an
/// arbitrarily long critical section. It does not cost progress: a pass that
/// stops early says so, and the caller resumes immediately.
pub const PROMOTION_BUDGET: usize = 64;

/// The governed execution control point. Cheap to wrap in `Arc` and share.
///
/// `Debug` prints the policy's class set and the id counters — never the
/// guarded state, which would take the mutex inside a formatter (a host
/// `Debug` in a log line must not contend with admission).
pub struct Governor {
    policy: PolicySet,
    clock: Arc<dyn Clock>,
    next_permit: AtomicU64,
    next_ticket: AtomicU64,
    next_seq: AtomicU64,
    state: Mutex<GovernedState>,
}

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
        Ok(Self::construct(policy, clock))
    }

    /// Raw constructor (no validation). Private: the validated [`Governor::new`]
    /// and the `test-util`-gated [`Governor::new_unchecked`] both route through it.
    fn construct(policy: PolicySet, clock: Arc<dyn Clock>) -> Self {
        Self {
            policy,
            clock,
            next_permit: AtomicU64::new(1),
            next_ticket: AtomicU64::new(1),
            next_seq: AtomicU64::new(1),
            state: Mutex::new(GovernedState::default()),
        }
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
        Self::construct(policy, clock)
    }

    pub fn policy(&self) -> &PolicySet {
        &self.policy
    }

    /// Sample the driven clock. Called **before** the state lock is taken; the
    /// value is clamped to the monotonic commit watermark inside the transition.
    fn sample_clock_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    fn fresh_ids(&self) -> admission::Ids {
        admission::Ids {
            permit_id: self.next_permit.fetch_add(1, Ordering::Relaxed),
            ticket: self.next_ticket.fetch_add(1, Ordering::Relaxed),
            seq_no: self.next_seq.fetch_add(1, Ordering::Relaxed),
        }
    }

    /// The capability pool a spec resolves to by its declared substrate hint.
    /// Hosts that resolve an execution plan (e.g. a stack request that must run
    /// on a dedicated worker) pass the resolved capability explicitly instead.
    fn declared_capability(&self, spec: &TaskSpec) -> Option<CapabilityName> {
        spec.primary_substrate_hint()
            .capability_pool()
            .map(|pool| self.intern_capability(pool))
    }

    /// Resolve a pool name to its interned `Arc`. A registered pool costs a
    /// clone; an unregistered one (no substrate provides it, so it is ungated by
    /// definition) is interned on the spot. That allocation only happens for
    /// names the inventory does not know, which the built-in hint set never
    /// produces.
    fn intern_capability(&self, pool: &str) -> CapabilityName {
        self.policy
            .capability_name(pool)
            .unwrap_or_else(|| CapabilityName::from(pool))
    }

    // ---- admission --------------------------------------------------------

    /// Admit a request without a promotion waker (caller does not wait on a
    /// queue). The admission key is derived authoritatively from
    /// `spec.root_operation_id`; there is no caller-supplied key.
    pub fn admit(&self, spec: &TaskSpec) -> AdmissionDecision {
        self.admit_inner(spec, self.declared_capability(spec), None)
    }

    /// Admit a request, registering a waker that fires if the request is queued
    /// and later promoted.
    pub fn admit_waitable(
        &self,
        spec: &TaskSpec,
        waker: Arc<dyn PermitWaker>,
    ) -> AdmissionDecision {
        self.admit_inner(spec, self.declared_capability(spec), Some(waker))
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
        capability: Option<&str>,
        waker: Option<Arc<dyn PermitWaker>>,
    ) -> AdmissionDecision {
        let capability = capability.map(|pool| self.intern_capability(pool));
        self.admit_inner(spec, capability, waker)
    }

    fn admit_inner(
        &self,
        spec: &TaskSpec,
        capability: Option<CapabilityName>,
        waker: Option<Arc<dyn PermitWaker>>,
    ) -> AdmissionDecision {
        // The spec's shape is a property of the spec alone; validating it under
        // the state mutex would charge every other caller for it. A malformed
        // spec never touches the state at all, and its waker (a host `Arc`) is
        // dropped here, outside the lock, like any other retired reference.
        if let Err(verdict) = admission::precheck(spec) {
            drop(waker);
            return AdmissionDecision::Rejected(verdict);
        }
        let sampled = self.sample_clock_ms();
        let ids = self.fresh_ids();
        let request = admission::AdmissionRequest { spec, capability };
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
        let mut state = self.state.lock();
        let now = state.commit_time(sampled);
        match state.tickets.get(&ticket).cloned() {
            Some(TicketState::Granted(permit_id)) => {
                state.tickets.remove(&ticket);
                if let Some(record) = state.permits.get_mut(&permit_id) {
                    record.pending_ticket = None;
                    record.ledger.last_touched_ms = record.ledger.last_touched_ms.max(now);
                    // The waiter is here; its notifier has no further job. Drop
                    // it outside the lock like every other host reference.
                    let waker = record.claim_waker.take();
                    drop(state);
                    drop(waker);
                } else {
                    // Unreachable while `Granted` implies a live permit, and the
                    // debug invariant asserts exactly that. Fail closed rather
                    // than hand out an id with nothing behind it. The waiter is
                    // being answered right now, so nothing is retained for it.
                    return ClaimOutcome::Terminal(TerminalReason::Released);
                }
                ClaimOutcome::Ready(permit_id)
            }
            Some(TicketState::Queued { .. }) => ClaimOutcome::Pending,
            Some(TicketState::Terminal(reason)) => {
                state.forget_terminal_ticket(ticket);
                ClaimOutcome::Terminal(reason)
            }
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
            Some(TicketState::Queued { .. }) => ClaimOutcome::Pending,
            Some(TicketState::Terminal(reason)) => ClaimOutcome::Terminal(*reason),
            None => ClaimOutcome::Invalid,
        }
    }

    /// Abandon a queued/promoted ticket (e.g. on acquire timeout). Releases the
    /// permit if the ticket had already been promoted.
    pub fn abandon(&self, ticket: Ticket) {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            match state.tickets.get(&ticket).cloned() {
                Some(TicketState::Granted(permit_id)) => {
                    state.unwind(permit_id, TerminalReason::Abandoned, &mut effects);
                    // The abandoning waiter is the one that asked; it does not
                    // need to be told, and it will not claim.
                    state.forget_terminal_ticket(ticket);
                    let pass = self.promote(&mut state, now);
                    effects.absorb(pass);
                }
                Some(TicketState::Queued { class }) => {
                    let mut released_guard = None;
                    if let Some(cstate) = state.classes.get_mut(&class) {
                        if let Some(pos) = cstate.queue.iter().position(|r| r.ticket == ticket) {
                            let request = cstate.queue.remove(pos).expect("position just found");
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
                    }
                    state.tickets.remove(&ticket);
                    // A dequeued child no longer occupies the recursion guard.
                    if let Some((root, stage)) = released_guard {
                        state.vacate_recursion_guard(&root, &stage);
                    }
                    // A cancelled request never received service, so it leaves no
                    // virtual-time debt and no idle DRR credit behind it.
                    fairness::on_queue_changed(&mut state, &class);
                    let pass = self.promote(&mut state, now);
                    effects.absorb(pass);
                }
                Some(TicketState::Terminal(_)) => {
                    state.forget_terminal_ticket(ticket);
                }
                None => {}
            }
        }
        self.drain_effects(effects, sampled);
    }

    // ---- permit lifecycle -------------------------------------------------

    /// Release a permit, returning its resources and promoting queued work.
    ///
    /// The outcome says whether there was a permit to release. `UnknownPermit`
    /// is never a no-op to shrug at: it is either a double release or a lease
    /// the sweep already reclaimed, and the holder has to know which path it is
    /// on (see [`ReleaseOutcome`]).
    pub fn release(&self, permit_id: PermitId) -> ReleaseOutcome {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            if state
                .unwind(permit_id, TerminalReason::Released, &mut effects)
                .is_none()
            {
                return ReleaseOutcome::UnknownPermit;
            }
            let pass = self.promote(&mut state, now);
            effects.absorb(pass);
        }
        self.drain_effects(effects, sampled);
        ReleaseOutcome::Released
    }

    /// Advance a live permit's ownership phase (`DispatchReserved` → `Accepted`
    /// → `Running` → `CleanupPending`).
    ///
    /// Phases are what keep a caller's answer separate from the work's custody:
    /// a timed-out caller has its result while the permit is still `Running`, and
    /// the gauges say so instead of reporting the worker as gone. Monotonic —
    /// a backwards or repeated declaration returns `false` and changes nothing.
    ///
    /// An embedder that drives the engine directly (via `taskmesh::ext`) owes
    /// these transitions: the leak sweep reclaims only `DispatchReserved`
    /// permits, so work that is never advanced to `Running` is *reclaimable*
    /// while it runs, and `started_total`/the phase gauges only mean something
    /// if the phases are declared.
    #[must_use = "`false` means the permit is gone or the phase did not advance; a caller that ignores it runs on capacity it may not hold"]
    pub fn advance_phase(&self, permit_id: PermitId, phase: ExecutionPhase) -> bool {
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
        let mut granted = 0usize;
        loop {
            if granted >= PROMOTION_BUDGET {
                // Out of budget. Ask — without selecting — whether runnable work
                // remains: `select` charges the chosen class (DRR deficit, ring
                // cursor, WFQ virtual time) for a dispatch, and a dispatch that
                // is then not made would skew the next pass. If work remains,
                // nothing external will necessarily arrive to notice it, so say
                // so; the caller re-enters after releasing the lock.
                effects.more_runnable = fairness::has_runnable(state, &self.policy);
                break;
            }
            let Some(class) = fairness::select(state, &self.policy) else {
                break;
            };
            let Some(head) = state.class_mut(&class).queue.pop_front() else {
                break;
            };
            fairness::on_queue_changed(state, &class);
            state.tickets.remove(&head.ticket);
            let permit_id = self.next_permit.fetch_add(1, Ordering::Relaxed);
            state.grant(GrantRequest {
                permit_id,
                class: &class,
                root_operation_id: &head.root_operation_id,
                scope: head.scope.clone(),
                target_stage: head.target_stage.clone(),
                provenance: head.provenance,
                capability: head.capability.clone(),
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
            granted += 1;
        }
        // Every pass records whether a continuation is owed. A pass that drained
        // the runnable set (or found nothing) clears the flag, whichever
        // transition ran it.
        state.promotion_pending = effects.more_runnable;
        effects
    }

    /// Fire and retire host references collected by a transition, then resume
    /// promotion if the budget cut a pass short.
    ///
    /// A panicking host port is a contract fault and is propagated — but only
    /// after the whole drain has finished: every waiter in every continuation
    /// pass is notified first. Aborting the drain on the first panic would leave
    /// runnable work queued with no event left to promote it.
    fn drain_effects(&self, mut effects: TransitionEffects, sampled: u64) {
        let mut first_panic: Option<Box<dyn std::any::Any + Send>> = None;
        loop {
            let more = effects.more_runnable;
            if let Some(payload) = self.apply_effects(std::mem::take(&mut effects)) {
                first_panic.get_or_insert(payload);
            }
            if !more {
                break;
            }
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            effects = self.promote(&mut state, now);
        }
        if let Some(payload) = first_panic {
            std::panic::resume_unwind(payload);
        }
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

    // ---- memory governance (T05) -----------------------------------------

    /// Reconcile a permit's held memory against a measured byte reading.
    ///
    /// A *downward* reconcile frees memory, so queued work is promoted. An
    /// *upward* reconcile records measured reality even if it pushes held memory
    /// past `max_memory_units`; the cap is then enforced fail-closed for *new*
    /// admissions, so an overcommitting in-flight task cannot let new work in.
    /// Observed overage is real memory: it is recorded as pressure, never
    /// discarded to keep the total under the cap.
    ///
    /// The epoch is assigned *inside* the same transition that applies the
    /// reading (the next one after whatever is currently recorded). Reading the
    /// epoch in one critical section and applying under another would let two
    /// unordered reporters pick the same "next" epoch and have the later commit
    /// rejected as stale — a lost measurement dressed up as ordering.
    pub fn reconcile_memory(&self, permit_id: PermitId, measured_bytes: u64) -> bool {
        self.reconcile_memory_inner(permit_id, measured_bytes, None)
            .is_applied()
    }

    /// Reconcile with an explicit measurement epoch.
    ///
    /// Concurrent reporters need an order. Without one, a reading that was taken
    /// first but committed last silently overwrites newer truth — including the
    /// lease activity timestamp, which then makes live work look stale.
    pub fn reconcile_memory_at(
        &self,
        permit_id: PermitId,
        measured_bytes: u64,
        epoch: u64,
    ) -> ReconcileOutcome {
        self.reconcile_memory_inner(permit_id, measured_bytes, Some(epoch))
    }

    fn reconcile_memory_inner(
        &self,
        permit_id: PermitId,
        measured_bytes: u64,
        epoch: Option<u64>,
    ) -> ReconcileOutcome {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        let outcome = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            let Some(record) = state.permits.get(&permit_id) else {
                return ReconcileOutcome::UnknownPermit;
            };
            let Some(policy) = self.policy.class(&record.class) else {
                return ReconcileOutcome::UnknownPermit;
            };
            let epoch = epoch.unwrap_or_else(|| record.ledger.measurement_epoch.wrapping_add(1));
            let mode = policy.memory_permit_mode;
            let scale = self.policy.resources.memory_unit_scale;
            let outcome = memory::reconcile(
                &mut state,
                permit_id,
                measured_bytes,
                epoch,
                mode,
                scale,
                now,
            );
            if outcome.is_applied() {
                // Promote unconditionally on success: a downward reconcile freed
                // budget; an upward one leaves no capacity so promote is a no-op.
                let pass = self.promote(&mut state, now);
                effects.absorb(pass);
            }
            outcome
        };
        self.drain_effects(effects, sampled);
        outcome
    }

    /// Return `freed_units` of a still-held permit's memory at a stage boundary.
    ///
    /// Governed by the class's [`taskmesh_contract::MemoryReleasePolicy`] (D02): an
    /// `OnTaskCompletion` class does not get an early release, and being told so
    /// is the point — folding a policy refusal into "freed 0 units" is
    /// indistinguishable from a permit that held nothing.
    pub fn release_stage_memory(
        &self,
        permit_id: PermitId,
        freed_units: u32,
    ) -> StageReleaseOutcome {
        self.release_stage_memory_inner(permit_id, freed_units, None)
    }

    /// Stage release with an explicit, strictly increasing sequence number.
    /// A duplicate or reordered stage event is rejected instead of double-applied.
    pub fn release_stage_memory_seq(
        &self,
        permit_id: PermitId,
        freed_units: u32,
        sequence: u64,
    ) -> StageReleaseOutcome {
        self.release_stage_memory_inner(permit_id, freed_units, Some(sequence))
    }

    fn release_stage_memory_inner(
        &self,
        permit_id: PermitId,
        freed_units: u32,
        sequence: Option<u64>,
    ) -> StageReleaseOutcome {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        let outcome = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            let Some(record) = state.permits.get(&permit_id) else {
                return StageReleaseOutcome::UnknownPermit;
            };
            let Some(policy) = self.policy.class(&record.class) else {
                return StageReleaseOutcome::UnknownPermit;
            };
            let release_policy = policy.memory_release_policy;
            let outcome = memory::release_stage_memory(
                &mut state,
                permit_id,
                freed_units,
                release_policy,
                sequence,
                now,
            );
            if outcome.freed_units() > 0 {
                let pass = self.promote(&mut state, now);
                effects.absorb(pass);
            }
            outcome
        };
        self.drain_effects(effects, sampled);
        outcome
    }

    /// Sweep for leaked permits using the default staleness window.
    pub fn reap_leaks(&self) -> LeakSweepReport {
        self.reap_leaks_with(memory::DEFAULT_LEAK_STALE_MS)
    }

    /// Sweep for leaked permits older than `stale_after_ms`.
    ///
    /// Only permits that never reached an executor are reclaimed; a stale lease
    /// on running work is reported in `retained_active` and left charged.
    pub fn reap_leaks_with(&self, stale_after_ms: u64) -> LeakSweepReport {
        let sampled = self.sample_clock_ms();
        let mut effects = TransitionEffects::default();
        let report = {
            let mut state = self.state.lock();
            let now = state.commit_time(sampled);
            let report =
                memory::reap_leaks(&mut state, &self.policy, now, stale_after_ms, &mut effects);
            if report.reclaimed_permits > 0 {
                let pass = self.promote(&mut state, now);
                effects.absorb(pass);
            }
            report
        };
        self.drain_effects(effects, sampled);
        report
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
            .map(|r| r.provenance)
    }

    /// The (preserved) checkpoint policy for a class.
    pub fn checkpoint_policy(&self, class: &TaskClass) -> Option<CheckpointPolicy> {
        self.policy
            .class(class)
            .map(|p| composite::checkpoint_hooks(&p.checkpoint_policy))
    }

    // ---- inventory (T08) --------------------------------------------------

    /// The substrate registry snapshot, ordered by name.
    pub fn substrates(&self) -> Vec<SubstrateRecord> {
        inventory::snapshot(self.policy.substrates())
    }

    // ---- observation ------------------------------------------------------

    pub fn snapshot(&self) -> Snapshot {
        let state = self.state.lock();
        let mut classes: BTreeMap<TaskClass, ClassSnapshot> = BTreeMap::new();
        // O(classes): per-class held totals are maintained incrementally in
        // grant/unwind/reconcile, so no permit scan is needed.
        let known = self
            .policy
            .classes
            .keys()
            .chain(state.classes.keys())
            .cloned();
        for class in known {
            let projection = state
                .classes
                .get(&class)
                .map_or_else(ClassSnapshot::default, |c| ClassSnapshot {
                    inflight: c.inflight,
                    queued: c.queued(),
                    cpu_units_held: c.cpu_units_held,
                    memory_units_held: c.memory_units_held,
                    dispatch_reserved: c.dispatch_reserved,
                    accepted: c.accepted,
                    running: c.running,
                    cleanup_pending: c.cleanup_pending,
                    admitted_total: c.admitted_total,
                    started_total: c.started_total,
                    terminated_total: c.terminated_total,
                });
            classes.entry(class).or_insert(projection);
        }
        // Every registered pool is always present — gated (`limit > 0`) or
        // ungated (`limit == 0`) — so a consumer can index `capabilities["x"]`
        // and a dashboard never sees a key appear and vanish with occupancy.
        let mut capabilities: BTreeMap<String, CapabilityUsage> = BTreeMap::new();
        for record in self.policy.substrates().values() {
            if let Some(pool) = record.capability_pool.as_deref() {
                capabilities
                    .entry(pool.to_owned())
                    .or_insert_with(|| CapabilityUsage {
                        in_use: state.capability_in_use(pool),
                        limit: 0,
                    });
            }
        }
        for (pool, limit) in self.policy.capability_limits() {
            capabilities.insert(
                pool.clone(),
                CapabilityUsage {
                    in_use: state.capability_in_use(pool),
                    limit: *limit,
                },
            );
        }
        for (pool, in_use) in &state.capability_in_use {
            capabilities
                .entry(pool.to_string())
                .or_insert(CapabilityUsage {
                    in_use: *in_use,
                    limit: 0,
                });
        }
        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            classes,
            substrates: inventory::snapshot(self.policy.substrates()),
            capabilities,
        }
    }

    /// Diagnostic view of a queued ticket: why it is waiting and since when.
    ///
    /// The blocking cause is preserved from intake so an acquire timeout can
    /// report what the caller was actually waiting on — a saturated capability
    /// pool and a saturated class budget are different operational problems and
    /// a single generic timeout hides which one occurred.
    pub fn pending_view(&self, ticket: Ticket) -> Option<PendingView> {
        let state = self.state.lock();
        let TicketState::Queued { class } = state.tickets.get(&ticket)? else {
            return None;
        };
        let cstate = state.classes.get(class)?;
        let request = cstate.queue.iter().find(|r| r.ticket == ticket)?;
        Some(PendingView {
            class: request.class.clone(),
            request_key: request.request_key.clone(),
            blocked_on: request.blocked_on,
            enqueued_at_ms: request.enqueued_at_ms,
            queue_wait_ms: state
                .commit_watermark_ms
                .saturating_sub(request.enqueued_at_ms),
            capability: request.capability.clone(),
        })
    }

    /// Why a queued ticket is waiting, or `None` if it is not queued.
    pub fn pending_block_reason(&self, ticket: Ticket) -> Option<CapacityBlock> {
        self.pending_view(ticket).map(|view| view.blocked_on)
    }

    /// Full ledger view of a live permit.
    ///
    /// Exposed so an *independent* observer can recompute the aggregates rather
    /// than re-reading the same incrementally-maintained counters the engine
    /// uses — a conservation check that shares its arithmetic with the thing it
    /// checks proves nothing.
    pub fn permit_ledger(&self, permit_id: PermitId) -> Option<PermitLedgerView> {
        let state = self.state.lock();
        let record = state.permits.get(&permit_id)?;
        Some(PermitLedgerView {
            permit_id,
            class: record.class.clone(),
            capability: record.capability.clone(),
            phase: record.phase,
            cpu_units: record.ledger.cpu_units,
            original_estimate_units: record.ledger.original_estimate_units,
            remaining_reservation_units: record.ledger.remaining_reservation_units,
            effective_units: record.ledger.effective_units,
            measured_bytes: record.ledger.measured_bytes,
            measurement_epoch: record.ledger.measurement_epoch,
            leased_at_ms: record.ledger.leased_at_ms,
            last_touched_ms: record.ledger.last_touched_ms,
        })
    }

    /// Every live permit ledger, for an independent conservation oracle.
    pub fn permit_ledgers(&self) -> Vec<PermitLedgerView> {
        let state = self.state.lock();
        state
            .permits
            .values()
            .map(|record| PermitLedgerView {
                permit_id: record.permit_id,
                class: record.class.clone(),
                capability: record.capability.clone(),
                phase: record.phase,
                cpu_units: record.ledger.cpu_units,
                original_estimate_units: record.ledger.original_estimate_units,
                remaining_reservation_units: record.ledger.remaining_reservation_units,
                effective_units: record.ledger.effective_units,
                measured_bytes: record.ledger.measured_bytes,
                measurement_epoch: record.ledger.measurement_epoch,
                leased_at_ms: record.ledger.leased_at_ms,
                last_touched_ms: record.ledger.last_touched_ms,
            })
            .collect()
    }

    /// The sticky accounting-fault reason, if the ledger ever failed to
    /// represent a total. While set, admission is refused.
    pub fn accounting_fault(&self) -> Option<&'static str> {
        self.state.lock().accounting_fault
    }

    /// How many terminal ticket records are currently retained for late
    /// claimers. Never exceeds [`crate::MAX_TERMINAL_TICKETS`].
    pub fn retained_terminal_tickets(&self) -> usize {
        self.state.lock().retained_terminal_tickets()
    }

    /// Ring positions the DRR scheduler has examined on this governor — the
    /// work oracle behind TM16-012 (`O(runnable classes)` per selection, never
    /// `O(cost / quantum)`).
    #[cfg(feature = "test-util")]
    #[doc(hidden)]
    pub fn drr_ring_visits(&self) -> u64 {
        self.state.lock().drr_ring_visits
    }

    // ---- validation (T02) -------------------------------------------------

    /// Fail-closed configuration validation. Every impossible budget is rejected
    /// at construction, never deferred to a runtime admit path.
    pub fn validate_policy(policy: &PolicySet) -> Result<(), GovernorError> {
        Self::validate_inventory(policy)?;
        let budget = &policy.resources;
        for (class, class_policy) in &policy.classes {
            let cost = class_policy.permit_cost;

            if budget.per_request_max_cpu_units != 0
                && cost.cpu_units > budget.per_request_max_cpu_units
            {
                return Err(violation(format!(
                    "class {class} exceeds per-request cpu limit"
                )));
            }
            if budget.per_request_max_memory_units != 0
                && cost.memory_units > budget.per_request_max_memory_units
            {
                return Err(violation(format!(
                    "class {class} exceeds per-request memory limit"
                )));
            }
            if budget.max_cpu_units != 0 && cost.cpu_units > budget.max_cpu_units {
                return Err(violation(format!(
                    "class {class} exceeds global cpu budget"
                )));
            }
            if budget.max_memory_units != 0 && cost.memory_units > budget.max_memory_units {
                return Err(violation(format!(
                    "class {class} exceeds global memory budget"
                )));
            }

            if matches!(
                class_policy.memory_permit_mode,
                MemoryPermitMode::Measured | MemoryPermitMode::Hybrid
            ) && budget.memory_unit_scale.bytes_per_unit == 0
            {
                return Err(violation(format!(
                    "class {class}: measured/hybrid memory requires bytes_per_unit > 0"
                )));
            }

            if class_policy.overflow_policy == OverflowPolicy::QueueWithinDepth
                && class_policy.max_queue_depth == 0
            {
                return Err(violation(format!(
                    "class {class} is queueable but has max_queue_depth == 0"
                )));
            }

            // The scavenger discipline only dispatches in the best-effort tier;
            // declaring it on a non-best-effort class is contradictory.
            if class_policy.fairness == FairnessPolicy::BestEffortScavenger
                && !class_policy.best_effort
            {
                return Err(violation(format!(
                    "class {class} uses BestEffortScavenger fairness but is not best_effort"
                )));
            }

            // A zero weight has no proportional meaning. Coercing it to 1 would
            // make an obviously wrong config look like a deliberate one.
            if let FairnessPolicy::WeightedFairQueue { weight, .. } = class_policy.fairness {
                if weight == 0 {
                    return Err(violation(format!(
                        "class {class} declares WeightedFairQueue weight 0; weight must be >= 1"
                    )));
                }
            }

            if class_policy.memory_overcommit_policy == MemoryOvercommitPolicy::Queue
                && class_policy.max_queue_depth == 0
            {
                return Err(violation(format!(
                    "class {class} queues on overcommit but has max_queue_depth == 0"
                )));
            }

            if let MemoryOvercommitPolicy::DegradeToLight { fallback_class } =
                &class_policy.memory_overcommit_policy
            {
                if fallback_class == class {
                    return Err(violation(format!("class {class} degrades to itself")));
                }
                let Some(fallback) = policy.classes.get(fallback_class) else {
                    return Err(violation(format!(
                        "class {class} degrades to unknown fallback {fallback_class}"
                    )));
                };
                // A degrade is a promise of *some* service under memory
                // pressure. Pointing it at a class that can never dispatch turns
                // that promise into a rejection with a misleading verdict.
                if fallback.is_disabled() {
                    return Err(violation(format!(
                        "class {class} degrades to disabled fallback {fallback_class}"
                    )));
                }
                // A degrade is one hop (D03): a request that already degraded
                // is admitted against the fallback's resource account without
                // consulting the fallback's own overcommit policy. A fallback
                // that itself declares `DegradeToLight` therefore promises a
                // second hop the engine never takes — and on a cycle (a → b →
                // a) the "second hop" is the class that just failed. Either
                // way the configuration says something the runtime does not
                // do, so it is refused at construction.
                if matches!(
                    fallback.memory_overcommit_policy,
                    MemoryOvercommitPolicy::DegradeToLight { .. }
                ) {
                    return Err(violation(format!(
                        "class {class} degrades to fallback {fallback_class}, which itself \
                         degrades; a degrade is one hop and cannot chain"
                    )));
                }
            }
        }

        // Fairness is a per-tier discipline: when several classes are runnable in
        // the same tier, the scheduler arbitrates them with ONE discipline. A
        // heterogeneous mix would let the lexically-first class silently impose
        // its discipline on the others, so reject it at construction. Weights/
        // quanta/slack may still differ between classes (same discipline kind);
        // only the discipline *kind* must agree within a tier. Best-effort forms
        // a separate tier. Disabled classes never dispatch and are exempt.
        let mut primary_kind: Option<std::mem::Discriminant<FairnessPolicy>> = None;
        let mut best_effort_kind: Option<std::mem::Discriminant<FairnessPolicy>> = None;
        for (class, class_policy) in &policy.classes {
            if class_policy.is_disabled() {
                continue;
            }
            let kind = std::mem::discriminant(&class_policy.fairness);
            let tier = if class_policy.best_effort {
                &mut best_effort_kind
            } else {
                &mut primary_kind
            };
            match tier {
                None => *tier = Some(kind),
                Some(existing) if *existing != kind => {
                    return Err(violation(format!(
                        "class {class} mixes a different fairness discipline within its \
                         scheduling tier; all classes in a tier must share one discipline"
                    )));
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// Validate the substrate registry as a whole.
    ///
    /// Per-record validation at registration time is not enough: a registry can
    /// still be handed over empty, keyed under a name that disagrees with the
    /// record it holds, or missing a built-in the host's dispatch table assumes
    /// exists. The built-ins are intrinsic to *any* governor, so their presence
    /// and their exact kind/pool binding are checked here, where the policy
    /// becomes live, regardless of how the registry was assembled.
    fn validate_inventory(policy: &PolicySet) -> Result<(), GovernorError> {
        let registry = policy.substrates();
        for (key, record) in registry {
            inventory::validate_record(record)?;
            if key.as_str() != record.name.as_ref() {
                return Err(violation(format!(
                    "substrate registry key {key} does not match record name {}",
                    record.name
                )));
            }
        }
        for expected in inventory::builtin_records() {
            let Some(found) = registry.get(expected.name.as_ref()) else {
                return Err(violation(format!(
                    "substrate registry is missing built-in {}",
                    expected.name
                )));
            };
            if found.kind != expected.kind || found.capability_pool != expected.capability_pool {
                return Err(violation(format!(
                    "built-in substrate {} is registered with a non-canonical kind/pool binding",
                    expected.name
                )));
            }
        }
        for (pool, limit) in policy.capability_limits() {
            if *limit == 0 {
                continue;
            }
            let bound = registry.values().any(|record| {
                record.kind != SubstrateKind::AuthorityOnly
                    && record.capability_pool.as_deref() == Some(pool.as_str())
            });
            if !bound {
                return Err(violation(format!(
                    "capability limit declared for pool {pool}, which no executing substrate provides"
                )));
            }
        }
        Ok(())
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
    /// Logical commit time at which the request entered the queue.
    pub enqueued_at_ms: u64,
    /// Logical time spent queued so far.
    pub queue_wait_ms: u64,
    /// The capability pool frozen at intake.
    pub capability: Option<CapabilityName>,
}

/// Diagnostic projection of one live permit ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermitLedgerView {
    pub permit_id: PermitId,
    pub class: TaskClass,
    pub capability: Option<CapabilityName>,
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
