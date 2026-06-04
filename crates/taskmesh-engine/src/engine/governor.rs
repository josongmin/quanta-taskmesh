//! The governor: the application service at the hexagon's core. It owns the
//! driven [`Clock`] port and the mutex-guarded [`GovernedState`], and delegates
//! every rule to a feature slice. No Tokio, no I/O, no product taxonomy.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use taskmesh_contract::{
    CheckpointPolicy, ClassSnapshot, Clock, FairnessPolicy, GovernorError, MemoryOvercommitPolicy,
    MemoryPermitMode, OverflowPolicy, PermitWaker, Snapshot, SubstrateRecord, TaskClass, TaskSpec,
};

use crate::engine::state::GovernedState;
use crate::features::{admission, composite, fairness, inventory, memory};
use crate::shared::{
    AdmissionDecision, LeakSweepReport, PermitId, PolicySet, Provenance, RequestKey,
    RootAttribution, Ticket,
};

/// The governed execution control point. Cheap to wrap in `Arc` and share.
pub struct Governor {
    policy: PolicySet,
    clock: Arc<dyn Clock>,
    next_permit: AtomicU64,
    next_ticket: AtomicU64,
    next_seq: AtomicU64,
    state: Mutex<GovernedState>,
}

impl Governor {
    /// Construct a governor, **validating the policy fail-closed** first. Any
    /// impossible budget, invalid memory scaling, mixed-tier fairness, or misused
    /// degrade/queue policy is rejected here — a direct engine embedder (via
    /// `taskmesh::ext`) gets the same construction-time rejection the host
    /// `Builder` enforces, so contradictory policy can never boot.
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

    fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    fn fresh_ids(&self) -> admission::Ids {
        admission::Ids {
            permit_id: self.next_permit.fetch_add(1, Ordering::Relaxed),
            ticket: self.next_ticket.fetch_add(1, Ordering::Relaxed),
            seq_no: self.next_seq.fetch_add(1, Ordering::Relaxed),
        }
    }

    // ---- admission --------------------------------------------------------

    /// Admit a request without a promotion waker (caller does not wait on a queue).
    pub fn admit(&self, spec: &TaskSpec, key: RequestKey) -> AdmissionDecision {
        self.admit_inner(spec, key, None)
    }

    /// Admit a request, registering a waker that fires if the request is queued
    /// and later promoted.
    pub fn admit_waitable(
        &self,
        spec: &TaskSpec,
        key: RequestKey,
        waker: Arc<dyn PermitWaker>,
    ) -> AdmissionDecision {
        self.admit_inner(spec, key, Some(waker))
    }

    fn admit_inner(
        &self,
        spec: &TaskSpec,
        key: RequestKey,
        waker: Option<Arc<dyn PermitWaker>>,
    ) -> AdmissionDecision {
        let now = self.now_ms();
        let ids = self.fresh_ids();
        let mut state = self.state.lock();
        admission::admit(&mut state, &self.policy, now, spec, key, waker, ids)
    }

    /// Claim a permit for a previously-queued ticket once it has been promoted.
    pub fn claim(&self, ticket: Ticket) -> Option<PermitId> {
        self.state.lock().granted.remove(&ticket)
    }

    /// Abandon a queued/promoted ticket (e.g. on acquire timeout). Releases the
    /// permit if the ticket had already been promoted.
    pub fn abandon(&self, ticket: Ticket) {
        let mut state = self.state.lock();
        if let Some(permit_id) = state.granted.remove(&ticket) {
            drop(state);
            self.release(permit_id);
            return;
        }
        let mut released_guard = None;
        for cstate in state.classes.values_mut() {
            if let Some(pos) = cstate.queue.iter().position(|r| r.ticket == ticket) {
                let req = cstate.queue.remove(pos).expect("position just found");
                if matches!(req.scope, taskmesh_contract::TaskScope::Child { .. }) {
                    released_guard = Some((req.root_operation_id, req.target_stage));
                }
                break;
            }
        }
        // A dequeued child no longer occupies the recursion guard.
        if let Some(key) = released_guard {
            state.active_recursion.remove(&key);
        }
    }

    // ---- permit lifecycle -------------------------------------------------

    /// Release a permit, returning its resources and promoting queued work.
    pub fn release(&self, permit_id: PermitId) {
        let now = self.now_ms();
        let wakers = {
            let mut state = self.state.lock();
            if state.unwind(permit_id).is_none() {
                return;
            }
            self.promote(&mut state, now)
        };
        // Fire host callbacks AFTER releasing the governor lock: `PermitWaker` is a
        // driven host port, and a blocking or re-entrant implementation must never
        // run under the engine mutex (deadlock / global admission serialization).
        wake_all(wakers);
    }

