//! The shared mutable state behind the governor's mutex, plus the pure capacity
//! arithmetic every feature consults. Feature *services* mutate this; feature
//! *domain* functions never touch it.
//!
//! # Invariants this module owns
//!
//! 1. **One authority.** Semantic capacity (class inflight, CPU/memory budget)
//!    and physical capability-pool occupancy are decided by the *same* check and
//!    committed by the *same* transition. There is no second ledger and no
//!    second queue in front of this one.
//! 2. **Exact arithmetic.** Aggregates are `u128` and every capacity comparison
//!    is checked. Nothing saturates: a saturated total reports *less* resource
//!    than is held, which is precisely how an over-budget admission hides.
//! 3. **One ticket lifecycle.** A ticket is `Queued`, `Granted`, or `Terminal` —
//!    never "missing", which a waiter cannot distinguish from "not yet".
//! 4. **No foreign code under the lock.** Values that own host `Arc`s
//!    (wakers) are moved into [`TransitionEffects`] and dropped after the mutex
//!    is released.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use taskmesh_contract::{ExecutionPhase, PermitWaker, TaskClass, TaskScope, TaskStage};

use crate::shared::{CapabilityName, PermitId, Provenance, RequestKey, ResolvedCost, Seq, Ticket};

/// How many terminal ticket outcomes are retained for late claimers.
///
/// Retention is bounded on purpose: an unbounded tombstone map is a leak. Past
/// the bound the oldest outcome is evicted and a late claim reports
/// [`ClaimOutcome::Invalid`] instead — still terminal, still not a hang, just
/// without the specific reason.
pub const MAX_TERMINAL_TICKETS: usize = 4096;

/// A request waiting in a class queue for capacity.
pub struct PendingRequest {
    pub ticket: Ticket,
    pub seq_no: Seq,
    pub class: TaskClass,
    /// Admission key derived from the root operation id; audit/diagnostics only.
    pub request_key: RequestKey,
    pub root_operation_id: String,
    pub scope: TaskScope,
    pub target_stage: TaskStage,
    /// Classification provenance (source/reason), preserved through promotion.
    pub provenance: Provenance,
    pub cost: ResolvedCost,
    /// The capability pool this request must occupy to run. Frozen at intake so
    /// promotion can never re-resolve it to a cheaper pool.
    pub capability: Option<CapabilityName>,
    pub enqueued_at_ms: u64,
    /// Absolute deadline used by `DeadlineAware`; `u64::MAX` when unset.
    pub deadline_ms: u64,
    /// WFQ virtual finish tag assigned at enqueue.
    pub finish_tag: u128,
    /// Which limit forced this request into the queue. Preserved so an acquire
    /// timeout reports the cause the caller was actually waiting on.
    pub blocked_on: CapacityBlock,
    pub waker: Option<Arc<dyn PermitWaker>>,
}

/// Per-class admission state and queue.
#[derive(Default)]
pub struct ClassState {
    pub inflight: u32,
    pub queue: VecDeque<PendingRequest>,
    /// WFQ: the finish tag of the most recently enqueued request.
    pub last_finish_tag: u128,
    /// DRR: accumulated deficit, in the same unit domain as head cost.
    pub deficit: u128,
    /// Running per-class CPU units held by inflight permits (snapshot in O(1)).
    pub cpu_units_held: u128,
    /// Running per-class effective memory units held by inflight permits.
    pub memory_units_held: u128,
    /// Live gauges per ownership phase; they partition `inflight`.
    pub dispatch_reserved: u32,
    pub accepted: u32,
    pub running: u32,
    pub cleanup_pending: u32,
    /// Cumulative permits granted.
    pub admitted_total: u128,
    /// Cumulative permits that reached `Running`.
    pub started_total: u128,
    /// Cumulative permits released.
    pub terminated_total: u128,
}

impl ClassState {
    pub fn queued(&self) -> u32 {
        u32::try_from(self.queue.len()).unwrap_or(u32::MAX)
    }

    fn phase_slot(&mut self, phase: ExecutionPhase) -> &mut u32 {
        match phase {
            ExecutionPhase::DispatchReserved => &mut self.dispatch_reserved,
            ExecutionPhase::Accepted => &mut self.accepted,
            ExecutionPhase::Running => &mut self.running,
            // `ExecutionPhase` is `#[non_exhaustive]`. A phase this build does
            // not know is accounted as cleanup — the most conservative bucket,
            // since it keeps the request charged and inside `inflight`.
            ExecutionPhase::CleanupPending | _ => &mut self.cleanup_pending,
        }
    }

