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

use crate::features::memory::MeasurementSequence;
use crate::shared::{
    CapabilityId, CapabilityRequirementSet, PermitId, Provenance, RequestKey, ResolvedCost, Seq,
    Ticket,
};

/// How many terminal ticket outcomes are retained for late claimers.
///
/// Retention is bounded on purpose: an unbounded tombstone map is a leak. Past
/// the bound the oldest outcome is evicted and a late claim reports
/// [`ClaimOutcome::Invalid`] instead — still terminal, still not a hang, just
/// without the specific reason.
pub const MAX_TERMINAL_TICKETS: usize = 4096;

/// A request waiting in a class queue for capacity.
pub struct PendingRequest {
    /// Permit identity reserved at intake. Promotion consumes this exact
    /// identity, so it cannot fail later after the request has entered a queue.
    pub permit_id: PermitId,
    pub ticket: Ticket,
    pub seq_no: Seq,
    pub class: TaskClass,
    /// Admission key derived from the root operation id; audit/diagnostics only.
    pub request_key: RequestKey,
    pub operation: String,
    pub root_operation_id: String,
    pub scope: TaskScope,
    /// Exact live parent generation observed at enqueue. `None` means no parent
    /// was live yet, so promotion may resolve a later declaration by operation.
    pub parent_permit_id: Option<PermitId>,
    pub target_stage: TaskStage,
    /// Classification provenance (source/reason), preserved through promotion.
    pub provenance: Provenance,
    pub cost: ResolvedCost,
    /// The capability pool this request must occupy to run. Frozen at intake so
    /// promotion can never re-resolve it to a cheaper pool.
    pub capabilities: CapabilityRequirementSet,
    pub enqueued_at_ms: u64,
    /// Absolute deadline used by `DeadlineAware`; `u64::MAX` when unset.
    pub deadline_ms: u64,
    /// WFQ virtual finish tag assigned at enqueue.
    pub finish_tag: u128,
    /// Global virtual time observed at enqueue, before same-class queued debt.
    /// This lets cancellation rebuild survivor tags without erasing progress
    /// made by other classes between arrivals.
    pub wfq_arrival_tag: u128,
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
    /// WFQ: virtual finish after the service actually received in this busy
    /// period. This excludes skipped queued work when a request overtakes a
    /// capability-blocked head. Queue rebuilds start from this baseline.
    pub last_served_finish_tag: u128,
    /// DRR: accumulated deficit, in the same unit domain as head cost.
    pub deficit: u128,
    /// Work oracle for WFQ queue repricing. Ordinary head service must not
    /// traverse the remaining queue; only out-of-order service or cancellation
    /// can require a survivor rebuild.
    #[cfg(feature = "test-util")]
    pub wfq_reprice_visits: u64,
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
    pub measurement_sequence: MeasurementSequence,
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
    pub operation: String,
    pub root_operation_id: String,
    pub scope: TaskScope,
    /// Exact parent generation observed when this permit was granted. Operation
    /// text can be reused after release, so ancestry must not be reconstructed
    /// from the current operation index.
    pub parent_permit_id: Option<PermitId>,
    pub target_stage: TaskStage,
    /// Classification provenance (source/reason), auditable post-intake.
    pub provenance: Provenance,
    /// The capability pool this permit occupies while live.
    pub capabilities: CapabilityRequirementSet,
    /// Current ownership phase. Advances monotonically.
    pub phase: ExecutionPhase,
    /// The custody proof minted when the permit left `DispatchReserved`, or
    /// `None` while it has not. Once set, only a [`LeaseToken`] carrying this
    /// value ends the permit (see [`ReleaseOutcome::HeldByLease`]).
    pub lease_nonce: Option<u64>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// Engine-owned reason a queued acquisition ended without custody transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalReason {
    Reclaimed,
    Released,
    Abandoned,
    /// The promoted permit could not be handed to its waiter because retiring
    /// the final host notifier panicked. The engine compensated every charge
    /// before publishing this outcome.
    ClaimDeliveryFailed,
    IrreversibleWaitCycle {
        held_by_parent: taskmesh_contract::HeldCapacity,
    },
}

