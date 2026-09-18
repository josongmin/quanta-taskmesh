//! Random transition sequences against the **production** `Governor`.
//!
//! `tests/differential_model.rs` checks the admission/promotion state machine
//! against an executable specification on proptest-generated sequences; this
//! target drives the same engine with libFuzzer's coverage feedback instead,
//! over every public transition (admit / claim / abandon / release / lease
//! release / phase advance / stage memory release / reconcile / leak sweep /
//! close), and checks the published invariants after each one:
//!
//! * nothing panics (the engine's own `debug_assert` consistency checks are
//!   armed — see `Cargo.toml`);
//! * `Snapshot::conservation_violation()` is `None` and no accounting fault is
//!   recorded;
//! * no class exceeds its `max_inflight` / `max_queue_depth`, no pool exceeds
//!   its limit, held cpu never exceeds the budget, and held memory never
//!   exceeds it *by admission* — an upward measured reconcile records reality
//!   and may push held memory over the budget, after which a class with a
//!   memory cost cannot be admitted (queued, shed, or degraded to a free
//!   fallback) until units are returned;
//! * every outcome the engine reports about a handle the harness holds agrees
//!   with what the harness did with it (a permit it released is
//!   `UnknownPermit` afterwards; a token spent twice is refused);
//! * after everything the harness holds is returned, the governor is quiescent:
//!   every gauge zero, `admitted_total == terminated_total`, no ledger left.
#![no_main]

use std::collections::BTreeMap;
use std::sync::Arc;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ExecutionPhase, FairnessPolicy, ManualClock,
    MemoryOvercommitPolicy, MemoryPermitMode, MemoryReleasePolicy, OverflowPolicy, ResourceBudget,
    TaskClass, TaskSpec, TaskStage,
};
use taskmesh_engine::{
    AdmissionDecision, AdvanceOutcome, ClaimOutcome, Governor, LeaseToken, PermitId, PolicySet,
    ReleaseOutcome, Ticket,
};

const CLASSES: [&str; 3] = ["alpha", "beta", "gamma"];

#[derive(Arbitrary, Debug)]
struct ClassShape {
    max_inflight: u8,
    max_queue_depth: u8,
    cpu_units: u8,
    memory_units: u8,
    overflow: u8,
    overcommit: u8,
    release: u8,
    permit_mode: u8,
}

#[derive(Arbitrary, Debug, Clone, Copy)]
enum Op {
    Admit {
        class: u8,
        substrate: u8,
        child_of: Option<u8>,
    },
    Claim(u8),
    Abandon(u8),
    Release(u8),
    Advance {
        permit: u8,
        phase: u8,
    },
    ReleaseLeased(u8),
    StageRelease {
        permit: u8,
        units: u8,
    },
    Reconcile {
        permit: u8,
        bytes: u16,
        epoch: u8,
    },
    Clock(u16),
    Reap(u16),
    Close,
}

#[derive(Arbitrary, Debug)]
struct Scenario {
    classes: [ClassShape; 3],
    fairness: u8,
    budget_cpu: u16,
    budget_memory: u16,
    blocking_limit: u8,
    ops: Vec<Op>,
}

/// One fairness tier for every class: the policy validator refuses mixed
/// tiers, and a scenario the governor refuses to build teaches nothing.
fn fairness(selector: u8, index: usize) -> FairnessPolicy {
    match selector % 4 {
        0 => FairnessPolicy::Fifo,
        1 => FairnessPolicy::DeficitRoundRobin {
            quantum: 1 + u32::from(selector / 4) + index as u32,
        },
        2 => FairnessPolicy::WeightedFairQueue {
            weight: 1 + u32::from(selector / 4) * 7 + index as u32,
            burst: 0,
        },
        _ => FairnessPolicy::BestEffortScavenger,
    }
}