    fn enter_phase(&mut self, phase: ExecutionPhase) {
        let slot = self.phase_slot(phase);
        *slot = slot.saturating_add(1);
    }

    fn leave_phase(&mut self, phase: ExecutionPhase) {
        let slot = self.phase_slot(phase);
        *slot = slot.saturating_sub(1);
    }
}

/// Memory/CPU ledger for one outstanding permit (T05).
///
/// The three memory quantities are deliberately distinct (D02):
///
/// - `original_estimate_units` — what the class policy reserved. Never changes;
///   it is provenance, not a live claim.
/// - `remaining_reservation_units` — the part of that estimate still held. A
///   stage-boundary release reduces it, and it never grows again: a reconcile
///   reports *measurement*, it does not re-reserve returned capacity.
/// - `effective_units` — what the ledger currently charges.
#[derive(Debug, Clone)]
pub struct PermitLedger {
    pub cpu_units: u32,
    pub original_estimate_units: u32,
    pub remaining_reservation_units: u32,
    pub measured_bytes: u64,
    /// Monotonic per-permit measurement sequence. A reading that arrives with a
    /// stale epoch is rejected instead of overwriting a newer one.
    pub measurement_epoch: u64,
    /// Monotonic per-permit stage-release sequence. A duplicate or out-of-order
    /// stage delta is rejected rather than applied twice.
    pub stage_sequence: u64,
    pub effective_units: u32,
    /// Logical commit time at which the lease was created.
    pub leased_at_ms: u64,
    /// Logical commit time of the most recent accepted activity on this permit.
    pub last_touched_ms: u64,
}

/// A granted permit and everything needed to unwind it on release.
pub struct PermitRecord {
    pub permit_id: PermitId,
    pub class: TaskClass,
    pub root_operation_id: String,
    pub scope: TaskScope,
    pub target_stage: TaskStage,
    /// Classification provenance (source/reason), auditable post-intake.
    pub provenance: Provenance,
    /// The capability pool this permit occupies while live.
    pub capability: Option<CapabilityName>,
    /// Current ownership phase. Advances monotonically.
    pub phase: ExecutionPhase,
    /// The queued ticket this permit was promoted for, until it is claimed. A
    /// permit that dies before its claim uses this to hand the waiter a terminal
    /// outcome instead of leaving it parked.
    pub pending_ticket: Option<Ticket>,
    /// Waker of the not-yet-claimed waiter, so a terminal transition can notify
    /// it. Cleared on claim; dropped outside the lock.
    pub claim_waker: Option<Arc<dyn PermitWaker>>,
    pub ledger: PermitLedger,
}

/// Aggregated accounting for one root operation and its children (T06).
#[derive(Debug, Clone, Default)]
pub struct RootExecutionState {
    pub child_inflight: u32,
    pub cpu_units: u128,
    pub memory_units: u128,
    pub active_stages: BTreeSet<TaskStage>,
}

/// Which limit, if any, blocks an admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacityBlock {
    Ok,
    Inflight,
    /// The resolved capability pool is fully occupied.
    Capability,
    Cpu,
    Memory,
    /// Capacity exists, but an earlier arrival of the same class is queued and
    /// runnable ahead of this one (D08 in-class FIFO). The newcomer waits its
    /// turn instead of overtaking work that was only waiting for a promotion pass.
    QueuedBehind,
    /// The ledger cannot represent the resulting total. Fail-closed: the engine
    /// refuses new work rather than committing a number it cannot reproduce.
    AccountingFault,
}

pub use taskmesh_contract::TerminalReason;

/// Lifecycle position of a ticket. Exactly one variant holds at any time.
#[derive(Debug, Clone)]
pub enum TicketState {
    Queued { class: TaskClass },
    Granted(PermitId),
    Terminal(TerminalReason),
}