impl From<TerminalReason> for taskmesh_contract::TerminalReason {
    fn from(reason: TerminalReason) -> Self {
        match reason {
            TerminalReason::Reclaimed => Self::Reclaimed,
            TerminalReason::Released => Self::Released,
            TerminalReason::Abandoned => Self::Abandoned,
            TerminalReason::ClaimDeliveryFailed => Self::ClaimDeliveryFailed,
            TerminalReason::IrreversibleWaitCycle { held_by_parent } => {
                Self::IrreversibleWaitCycle { held_by_parent }
            }
        }
    }
}

/// Lifecycle position of a ticket. Exactly one variant holds at any time.
#[derive(Debug, Clone)]
pub enum TicketState {
    Queued {
        class: TaskClass,
    },
    Granted(PermitId),
    /// Claim has started retiring the final notifier outside the mutex. The
    /// permit is not caller-owned until this state is finalized.
    Claiming(PermitId),
    Terminal(TerminalReason),
}

/// The result of claiming a ticket. `Pending` and `Terminal` are *different*
/// answers: conflating them (as a bare `None` does) is what turns a reclaimed
/// permit into a waiter that parks forever.
#[derive(Debug, Clone, PartialEq, Eq)]
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

/// Result of abandoning a queued or promoted ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an invalid ticket means the requested abandon did not occur; handle the outcome"]
pub enum AbandonOutcome {
    /// A queued request was removed or its promoted permit was released.
    Abandoned,
    /// A retained terminal result was explicitly discarded.
    TerminalDiscarded,
    /// The ticket is unknown to this governor. No state or callback changed.
    Invalid,
}

/// The result of releasing a permit.
///
/// A release that finds no permit is one of two things: a double release
/// (host bug) or a release of a lease the leak sweep already reclaimed (a real
/// race the host must be able to notice). Returning `()` for both is how either
/// stays invisible until the accounting drifts.
///
/// A release that finds a permit it is not entitled to end is a third thing —
/// a caller reaching for capacity that a dispatched execution still owns — and
/// it changes nothing (`HeldByLease`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an unknown permit on release is a double release or a reclaimed lease, and a held one was not released; handle it"]
pub enum ReleaseOutcome {
    /// The permit was live; its capacity, capability slot, and attribution are
    /// returned and queued work has been promoted.
    Released,
    /// No live permit had this id: already released, reclaimed, abandoned, or
    /// never granted by this governor.
    UnknownPermit,
    /// A live permit has this id and the caller's proof of custody is not the
    /// permit's, so nothing was released: either a plain
    /// [`Governor::release`](crate::Governor::release) of a permit that has
    /// left `DispatchReserved` — its [`LeaseToken`] owns it now — or a
    /// [`Governor::release_leased`](crate::Governor::release_leased) with a
    /// token that is not this permit's lease (one minted by another governor
    /// for the same id). `phase` is the permit's current phase; it reads
    /// `DispatchReserved` only in the second case, for a permit that has no
    /// lease at all and that `release` would end.
    HeldByLease { phase: ExecutionPhase },
}

/// Proof of custody over a dispatched permit.
///
/// Minted exactly once per permit, by the [`Governor::advance_phase`]
/// transition that moves it out of `DispatchReserved`, and handed to the
/// caller that declared the phase. From that moment the permit is owned by
/// whoever holds this value: [`Governor::release`] answers
/// [`ReleaseOutcome::HeldByLease`] and changes nothing, and only
/// [`Governor::release_leased`] with this token ends the permit. The leak sweep
/// never reclaims a leased permit either, so a token that is dropped without
/// being spent leaves its capacity charged for good — hence `#[must_use]`.
///
/// The token is not `Clone` or `Copy`: one execution, one proof, spent once.
/// Its nonce is drawn from a process-wide counter, independently of the
/// governor authority already embedded in each permit handle. It is a guard
/// against accidental misuse of custody proofs,
/// not a security boundary: an embedder that reaches for engine internals is
/// trusted by construction, and nothing here is meant to stop a hostile one.
///
/// [`Governor::advance_phase`]: crate::Governor::advance_phase
/// [`Governor::release`]: crate::Governor::release
/// [`Governor::release_leased`]: crate::Governor::release_leased
#[derive(Debug, PartialEq, Eq, Hash)]
#[must_use = "a lease token that is dropped unspent leaves its permit charged forever: nothing else can release a dispatched permit"]
pub struct LeaseToken {
    pub(crate) permit_id: PermitId,
    pub(crate) nonce: u64,
}