fn class_policy(shape: &ClassShape, fairness: FairnessPolicy, index: usize) -> ClassPolicy {
    let overflow = match shape.overflow % 3 {
        0 => OverflowPolicy::Reject,
        1 => OverflowPolicy::QueueWithinDepth,
        _ => OverflowPolicy::DropBestEffort,
    };
    let overcommit = match shape.overcommit % 3 {
        0 => MemoryOvercommitPolicy::Reject,
        1 => MemoryOvercommitPolicy::Queue,
        _ => MemoryOvercommitPolicy::DegradeToLight {
            fallback_class: TaskClass::new(CLASSES[(index + 1) % CLASSES.len()]),
        },
    };
    let release = match shape.release % 3 {
        0 => MemoryReleasePolicy::OnTaskCompletion,
        1 => MemoryReleasePolicy::OnStageBoundary,
        _ => MemoryReleasePolicy::LeakDetecting,
    };
    let permit_mode = match shape.permit_mode % 3 {
        0 => MemoryPermitMode::Estimated,
        1 => MemoryPermitMode::Measured,
        _ => MemoryPermitMode::Hybrid,
    };
    ClassPolicy::new()
        .max_inflight(u32::from(shape.max_inflight % 6))
        .max_queue_depth(u32::from(shape.max_queue_depth % 6))
        .cpu_units(u32::from(shape.cpu_units % 4))
        .memory_units(u32::from(shape.memory_units % 4))
        .fairness(fairness)
        .overflow_policy(overflow)
        .memory_overcommit_policy(overcommit)
        .memory_release_policy(release)
        .memory_permit_mode(permit_mode)
}

fn spec(class: u8, substrate: u8, child_of: Option<u8>, seq: usize) -> TaskSpec {
    let class = TaskClass::new(CLASSES[usize::from(class) % CLASSES.len()]);
    let spec = match substrate % 4 {
        0 => TaskSpec::io(class),
        1 => TaskSpec::blocking(class),
        2 => TaskSpec::cpu(class),
        _ => TaskSpec::local(class),
    }
    .operation(format!("op{seq}"));
    match child_of {
        Some(root) => spec.child_of(format!("root{}", root % 4), TaskStage::new("parent")),
        None => spec,
    }
}

struct Harness {
    governor: Governor,
    clock: Arc<ManualClock>,
    limits: BTreeMap<TaskClass, (u32, u32)>,
    /// Per class: memory cost, and the fallback class if it degrades.
    memory_cost: BTreeMap<TaskClass, (u32, Option<TaskClass>)>,
    budget: (u128, u128),
    /// An upward measured reconcile was applied: held memory may now exceed
    /// the budget, which bounds admission, not reality.
    reconciled_upward: bool,
    blocking_limit: u32,
    /// Tickets the harness still owns (queued, or promoted but unclaimed).
    tickets: Vec<Ticket>,
    /// Permits the harness holds by id (undispatched).
    permits: Vec<PermitId>,
    /// Leases the harness holds (dispatched).
    tokens: Vec<LeaseToken>,
    /// Permits the harness has ended itself: the engine must never know them
    /// again.
    ended: Vec<PermitId>,
    closed: bool,
}