    /// Promote queued work into freed capacity, fairly, until nothing is runnable.
    /// Returns the wakers of the promoted requests to be fired *after* the lock
    /// is released.
    #[must_use]
    fn promote(&self, state: &mut GovernedState, now: u64) -> Vec<Arc<dyn PermitWaker>> {
        let mut wakers = Vec::new();
        while let Some(class) = fairness::select(state, &self.policy) {
            let Some(head) = state.class_mut(&class).queue.pop_front() else {
                break;
            };
            let permit_id = self.next_permit.fetch_add(1, Ordering::Relaxed);
            state.grant(
                permit_id,
                &class,
                &head.root_operation_id,
                head.scope.clone(),
                head.target_stage.clone(),
                head.provenance,
                head.cost,
                head.cost.memory_units,
                now,
            );
            state.granted.insert(head.ticket, permit_id);
            if let Some(waker) = head.waker {
                wakers.push(waker);
            }
        }
        wakers
    }

    // ---- memory governance (T05) -----------------------------------------

    /// Reconcile a permit's held memory against a measured byte reading.
    ///
    /// A *downward* reconcile frees memory, so queued work is promoted (like
    /// `release_stage_memory`/leak sweep). An *upward* reconcile records measured
    /// reality even if it pushes held memory past `max_memory_units`; the cap is
    /// then enforced fail-closed for *new* admissions (`capacity_for` blocks any
    /// admit while `held > budget`), so an overcommitting in-flight task cannot
    /// let new work in.
    pub fn reconcile_memory(&self, permit_id: PermitId, measured_bytes: u64) -> bool {
        let now = self.now_ms();
        let (ok, wakers) = {
            let mut state = self.state.lock();
            let Some(record) = state.permits.get(&permit_id) else {
                return false;
            };
            let Some(policy) = self.policy.class(&record.class) else {
                return false;
            };
            let mode = policy.memory_permit_mode;
            let scale = self.policy.resources.memory_unit_scale;
            let ok = memory::reconcile(&mut state, permit_id, measured_bytes, mode, scale, now);
            // Promote unconditionally on success: a downward reconcile freed
            // budget (promotes queued work); an upward one leaves no capacity so
            // promote is a no-op.
            let wakers = if ok {
                self.promote(&mut state, now)
            } else {
                Vec::new()
            };
            (ok, wakers)
        };
        wake_all(wakers);
        ok
    }

    /// Return `freed_units` of a still-held permit's memory at a stage boundary.
    pub fn release_stage_memory(&self, permit_id: PermitId, freed_units: u32) -> u32 {
        let now = self.now_ms();
        let (freed, wakers) = {
            let mut state = self.state.lock();
            let freed = memory::release_stage_memory(&mut state, permit_id, freed_units);
            let wakers = if freed > 0 {
                self.promote(&mut state, now)
            } else {
                Vec::new()
            };
            (freed, wakers)
        };
        wake_all(wakers);
        freed
    }

    /// Sweep for leaked permits using the default staleness window.
    pub fn reap_leaks(&self) -> LeakSweepReport {
        self.reap_leaks_with(memory::DEFAULT_LEAK_STALE_MS)
    }

    /// Sweep for leaked permits older than `stale_after_ms`.
    pub fn reap_leaks_with(&self, stale_after_ms: u64) -> LeakSweepReport {
        let now = self.now_ms();
        let (report, wakers) = {
            let mut state = self.state.lock();
            let report = memory::reap_leaks(&mut state, &self.policy, now, stale_after_ms);
            let wakers = if report.reclaimed_permits > 0 {
                self.promote(&mut state, now)
            } else {
                Vec::new()
            };
            (report, wakers)
        };
        wake_all(wakers);
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
        inventory::snapshot(&self.policy.substrates)
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
            let (inflight, queued, cpu, mem) =
                state.classes.get(&class).map_or((0, 0, 0, 0), |c| {
                    (
                        c.inflight,
                        c.queued(),
                        c.cpu_units_held,
                        c.memory_units_held,
                    )
                });
            classes.entry(class).or_insert(ClassSnapshot {
                inflight,
                queued,
                cpu_units_held: cpu,
                memory_units_held: mem,
            });
        }
        Snapshot {
            classes,
            substrates: inventory::snapshot(&self.policy.substrates),
        }
    }

    // ---- validation (T02) -------------------------------------------------

    /// Fail-closed configuration validation. Every impossible budget is rejected
    /// at construction, never deferred to a runtime admit path.
    pub fn validate_policy(policy: &PolicySet) -> Result<(), GovernorError> {
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
                if !policy.classes.contains_key(fallback_class) {
                    return Err(violation(format!(
                        "class {class} degrades to unknown fallback {fallback_class}"
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
}

fn violation(message: String) -> GovernorError {
    GovernorError::PolicyViolation(message.into())
}

/// Fire promotion wakers outside the governor lock.
fn wake_all(wakers: Vec<Arc<dyn PermitWaker>>) {
    for waker in wakers {
        waker.wake();
    }
}