/// The result of claiming a ticket. `Pending` and `Terminal` are *different*
/// answers: conflating them (as a bare `None` does) is what turns a reclaimed
/// permit into a waiter that parks forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "a Ready permit that is dropped is leaked capacity; handle every arm"]
pub enum ClaimOutcome {
    /// Ownership transferred to the caller.
    Ready(PermitId),
    /// Still queued; keep waiting.
    Pending,
    /// The request ended without the caller getting ownership.
    Terminal(TerminalReason),
    /// Unknown ticket: never issued, already claimed, or its terminal record
    /// aged out of the bounded retention. Terminal either way — never "wait".
    Invalid,
}

/// The result of releasing a permit.
///
/// A release that finds no permit is one of two things: a double release
/// (host bug) or a release of a lease the leak sweep already reclaimed (a real
/// race the host must be able to notice). Returning `()` for both is how either
/// stays invisible until the accounting drifts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an unknown permit on release is a double release or a reclaimed lease; handle it"]
pub enum ReleaseOutcome {
    /// The permit was live; its capacity, capability slot, and attribution are
    /// returned and queued work has been promoted.
    Released,
    /// No live permit had this id: already released, reclaimed, abandoned, or
    /// never granted by this governor.
    UnknownPermit,
}

/// Host-owned values a transition produced that must not be touched under the
/// engine mutex.
///
/// A driven port's `wake()` **and its destructor** are host code. Running either
/// under the transition lock lets the host re-enter the governor and deadlock,
/// so every such value leaves the critical section inside this struct.
#[derive(Default)]
pub struct TransitionEffects {
    /// Waiters to notify after the lock is released.
    pub wake: Vec<Arc<dyn PermitWaker>>,
    /// References whose only remaining job is to be dropped outside the lock.
    pub retire: Vec<Arc<dyn PermitWaker>>,
    /// Runnable work remained when the promotion budget ran out. The caller must
    /// re-enter promotion rather than waiting for an external event.
    pub more_runnable: bool,
}

impl TransitionEffects {
    pub fn absorb(&mut self, other: Self) {
        self.wake.extend(other.wake);
        self.retire.extend(other.retire);
        self.more_runnable |= other.more_runnable;
    }
}

/// Everything one grant needs. A struct rather than a dozen positional
/// arguments so a caller cannot transpose two of them.
pub struct GrantRequest<'a> {
    pub permit_id: PermitId,
    pub class: &'a TaskClass,
    pub root_operation_id: &'a str,
    pub scope: TaskScope,
    pub target_stage: TaskStage,
    pub provenance: Provenance,
    pub capability: Option<CapabilityName>,
    pub cost: ResolvedCost,
    pub reserved_units: u32,
    pub now_ms: u64,
    pub pending_ticket: Option<Ticket>,
    pub claim_waker: Option<Arc<dyn PermitWaker>>,
}

/// The full governed state. One instance lives behind the governor's mutex.
#[derive(Default)]
pub struct GovernedState {
    pub classes: BTreeMap<TaskClass, ClassState>,
    pub permits: BTreeMap<PermitId, PermitRecord>,
    pub roots: BTreeMap<String, RootExecutionState>,
    /// The `(root, stage)` pairs a child currently occupies — queued or granted.
    /// Keyed by root so the admit-path lookup borrows the spec's strings instead
    /// of cloning them into a tuple key.
    pub active_recursion: BTreeMap<String, BTreeSet<TaskStage>>,
    /// Single ticket index: every issued ticket is here until it is claimed or
    /// its terminal record is evicted.
    pub tickets: BTreeMap<Ticket, TicketState>,
    /// Retained terminal tickets in retention order (`retention seq → ticket`)
    /// for bounded FIFO eviction, plus the reverse index so observing one record
    /// is `O(log n)` under the state mutex rather than a scan of the window.
    terminal_order: BTreeMap<u64, Ticket>,
    terminal_index: BTreeMap<Ticket, u64>,
    terminal_seq: u64,
    /// Exact global held units.
    pub cpu_units_held: u128,
    pub memory_units_held: u128,
    /// Live occupancy per capability pool, keyed by the interned name so a
    /// grant costs a clone of an `Arc`, not a `String`.
    pub capability_in_use: BTreeMap<CapabilityName, u32>,
    /// WFQ global virtual time, advanced on each dispatch.
    pub virtual_time: u128,
    /// DRR active-class ring cursor.
    pub drr_cursor: Option<TaskClass>,
    /// Monotonic logical commit time (D07). Every committed transition stamps
    /// `max(sampled_clock, watermark)` and advances this.
    pub commit_watermark_ms: u64,
    /// Set once an accounting invariant could not be represented. While set,
    /// admission is refused: the engine will not guess a total it cannot compute.
    pub accounting_fault: Option<&'static str>,
    /// A promotion pass ran out of budget and its continuation has not run yet
    /// (the lock is released between passes). While set, capacity that opens
    /// belongs to the queue: a direct admit of a queueable class is placed in
    /// the queue for the continuation to order fairly, instead of taking the
    /// slot ahead of work that was only waiting for the next pass.
    pub promotion_pending: bool,
    /// Work oracle for the DRR scheduler: ring positions examined so far. The
    /// arithmetic scheduler examines each runnable class once per selection; a
    /// scheduler that walks the ring visit by visit examines `cost / quantum`.
    /// Per governor so a test's bound is not disturbed by other governors.
    pub drr_ring_visits: u64,
}