impl Harness {
    fn check(&self) {
        let snapshot = self.governor.snapshot();
        assert_eq!(snapshot.conservation_violation(), None);
        assert_eq!(self.governor.accounting_fault(), None);
        let mut cpu: u128 = 0;
        let mut memory: u128 = 0;
        for (class, observed) in &snapshot.classes {
            // `max_inflight == 0` disables the class, so the bound holds
            // unconditionally: a disabled class admits nothing.
            let (max_inflight, max_queue_depth) = self.limits[class];
            assert!(
                observed.inflight <= max_inflight,
                "class {class}: inflight {} over max_inflight {max_inflight}",
                observed.inflight
            );
            assert!(
                observed.queued <= max_queue_depth,
                "class {class}: queued {} over max_queue_depth {max_queue_depth}",
                observed.queued
            );
            cpu += observed.cpu_units_held;
            memory += observed.memory_units_held;
        }
        // A budget of 0 is "unlimited" (the published convention), not "none".
        if self.budget.0 != 0 {
            assert!(
                cpu <= self.budget.0,
                "cpu held {cpu} over budget {}",
                self.budget.0
            );
        }
        if self.budget.1 != 0 && !self.reconciled_upward {
            assert!(
                memory <= self.budget.1,
                "memory held {memory} over budget {}",
                self.budget.1
            );
        }
        if let Some(pool) = snapshot.capabilities.get("blocking") {
            if self.blocking_limit != 0 {
                assert!(pool.in_use <= self.blocking_limit);
            }
        }
        for permit in &self.ended {
            assert!(
                self.governor.permit_ledger(*permit).is_none(),
                "permit {permit} ended by the harness is still in the ledger"
            );
        }
        if self.closed {
            assert!(self.governor.admission_closed(), "close is one-way");
        }
    }