impl LeaseToken {
    /// The permit this token is the lease of.
    pub fn permit_id(&self) -> PermitId {
        self.permit_id
    }

    /// Construct a token with an explicit nonce.
    ///
    /// Test-only: production code obtains tokens from
    /// [`Governor::advance_phase`](crate::Governor::advance_phase) and nowhere
    /// else. This exists so a test can present a *wrong* proof (a forged nonce,
    /// or a twin of a token already spent) and check that the engine refuses it.
    #[cfg(feature = "test-util")]
    #[doc(hidden)]
    pub fn forge(permit_id: PermitId, nonce: u64) -> Self {
        Self { permit_id, nonce }
    }

    /// The nonce this token carries. Test-only, for forging a twin or a
    /// near-miss of a real token.
    #[cfg(feature = "test-util")]
    #[doc(hidden)]
    pub fn nonce(&self) -> u64 {
        self.nonce
    }
}

/// The result of declaring an ownership phase for a live permit.
///
/// The first successful move out of `DispatchReserved` is where custody
/// changes hands: it mints the permit's [`LeaseToken`] and gives it to the
/// caller, who must keep it — nothing else releases the permit from then on.
/// Later moves along the same lease only advance the gauges.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "Leased carries the only proof that can release the permit, and Refused means it did not advance; handle every arm"]
pub enum AdvanceOutcome {
    /// The permit left `DispatchReserved`. The token is its lease.
    Leased(LeaseToken),
    /// The permit moved to a later phase under the lease minted earlier.
    Advanced,
    /// Nothing changed.
    Refused(AdvanceRefusal),
}

/// Why a phase declaration did not advance a permit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvanceRefusal {
    /// No live permit had this id: released, reclaimed, abandoned, or never
    /// granted by this governor.
    UnknownPermit,
    /// The permit is live but the declared phase is not later than its
    /// current one. Phases are monotonic; a backwards or repeated declaration
    /// is a caller ordering error and is reported rather than absorbed.
    NotLater { current: ExecutionPhase },
}

/// Source of lease nonces: one process-wide counter, so every token minted
/// in this process — by any governor — carries a distinct value.
///
/// Deliberately a `std` atomic outside the `sync` seam: the model checkers
/// swap in their own primitives for the state that transitions *synchronize*
/// on, and this counter is not that. Nothing reads it back but the transition
/// that drew it; its only property is uniqueness, which a relaxed `fetch_add`
/// provides under every memory model.
static LEASE_NONCES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn mint_lease_nonce() -> u64 {
    LEASE_NONCES.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
    pub operation: &'a str,
    pub root_operation_id: &'a str,
    pub scope: TaskScope,
    pub parent_permit_id: Option<PermitId>,
    pub target_stage: TaskStage,
    pub provenance: Provenance,
    pub capabilities: CapabilityRequirementSet,
    pub cost: ResolvedCost,
    pub reserved_units: u32,
    pub now_ms: u64,
    pub pending_ticket: Option<Ticket>,
    pub claim_waker: Option<Arc<dyn PermitWaker>>,
}

/// A live operation identity has at most one permit. Admission rejects a second
/// `(root, operation)` before grant, so a set of permit IDs would only allocate
/// an extra tree node for every grant. The root operation is stored inline;
/// descendants need a keyed lookup for exact immediate-parent resolution.
#[derive(Default)]
struct RootOperationPermits {
    root: Option<PermitId>,
    descendants: BTreeMap<String, PermitId>,
}