impl GovernedState {
    pub fn class_mut(&mut self, class: &TaskClass) -> &mut ClassState {
        self.classes.entry(class.clone()).or_default()
    }

    pub fn inflight(&self, class: &TaskClass) -> u32 {
        self.classes.get(class).map_or(0, |c| c.inflight)
    }

    pub fn queued(&self, class: &TaskClass) -> u32 {
        self.classes.get(class).map_or(0, ClassState::queued)
    }

    /// Clamp a clock sample to the monotonic commit watermark and advance it.
    /// Call this **after** taking the lock, with a sample read before it (D07).
    pub fn commit_time(&mut self, sampled_ms: u64) -> u64 {
        let committed = sampled_ms.max(self.commit_watermark_ms);
        self.commit_watermark_ms = committed;
        committed
    }

    pub fn capability_in_use(&self, capability: &str) -> u32 {
        self.capability_in_use.get(capability).copied().unwrap_or(0)
    }

    /// Pure capacity check: would a permit of `cost` for `class` fit right now,
    /// given the class inflight cap, the capability-pool limit, and the global
    /// resource budget?
    ///
    /// Every comparison is checked and widened before comparing. A total that
    /// cannot be represented is [`CapacityBlock::AccountingFault`], never a
    /// saturated value that compares as "fits".
    pub fn capacity_for(
        &self,
        class: &TaskClass,
        max_inflight: u32,
        cost: ResolvedCost,
        capability: Option<&str>,
        budget: &taskmesh_contract::ResourceBudget,
        capability_limits: &BTreeMap<String, u32>,
    ) -> CapacityBlock {
        if self.accounting_fault.is_some() {
            return CapacityBlock::AccountingFault;
        }
        if self.inflight(class) >= max_inflight {
            return CapacityBlock::Inflight;
        }
        if let Some(name) = capability {
            // An absent or zero limit means "ungated", matching the published
            // `0 == no limit` convention used by resource budgets.
            let limit = capability_limits.get(name).copied().unwrap_or(0);
            if limit != 0 && self.capability_in_use(name) >= limit {
                return CapacityBlock::Capability;
            }
        }
        if budget.max_cpu_units != 0 {
            let Some(next) = self.cpu_units_held.checked_add(u128::from(cost.cpu_units)) else {
                return CapacityBlock::AccountingFault;
            };
            if next > u128::from(budget.max_cpu_units) {
                return CapacityBlock::Cpu;
            }
        }
        if budget.max_memory_units != 0 {
            let Some(next) = self
                .memory_units_held
                .checked_add(u128::from(cost.memory_units))
            else {
                return CapacityBlock::AccountingFault;
            };
            if next > u128::from(budget.max_memory_units) {
                return CapacityBlock::Memory;
            }
        }
        CapacityBlock::Ok
    }

    /// D08 in-class FIFO: does a queued head of `class` — one that is runnable
    /// right now and only waiting for a promotion pass — stand ahead of a new
    /// arrival that itself fits?
    ///
    /// The one head that does *not* block a newcomer is one waiting on a
    /// capability pool the newcomer does not need: same class, different
    /// physical pool, and holding the newcomer back would let one saturated
    /// pool idle every other pool the class can use. That exception is the
    /// only way a later arrival of a class overtakes an earlier one.
    pub fn queued_head_blocks(
        &self,
        class: &TaskClass,
        max_inflight: u32,
        budget: &taskmesh_contract::ResourceBudget,
        capability_limits: &BTreeMap<String, u32>,
    ) -> bool {
        let Some(head) = self.classes.get(class).and_then(|c| c.queue.front()) else {
            return false;
        };
        self.capacity_for(
            class,
            max_inflight,
            head.cost,
            head.capability.as_deref(),
            budget,
            capability_limits,
        ) == CapacityBlock::Ok
    }