    fn step(&mut self, op: Op, seq: usize) {
        match op {
            Op::Admit {
                class,
                substrate,
                child_of,
            } => {
                let spec = spec(class, substrate, child_of, seq);
                let held_before = self
                    .governor
                    .snapshot()
                    .classes
                    .values()
                    .map(|c| c.memory_units_held)
                    .sum::<u128>();
                match self.governor.admit(&spec) {
                    AdmissionDecision::Admitted { permit_id } => {
                        assert!(
                            !self.closed,
                            "permit {permit_id} admitted after close_admission"
                        );
                        if self.budget.1 != 0 && held_before >= self.budget.1 {
                            // The budget is spent: only free work may enter — the
                            // class itself costs nothing, or it degrades to a
                            // fallback that costs nothing.
                            let (cost, fallback) = &self.memory_cost[&spec.class];
                            let fallback_cost =
                                fallback.as_ref().map(|class| self.memory_cost[class].0);
                            assert!(
                                *cost == 0 || fallback_cost == Some(0),
                                "permit {permit_id} admitted for class {} (memory cost {cost}) \
                                 with held {held_before} >= budget {}",
                                spec.class,
                                self.budget.1
                            );
                        }
                        self.permits.push(permit_id);
                    }
                    AdmissionDecision::Queued { ticket } => {
                        assert!(!self.closed, "ticket {ticket} queued after close_admission");
                        self.tickets.push(ticket);
                    }
                    AdmissionDecision::Rejected(verdict) => {
                        if self.closed {
                            assert_eq!(verdict, AdmissionVerdict::RuntimeUnavailable);
                        } else {
                            assert_ne!(
                                verdict,
                                AdmissionVerdict::RuntimeUnavailable,
                                "RuntimeUnavailable without a close or a fault"
                            );
                        }
                        // Every verdict renders and answers the hint question.
                        let _ = (verdict.to_string(), verdict.retry_after_ms());
                    }
                }
            }
            Op::Claim(index) => {
                if self.tickets.is_empty() {
                    return;
                }
                let at = usize::from(index) % self.tickets.len();
                let ticket = self.tickets[at];
                match self.governor.claim(ticket) {
                    ClaimOutcome::Ready(permit_id) => {
                        self.tickets.swap_remove(at);
                        self.permits.push(permit_id);
                    }
                    ClaimOutcome::Pending => {}
                    ClaimOutcome::Terminal(_) => {
                        self.tickets.swap_remove(at);
                    }
                    ClaimOutcome::Invalid => {
                        panic!("ticket {ticket} the harness owns is unknown to the engine")
                    }
                }
            }
            Op::Abandon(index) => {
                if self.tickets.is_empty() {
                    return;
                }
                let at = usize::from(index) % self.tickets.len();
                let ticket = self.tickets.swap_remove(at);
                self.governor.abandon(ticket);
                assert!(
                    matches!(
                        self.governor.ticket_status(ticket),
                        ClaimOutcome::Invalid | ClaimOutcome::Terminal(_)
                    ),
                    "ticket {ticket} still claimable after abandon"
                );
            }
            Op::Release(index) => {
                if self.permits.is_empty() {
                    return;
                }
                let at = usize::from(index) % self.permits.len();
                let permit = self.permits.swap_remove(at);
                match self.governor.release(permit) {
                    // Released here, or already reclaimed by the sweep (a stale
                    // LeakDetecting reservation): either way it is gone.
                    ReleaseOutcome::Released | ReleaseOutcome::UnknownPermit => {
                        self.ended.push(permit);
                    }
                    ReleaseOutcome::HeldByLease { .. } => {
                        panic!("permit {permit} the harness never advanced reads as leased")
                    }
                }
            }
            Op::Advance { permit, phase } => {
                if self.permits.is_empty() {
                    return;
                }
                let at = usize::from(permit) % self.permits.len();
                let permit = self.permits[at];
                let phase = match phase % 3 {
                    0 => ExecutionPhase::Accepted,
                    1 => ExecutionPhase::Running,
                    _ => ExecutionPhase::CleanupPending,
                };
                match self.governor.advance_phase(permit, phase) {
                    AdvanceOutcome::Leased(token) => {
                        assert_eq!(token.permit_id(), permit);
                        self.permits.swap_remove(at);
                        self.tokens.push(token);
                    }
                    AdvanceOutcome::Advanced => {
                        panic!("permit {permit} advanced without a lease the harness holds")
                    }
                    AdvanceOutcome::Refused(_) => {
                        // Reclaimed by the sweep before dispatch: gone for good.
                        if self.governor.permit_ledger(permit).is_none() {
                            self.permits.swap_remove(at);
                            self.ended.push(permit);
                        }
                    }
                }
            }
            Op::ReleaseLeased(index) => {
                if self.tokens.is_empty() {
                    return;
                }
                let at = usize::from(index) % self.tokens.len();
                let token = self.tokens.swap_remove(at);
                let permit = token.permit_id();
                assert_eq!(
                    self.governor.release_leased(token),
                    ReleaseOutcome::Released,
                    "the lease the harness holds must release its permit"
                );
                self.ended.push(permit);
            }
            Op::StageRelease { permit, units } => {
                let Some(permit) = self.any_live(permit) else {
                    return;
                };
                let _ = self.governor.release_stage_memory(permit, u32::from(units));
            }
            Op::Reconcile {
                permit,
                bytes,
                epoch,
            } => {
                let Some(permit) = self.any_live(permit) else {
                    return;
                };
                let before = self
                    .governor
                    .permit_ledger(permit)
                    .map(|ledger| ledger.effective_units);
                let outcome =
                    self.governor
                        .reconcile_memory_at(permit, u64::from(bytes), u64::from(epoch));
                if let (taskmesh_engine::ReconcileOutcome::Applied { held_units }, Some(before)) =
                    (outcome, before)
                {
                    if held_units > before {
                        self.reconciled_upward = true;
                    }
                }
            }
            Op::Clock(ms) => self.clock.advance(u64::from(ms)),
            Op::Reap(stale_ms) => {
                let report = self.governor.reap_leaks_with(u64::from(stale_ms));
                assert!(report.reclaimed_permits <= report.suspected_leaks);
                // Reclaimed permits were undispatched: the harness may still
                // hold their ids, and finds out on its next use of them.
            }
            Op::Close => {
                self.governor.close_admission();
                self.closed = true;
            }
        }
    }

    /// A permit the harness holds in either form, for the transitions that
    /// take an id whatever the custody (stage release, reconcile).
    fn any_live(&self, selector: u8) -> Option<PermitId> {
        let total = self.permits.len() + self.tokens.len();
        if total == 0 {
            return None;
        }
        let at = usize::from(selector) % total;
        Some(if at < self.permits.len() {
            self.permits[at]
        } else {
            self.tokens[at - self.permits.len()].permit_id()
        })
    }