impl RootOperationPermits {
    fn get(&self, root_operation_id: &str, operation: &str) -> Option<PermitId> {
        if operation == root_operation_id {
            self.root
        } else {
            self.descendants.get(operation).copied()
        }
    }

    fn insert(&mut self, root_operation_id: &str, operation: &str, permit_id: PermitId) {
        let previous = if operation == root_operation_id {
            self.root.replace(permit_id)
        } else {
            self.descendants.insert(operation.to_owned(), permit_id)
        };
        debug_assert!(previous.is_none(), "duplicate live operation identity");
    }

    fn remove(&mut self, root_operation_id: &str, operation: &str) -> Option<PermitId> {
        if operation == root_operation_id {
            self.root.take()
        } else {
            self.descendants.remove(operation)
        }
    }

    fn is_empty(&self) -> bool {
        self.root.is_none() && self.descendants.is_empty()
    }
}

/// The full governed state. One instance lives behind the governor's mutex.
#[derive(Default)]
pub struct GovernedState {
    pub classes: BTreeMap<TaskClass, ClassState>,
    pub permits: BTreeMap<PermitId, PermitRecord>,
    operation_permits: BTreeMap<String, RootOperationPermits>,
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
    pub capability_in_use: BTreeMap<CapabilityId, u32>,
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
    /// Set once by [`Self::close_admission`] and never cleared: the runtime is
    /// draining. While set, admission is refused before anything is charged;
    /// queued work is still promoted, claimed, and run, and every release is
    /// still accounted. Checked under the same lock that admits, so the
    /// snapshot taken after the close is the last word on what was let in.
    pub admission_closed: bool,
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

    pub fn capability_in_use(&self, capability: &CapabilityId) -> u32 {
        self.capability_in_use.get(capability).copied().unwrap_or(0)
    }

    pub fn permits_for_operation(
        &self,
        root_operation_id: &str,
        operation: &str,
    ) -> Option<PermitId> {
        self.operation_permits
            .get(root_operation_id)?
            .get(root_operation_id, operation)
    }

    /// Resolve the currently live parent generation named by a child scope.
    /// Callers that persist lineage must store the returned permit ID rather
    /// than resolving the operation text again after it can be reused.
    pub fn parent_permit_for_scope(
        &self,
        root_operation_id: &str,
        scope: &TaskScope,
    ) -> Option<PermitId> {
        let TaskScope::Child {
            parent_operation_id,
            ..
        } = scope
        else {
            return None;
        };
        self.permits_for_operation(root_operation_id, parent_operation_id)
    }

    pub fn has_operation_identity(&self, root_operation_id: &str, operation: &str) -> bool {
        self.permits_for_operation(root_operation_id, operation)
            .is_some()
            || self.classes.values().any(|class| {
                class.queue.iter().any(|request| {
                    request.root_operation_id == root_operation_id && request.operation == operation
                })
            })
    }