    /// Commit a granted permit: bump inflight, global held, capability
    /// occupancy, root accounting, and the recursion guard.
    pub fn grant(&mut self, request: GrantRequest<'_>) {
        let GrantRequest {
            permit_id,
            class,
            root_operation_id,
            scope,
            target_stage,
            provenance,
            capability,
            cost,
            reserved_units,
            now_ms,
            pending_ticket,
            claim_waker,
        } = request;

        {
            let cstate = self.class_mut(class);
            cstate.inflight += 1;
            cstate.cpu_units_held += u128::from(cost.cpu_units);
            cstate.memory_units_held += u128::from(cost.memory_units);
            cstate.admitted_total += 1;
            cstate.enter_phase(ExecutionPhase::DispatchReserved);
        }
        self.cpu_units_held += u128::from(cost.cpu_units);
        self.memory_units_held += u128::from(cost.memory_units);
        if let Some(name) = capability.as_ref() {
            let slot = self
                .capability_in_use
                .entry(CapabilityName::clone(name))
                .or_insert(0);
            *slot = slot.saturating_add(1);
        }

        // Recursion guard and root attribution are CHILD-only. A root must never
        // occupy `(root, stage)` itself, else its own first child — which targets
        // the parent stage — would be (wrongly) rejected as recursive.
        if matches!(scope, TaskScope::Child { .. }) {
            let root = self.roots.entry(root_operation_id.to_owned()).or_default();
            root.child_inflight += 1;
            root.cpu_units += u128::from(cost.cpu_units);
            root.memory_units += u128::from(cost.memory_units);
            root.active_stages.insert(target_stage.clone());
            self.occupy_recursion_guard(root_operation_id, &target_stage);
        }

        if let Some(ticket) = pending_ticket {
            self.tickets.insert(ticket, TicketState::Granted(permit_id));
        }

        self.permits.insert(
            permit_id,
            PermitRecord {
                permit_id,
                class: class.clone(),
                root_operation_id: root_operation_id.to_owned(),
                scope,
                target_stage,
                provenance,
                capability,
                phase: ExecutionPhase::DispatchReserved,
                pending_ticket,
                claim_waker,
                ledger: PermitLedger {
                    cpu_units: cost.cpu_units,
                    original_estimate_units: reserved_units,
                    remaining_reservation_units: reserved_units,
                    measured_bytes: 0,
                    measurement_epoch: 0,
                    stage_sequence: 0,
                    effective_units: cost.memory_units,
                    leased_at_ms: now_ms,
                    last_touched_ms: now_ms,
                },
            },
        );
        self.assert_consistent();
    }

    /// Advance a permit's ownership phase. Monotonic: a request never moves back
    /// to an earlier phase, and re-declaring the current phase is a no-op.
    /// Returns `false` for an unknown permit or a backwards move.
    pub fn advance_phase(&mut self, permit_id: PermitId, phase: ExecutionPhase) -> bool {
        let Some(record) = self.permits.get_mut(&permit_id) else {
            return false;
        };
        if phase.rank() <= record.phase.rank() {
            return false;
        }
        let previous = record.phase;
        record.phase = phase;
        let class = record.class.clone();
        let newly_started = !previous.has_started() && phase.has_started();
        let cstate = self.class_mut(&class);
        cstate.leave_phase(previous);
        cstate.enter_phase(phase);
        if newly_started {
            cstate.started_total += 1;
        }
        self.assert_consistent();
        true
    }