    fn finish(mut self) {
        for ticket in std::mem::take(&mut self.tickets) {
            self.governor.abandon(ticket);
        }
        for permit in std::mem::take(&mut self.permits) {
            match self.governor.release(permit) {
                ReleaseOutcome::Released | ReleaseOutcome::UnknownPermit => {}
                ReleaseOutcome::HeldByLease { .. } => {
                    panic!("permit {permit} the harness never advanced reads as leased")
                }
            }
        }
        for token in std::mem::take(&mut self.tokens) {
            assert_eq!(
                self.governor.release_leased(token),
                ReleaseOutcome::Released
            );
        }
        let snapshot = self.governor.snapshot();
        for (class, observed) in &snapshot.classes {
            assert_eq!(
                (observed.inflight, observed.queued),
                (0, 0),
                "class {class}: not quiescent after everything was returned"
            );
            assert_eq!(
                observed.admitted_total, observed.terminated_total,
                "class {class}: an admission never terminated"
            );
            assert_eq!(
                (observed.cpu_units_held, observed.memory_units_held),
                (0, 0)
            );
        }
        for (pool, usage) in &snapshot.capabilities {
            assert_eq!(usage.in_use, 0, "pool {pool}: slot leaked");
        }
        assert_eq!(snapshot.conservation_violation(), None);
        assert!(
            self.governor.permit_ledgers().is_empty(),
            "a permit ledger outlived every handle"
        );
    }
}

fuzz_target!(|scenario: Scenario| {
    let fairness_tier = scenario.fairness;
    let mut classes = BTreeMap::new();
    let mut limits = BTreeMap::new();
    let mut memory_cost = BTreeMap::new();
    for (index, shape) in scenario.classes.iter().enumerate() {
        let class = TaskClass::new(CLASSES[index]);
        let policy = class_policy(shape, fairness(fairness_tier, index), index);
        limits.insert(class.clone(), (policy.max_inflight, policy.max_queue_depth));
        let fallback = match &policy.memory_overcommit_policy {
            MemoryOvercommitPolicy::DegradeToLight { fallback_class } => {
                Some(fallback_class.clone())
            }
            _ => None,
        };
        memory_cost.insert(class.clone(), (policy.permit_cost.memory_units, fallback));
        classes.insert(class, policy);
    }
    let budget = (
        u32::from(scenario.budget_cpu % 64),
        u32::from(scenario.budget_memory % 64),
    );
    let blocking_limit = u32::from(scenario.blocking_limit % 5);
    let Ok(policy) = PolicySet::new(
        ResourceBudget::new()
            .cpu_units(budget.0)
            .memory_units(budget.1)
            // Measured / Hybrid permit modes need a byte scale to convert.
            .memory_unit_scale(64),
        classes,
    )
    .with_capability_limits(BTreeMap::from([("blocking".to_string(), blocking_limit)])) else {
        unreachable!("blocking is a built-in pool");
    };
    let clock = Arc::new(ManualClock::new(1_000));
    // A policy the validator refuses (mixed tiers, chained degrade, a queue
    // policy without a queue, …) is a refusal, not a crash: that path is what
    // `policy_topology` fuzzes. Here only buildable policies are driven.
    let Ok(governor) = Governor::new(policy, clock.clone()) else {
        return;
    };
    let mut harness = Harness {
        governor,
        clock,
        limits,
        memory_cost,
        budget: (u128::from(budget.0), u128::from(budget.1)),
        reconciled_upward: false,
        blocking_limit,
        tickets: Vec::new(),
        permits: Vec::new(),
        tokens: Vec::new(),
        ended: Vec::new(),
        closed: false,
    };
    harness.check();
    for (seq, op) in scenario.ops.into_iter().enumerate().take(256) {
        harness.step(op, seq);
        harness.check();
    }
    harness.finish();
});