    /// Commit a granted permit: bump inflight, global held, capability
    /// occupancy, root accounting, and the recursion guard.
    pub fn grant(&mut self, request: GrantRequest<'_>) {
        let GrantRequest {
            permit_id,
            class,
            operation,
            root_operation_id,
            scope,
            parent_permit_id,
            target_stage,
            provenance,
            capabilities,
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
        for id in capabilities.iter() {
            let slot = self.capability_in_use.entry(id.clone()).or_insert(0);
            *slot = slot.saturating_add(1);
        }
        if let Some(operations) = self.operation_permits.get_mut(root_operation_id) {
            operations.insert(root_operation_id, operation, permit_id);
        } else {
            let mut operations = RootOperationPermits::default();
            operations.insert(root_operation_id, operation, permit_id);
            self.operation_permits
                .insert(root_operation_id.to_owned(), operations);
        }

        // Recursion guard and root attribution are CHILD-only. A root must never
        // occupy `(root, stage)` itself, else its own first child — which targets
        // the parent stage — would be (wrongly) rejected as recursive.
        if matches!(scope, TaskScope::Child { .. }) {
            let root = match self.roots.get_mut(root_operation_id) {
                Some(root) => root,
                None => self.roots.entry(root_operation_id.to_owned()).or_default(),
            };
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
                operation: operation.to_owned(),
                root_operation_id: root_operation_id.to_owned(),
                scope,
                parent_permit_id,
                target_stage,
                provenance,
                capabilities,
                phase: ExecutionPhase::DispatchReserved,
                lease_nonce: None,
                pending_ticket,
                claim_waker,
                ledger: PermitLedger {
                    cpu_units: cost.cpu_units,
                    original_estimate_units: reserved_units,
                    remaining_reservation_units: reserved_units,
                    measured_bytes: 0,
                    measurement_sequence: MeasurementSequence::INITIAL,
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
    /// to an earlier phase, and re-declaring the current phase is refused.
    ///
    /// The move out of `DispatchReserved` is the one that mints the permit's
    /// [`LeaseToken`]: the nonce is recorded in the ledger here, in the same
    /// transition that changes the phase, so there is no instant at which the
    /// permit is dispatched but unowned.
    pub fn advance_phase(&mut self, permit_id: PermitId, phase: ExecutionPhase) -> AdvanceOutcome {
        let Some(record) = self.permits.get_mut(&permit_id) else {
            return AdvanceOutcome::Refused(AdvanceRefusal::UnknownPermit);
        };
        if phase.rank() <= record.phase.rank() {
            return AdvanceOutcome::Refused(AdvanceRefusal::NotLater {
                current: record.phase,
            });
        }
        let previous = record.phase;
        record.phase = phase;
        let outcome = if record.lease_nonce.is_none() {
            let nonce = mint_lease_nonce();
            record.lease_nonce = Some(nonce);
            AdvanceOutcome::Leased(LeaseToken { permit_id, nonce })
        } else {
            AdvanceOutcome::Advanced
        };
        let class = record.class.clone();
        let newly_started = !previous.has_started() && phase.has_started();
        let cstate = self.class_mut(&class);
        cstate.leave_phase(previous);
        cstate.enter_phase(phase);
        if newly_started {
            cstate.started_total += 1;
        }
        self.assert_consistent();
        outcome
    }

    /// Unwind a permit if — and only if — `proof` is its custody: `None` for a
    /// permit that has not left `DispatchReserved`, or its minted lease nonce
    /// once it has. A permit is never ended by a caller that does not hold
    /// exactly what the ledger says its owner holds.
    pub fn release_with_proof(
        &mut self,
        permit_id: PermitId,
        proof: Option<u64>,
        effects: &mut TransitionEffects,
    ) -> ReleaseOutcome {
        let Some(record) = self.permits.get(&permit_id) else {
            return ReleaseOutcome::UnknownPermit;
        };
        if record.lease_nonce != proof {
            return ReleaseOutcome::HeldByLease {
                phase: record.phase,
            };
        }
        match self.unwind(permit_id, TerminalReason::Released, effects) {
            Some(_) => ReleaseOutcome::Released,
            // The record was found a moment ago under the same lock.
            None => ReleaseOutcome::UnknownPermit,
        }
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
        for id in record.capabilities.iter() {
            if let Some(slot) = self.capability_in_use.get_mut(id) {
                *slot = slot.saturating_sub(1);
                if *slot == 0 {
                    self.capability_in_use.remove(id);
                }
            }
        }
        if let Some(operations) = self.operation_permits.get_mut(&record.root_operation_id) {
            let removed = operations.remove(&record.root_operation_id, &record.operation);
            debug_assert_eq!(
                removed,
                Some(permit_id),
                "permit missing from exact operation identity index"
            );
            if operations.is_empty() {
                self.operation_permits.remove(&record.root_operation_id);
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

    /// Stop admitting. One-way, idempotent; nothing already in the state is
    /// touched (D17).
    pub fn close_admission(&mut self) {
        self.admission_closed = true;
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
        let mut capability_by_pool: BTreeMap<&CapabilityId, u32> = BTreeMap::new();
        for record in self.permits.values() {
            permit_cpu += u128::from(record.ledger.cpu_units);
            permit_mem += u128::from(record.ledger.effective_units);
            *inflight_by_class.entry(&record.class).or_default() += 1;
            for id in record.capabilities.iter() {
                *capability_by_pool.entry(id).or_default() += 1;
            }
            debug_assert_eq!(
                record.lease_nonce.is_some(),
                record.phase != ExecutionPhase::DispatchReserved,
                "permit {} is {} with lease nonce {:?}: a permit is leased exactly when it has left DispatchReserved",
                record.permit_id,
                record.phase,
                record.lease_nonce
            );
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
            let from_permits = capability_by_pool.get(pool).copied().unwrap_or(0);
            debug_assert_eq!(
                *in_use, from_permits,
                "capability {pool} in_use {in_use} != live permit count {from_permits}"
            );
        }
        for (ticket, ticket_state) in &self.tickets {
            if let TicketState::Granted(permit_id) | TicketState::Claiming(permit_id) = ticket_state
            {
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

#[cfg(test)]
mod mutation_semantics {
    use super::*;

    #[test]
    fn operation_index_distinguishes_root_descendant_and_absence() {
        let mut operations = RootOperationPermits::default();
        assert!(operations.is_empty());
        let root = PermitId::new(1, 11);
        let child = PermitId::new(1, 12);
        operations.insert("root", "root", root);
        operations.insert("root", "child", child);
        assert!(!operations.is_empty());
        assert_eq!(operations.get("root", "root"), Some(root));
        assert_eq!(operations.get("root", "child"), Some(child));
        assert_eq!(operations.get("root", "missing"), None);
        assert_eq!(operations.remove("root", "root"), Some(root));
        assert!(!operations.is_empty());
        assert_eq!(operations.remove("root", "child"), Some(child));
        assert!(operations.is_empty());
    }

    #[test]
    fn governed_state_operation_identity_uses_exact_root_and_operation() {
        let mut state = GovernedState::default();
        let mut operations = RootOperationPermits::default();
        let permit_id = PermitId::new(1, 7);
        operations.insert("root", "child", permit_id);
        state
            .operation_permits
            .insert("root".to_owned(), operations);

        assert!(state.has_operation_identity("root", "child"));
        assert!(!state.has_operation_identity("root", "other"));
        assert!(!state.has_operation_identity("other", "child"));

        // Exercise the queued identity branch independently of the permit
        // index; otherwise the first positive assertion short-circuits before
        // evaluating its exact `(root, operation)` predicate.
        let class = TaskClass::new("queued");
        let spec = taskmesh_contract::TaskSpec::io(class.clone()).operation("queue-root");
        state.class_mut(&class).queue.push_back(PendingRequest {
            permit_id: PermitId::new(1, 8),
            ticket: Ticket::new(1, 1),
            seq_no: 1,
            class,
            request_key: RequestKey::from_root("queue-root"),
            operation: "queue-child".to_owned(),
            root_operation_id: "queue-root".to_owned(),
            scope: TaskScope::Root,
            parent_permit_id: None,
            target_stage: TaskStage::new("io"),
            provenance: Provenance::of(&spec),
            cost: ResolvedCost::default(),
            capabilities: CapabilityRequirementSet::empty(),
            enqueued_at_ms: 0,
            deadline_ms: u64::MAX,
            finish_tag: 0,
            wfq_arrival_tag: 0,
            blocked_on: CapacityBlock::Cpu,
            waker: None,
        });
        assert!(state.has_operation_identity("queue-root", "queue-child"));
        assert!(!state.has_operation_identity("queue-root", "other"));
        assert!(!state.has_operation_identity("other", "queue-child"));
    }

    #[test]
    fn accounting_fault_is_first_failure_sticky() {
        let mut state = GovernedState::default();
        state.record_accounting_fault("overflow");
        state.record_accounting_fault("later");
        assert_eq!(state.accounting_fault, Some("overflow"));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "per-class cpu sum != global")]
    fn consistency_oracle_detects_independent_ledger_drift() {
        let state = GovernedState {
            cpu_units_held: 1,
            ..GovernedState::default()
        };
        state.assert_consistent();
    }
}