    /// Record a terminal outcome for a ticket, evicting the oldest retained
    /// outcome when the bound is reached.
    pub fn record_terminal_ticket(&mut self, ticket: Ticket, reason: TerminalReason) {
        self.tickets.insert(ticket, TicketState::Terminal(reason));
        if let Some(previous) = self.terminal_index.remove(&ticket) {
            self.terminal_order.remove(&previous);
        }
        let seq = self.terminal_seq;
        self.terminal_seq = self.terminal_seq.wrapping_add(1);
        self.terminal_order.insert(seq, ticket);
        self.terminal_index.insert(ticket, seq);
        while self.terminal_order.len() > MAX_TERMINAL_TICKETS {
            let Some((&oldest_seq, &oldest)) = self.terminal_order.iter().next() else {
                break;
            };
            self.terminal_order.remove(&oldest_seq);
            self.terminal_index.remove(&oldest);
            if matches!(self.tickets.get(&oldest), Some(TicketState::Terminal(_))) {
                self.tickets.remove(&oldest);
            }
        }
    }

    /// Drop a retained terminal record once it has been observed. Only a
    /// terminal record is dropped: a ticket that is queued or granted stays.
    pub fn forget_terminal_ticket(&mut self, ticket: Ticket) {
        if let Some(seq) = self.terminal_index.remove(&ticket) {
            self.terminal_order.remove(&seq);
        }
        if matches!(self.tickets.get(&ticket), Some(TicketState::Terminal(_))) {
            self.tickets.remove(&ticket);
        }
    }

    /// Number of terminal records currently retained (bounded by
    /// [`crate::MAX_TERMINAL_TICKETS`]).
    pub fn retained_terminal_tickets(&self) -> usize {
        self.terminal_order.len()
    }

    /// Mark `(root, stage)` as occupied by a child (queued or granted).
    pub fn occupy_recursion_guard(&mut self, root_operation_id: &str, stage: &TaskStage) {
        let stages = match self.active_recursion.get_mut(root_operation_id) {
            Some(stages) => stages,
            None => self
                .active_recursion
                .entry(root_operation_id.to_owned())
                .or_default(),
        };
        stages.insert(stage.clone());
    }

    /// Release `(root, stage)`; the root's entry disappears with its last stage.
    pub fn vacate_recursion_guard(&mut self, root_operation_id: &str, stage: &TaskStage) {
        if let Some(stages) = self.active_recursion.get_mut(root_operation_id) {
            stages.remove(stage);
            if stages.is_empty() {
                self.active_recursion.remove(root_operation_id);
            }
        }
    }

    /// Unwind a permit fully: inflight, phase gauges, global held, capability
    /// occupancy, root accounting, recursion guard, the permit record, and — the
    /// part a bare `permits.remove` forgets — the ticket that granted it.
    ///
    /// Any host `Arc` the record still owns is moved into `effects` so it is
    /// dropped outside the lock.
    pub fn unwind(
        &mut self,
        permit_id: PermitId,
        reason: TerminalReason,
        effects: &mut TransitionEffects,
    ) -> Option<PermitRecord> {
        let mut record = self.permits.remove(&permit_id)?;
        if let Some(state) = self.classes.get_mut(&record.class) {
            state.inflight = state.inflight.saturating_sub(1);
            state.cpu_units_held = state
                .cpu_units_held
                .saturating_sub(u128::from(record.ledger.cpu_units));
            state.memory_units_held = state
                .memory_units_held
                .saturating_sub(u128::from(record.ledger.effective_units));
            state.leave_phase(record.phase);
            state.terminated_total += 1;
        }
        self.cpu_units_held = self
            .cpu_units_held
            .saturating_sub(u128::from(record.ledger.cpu_units));
        self.memory_units_held = self
            .memory_units_held
            .saturating_sub(u128::from(record.ledger.effective_units));
        if let Some(name) = record.capability.as_ref() {
            if let Some(slot) = self.capability_in_use.get_mut(name.as_ref()) {
                *slot = slot.saturating_sub(1);
                if *slot == 0 {
                    self.capability_in_use.remove(name.as_ref());
                }
            }
        }

        if matches!(record.scope, TaskScope::Child { .. }) {
            if let Some(root) = self.roots.get_mut(&record.root_operation_id) {
                root.child_inflight = root.child_inflight.saturating_sub(1);
                root.cpu_units = root
                    .cpu_units
                    .saturating_sub(u128::from(record.ledger.cpu_units));
                root.memory_units = root
                    .memory_units
                    .saturating_sub(u128::from(record.ledger.effective_units));
                root.active_stages.remove(&record.target_stage);
                if root.child_inflight == 0 {
                    self.roots.remove(&record.root_operation_id);
                }
            }
            // Symmetric with grant: only children occupy the recursion guard.
            self.vacate_recursion_guard(&record.root_operation_id, &record.target_stage);
        }

        // The ticket dies with the permit. Leaving the mapping behind is what
        // hands a waiter a permit id that no longer exists; removing it without
        // recording *why* is what parks that waiter forever.
        let pending_ticket = record.pending_ticket.take();
        let claim_waker = record.claim_waker.take();
        match (pending_ticket, claim_waker) {
            // A waiter is still owed an answer: record why, and notify it.
            (Some(ticket), waker) => {
                self.record_terminal_ticket(ticket, reason);
                if let Some(waker) = waker {
                    effects.wake.push(waker);
                }
            }
            // Already claimed: nobody to tell, but the reference still has to be
            // destroyed outside the lock.
            (None, Some(waker)) => effects.retire.push(waker),
            (None, None) => {}
        }

        self.assert_consistent();
        Some(record)
    }

    /// Flag an unrepresentable accounting result. Admission fails closed from
    /// here on; the condition is sticky because a ledger that lost a total
    /// cannot be trusted to have lost only one.
    pub fn record_accounting_fault(&mut self, what: &'static str) {
        if self.accounting_fault.is_none() {
            self.accounting_fault = Some(what);
        }
    }

    /// Debug-only structural invariants. This is the *independent* recomputation
    /// of every incrementally-maintained total, so a drift shows up in tests
    /// rather than in production accounting.
    #[cfg(debug_assertions)]
    pub fn assert_consistent(&self) {
        let class_cpu: u128 = self.classes.values().map(|c| c.cpu_units_held).sum();
        let class_mem: u128 = self.classes.values().map(|c| c.memory_units_held).sum();
        debug_assert_eq!(
            class_cpu, self.cpu_units_held,
            "per-class cpu sum != global"
        );
        debug_assert_eq!(
            class_mem, self.memory_units_held,
            "per-class mem sum != global"
        );

        let mut permit_cpu: u128 = 0;
        let mut permit_mem: u128 = 0;
        let mut inflight_by_class: BTreeMap<&TaskClass, u32> = BTreeMap::new();
        let mut capability_by_pool: BTreeMap<&str, u32> = BTreeMap::new();
        for record in self.permits.values() {
            permit_cpu += u128::from(record.ledger.cpu_units);
            permit_mem += u128::from(record.ledger.effective_units);
            *inflight_by_class.entry(&record.class).or_default() += 1;
            if let Some(name) = record.capability.as_ref() {
                *capability_by_pool.entry(name.as_ref()).or_default() += 1;
            }
        }
        debug_assert_eq!(
            permit_cpu, self.cpu_units_held,
            "permit cpu sum != global held"
        );
        debug_assert_eq!(
            permit_mem, self.memory_units_held,
            "permit mem sum != global held"
        );
        for (class, cstate) in &self.classes {
            let from_permits = inflight_by_class.get(class).copied().unwrap_or(0);
            debug_assert_eq!(
                cstate.inflight, from_permits,
                "class {class} inflight {} != live permit count {from_permits}",
                cstate.inflight
            );
            let phases = cstate.dispatch_reserved
                + cstate.accepted
                + cstate.running
                + cstate.cleanup_pending;
            debug_assert_eq!(
                cstate.inflight, phases,
                "class {class} inflight {} != phase sum {phases}",
                cstate.inflight
            );
            debug_assert_eq!(
                cstate.admitted_total,
                u128::from(cstate.inflight) + cstate.terminated_total,
                "class {class} admitted_total != inflight + terminated_total"
            );
        }
        for (pool, in_use) in &self.capability_in_use {
            let from_permits = capability_by_pool.get(pool.as_ref()).copied().unwrap_or(0);
            debug_assert_eq!(
                *in_use, from_permits,
                "capability {pool} in_use {in_use} != live permit count {from_permits}"
            );
        }
        for (ticket, ticket_state) in &self.tickets {
            if let TicketState::Granted(permit_id) = ticket_state {
                debug_assert!(
                    self.permits.contains_key(permit_id),
                    "granted ticket {ticket} maps to a permit that no longer exists"
                );
            }
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    pub fn assert_consistent(&self) {}
}
