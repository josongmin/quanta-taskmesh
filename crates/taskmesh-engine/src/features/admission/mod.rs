//! Admission (T03): the bounded, fail-closed decision that turns a `TaskSpec`
//! into an `Admitted` / `Queued` / `Rejected` outcome. This slice orchestrates
//! capacity, overflow, memory overcommit (T05), the recursion guard (T06), and
//! fairness enqueue tags (T04); each of those rules lives in its own slice.
//!
//! # One decision, both authorities
//!
//! Admission decides semantic capacity *and* physical capability occupancy in
//! the same transition. A request that cannot have a worker is not "admitted and
//! waiting": it is queued under its class's own [`taskmesh_contract::OverflowPolicy`]
//! within `max_queue_depth`, or shed. There is no unbounded holding area in
//! front of this decision, so `Reject` means reject and `queued` counts what is
//! actually waiting.
//!
//! # Resolved, then frozen
//!
//! The capability a request needs is resolved *before* intake and frozen onto
//! the pending record. Promotion never re-derives it, so a request cannot enter
//! the queue needing a scarce pool and leave it charged against a cheap one.

use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, MemoryOvercommitPolicy, PermitWaker, TaskClass, TaskScope,
    TaskSpec,
};

use crate::engine::state::{
    CapacityBlock, GovernedState, GrantRequest, PendingRequest, TransitionEffects,
};
use crate::features::{composite, fairness};
use crate::shared::{
    AdmissionDecision, CapabilityName, PermitId, PolicySet, Provenance, ResolvedCost, Seq, Ticket,
};

// ---- domain (pure) --------------------------------------------------------

/// The resource a permit reserves at admit time (the memory estimate; measured
/// modes reconcile later).
pub fn resolve_cost(policy: &ClassPolicy) -> ResolvedCost {
    ResolvedCost {
        cpu_units: policy.permit_cost.cpu_units,
        memory_units: policy.permit_cost.memory_units,
    }
}

/// Map a blocking limit to the verdict a non-queueing class sheds with.
pub fn verdict_for_block(block: CapacityBlock, retry_after_ms: Option<u64>) -> AdmissionVerdict {
    match block {
        CapacityBlock::Memory => AdmissionVerdict::MemorySaturated { retry_after_ms },
        CapacityBlock::Capability => AdmissionVerdict::SubstrateSaturated { retry_after_ms },
        // The ledger is untrustworthy: refuse work rather than admit against a
        // total the engine could not compute.
        CapacityBlock::AccountingFault => AdmissionVerdict::RuntimeUnavailable,
        // `QueuedBehind` is never shed by this mapping — a queue exists by
        // definition, so admission enqueues (or reports `QueueFull`). Listed for
        // exhaustiveness with the class-busy verdicts.
        CapacityBlock::Inflight
        | CapacityBlock::Cpu
        | CapacityBlock::QueuedBehind
        | CapacityBlock::Ok => AdmissionVerdict::CpuSaturated { retry_after_ms },
    }
}

// ---- service (stateful) ---------------------------------------------------

/// Pre-allocated identities for one admission attempt. Ids are cheap and unique;
/// over-allocating on a rejected/queued path is harmless.
#[derive(Clone, Copy)]
pub struct Ids {
    pub permit_id: PermitId,
    pub ticket: Ticket,
    pub seq_no: Seq,
}

/// One intake request: the spec plus the capability the host resolved for it.
pub struct AdmissionRequest<'a> {
    pub spec: &'a TaskSpec,
    /// The capability pool this work will actually occupy. `None` for an ungated
    /// substrate (async I/O concurrency is unbounded by design).
    pub capability: Option<CapabilityName>,
}

/// The stateless part of admission: fail closed on a malformed plan before any
/// capacity — or the state mutex — is touched.
///
///  - zero-stage / inconsistent per-stage class (`validate_shape`), and
///  - a fan-out stage without a complete deterministic reduce policy (T06).
///
/// Either makes the spec unshippable, so it must never be admitted.
pub fn precheck(spec: &TaskSpec) -> Result<(), AdmissionVerdict> {
    if composite::validate_shape(spec).is_err() || composite::reduce::validate_spec(spec).is_err() {
        return Err(AdmissionVerdict::MalformedTask);
    }
    Ok(())
}

/// Whether the class has any path into a queue: a queueable overflow policy or
/// a memory policy that queues on overcommit.
pub fn class_can_queue(policy: &ClassPolicy) -> bool {
    policy.overflow_policy.is_queueable()
        || policy.memory_overcommit_policy == MemoryOvercommitPolicy::Queue
}

/// Admit `request`. Fail-closed throughout: unknown/disabled classes and
/// recursive loops reject before any capacity is considered. The caller must
/// have run [`precheck`] outside the lock; it is not repeated here (a
/// malformed spec never reaches this transition — see `Governor::admit_inner`).
pub fn admit(
    state: &mut GovernedState,
    policies: &PolicySet,
    now_ms: u64,
    request: &AdmissionRequest<'_>,
    waker: Option<Arc<dyn PermitWaker>>,
    ids: Ids,
    effects: &mut TransitionEffects,
) -> AdmissionDecision {
    let spec = request.spec;
    debug_assert!(
        precheck(spec).is_ok(),
        "precheck runs before the lock is taken"
    );
    // D17: a closed runtime is the outermost fact about a submission. Decided
    // under the admission lock, before the class is even looked up, so nothing
    // is queued, charged, or counted — and the waker (host code) is retired
    // outside the lock like every other refusal's.
    if state.admission_closed {
        retire(effects, waker);
        return AdmissionDecision::Rejected(AdmissionVerdict::RuntimeUnavailable);
    }
    admit_class(
        state,
        policies,
        now_ms,
        request,
        &spec.class,
        waker,
        ids,
        true,
        effects,
    )
}

/// Hand a host `Arc` to the effect list so its destructor runs after the mutex
/// is released. A `PermitWaker` is host code in both directions: `wake()` *and*
/// `drop()`.
fn retire(effects: &mut TransitionEffects, waker: Option<Arc<dyn PermitWaker>>) {
    if let Some(waker) = waker {
        effects.retire.push(waker);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "single admission path threads request + ids + waker + effects together"
)]
fn admit_class(
    state: &mut GovernedState,
    policies: &PolicySet,
    now_ms: u64,
    request: &AdmissionRequest<'_>,
    class: &TaskClass,
    waker: Option<Arc<dyn PermitWaker>>,
    ids: Ids,
    degrade_allowed: bool,
    effects: &mut TransitionEffects,
) -> AdmissionDecision {
    let spec = request.spec;
    let Some(policy) = policies.class(class) else {
        retire(effects, waker);
        return AdmissionDecision::Rejected(AdmissionVerdict::UnknownClass {
            class: class.clone(),
        });
    };
    if policy.is_disabled() {
        retire(effects, waker);
        return AdmissionDecision::Rejected(AdmissionVerdict::ClassDisabled);
    }
    if composite::is_recursive(state, spec) {
        retire(effects, waker);
        return AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission);
    }

    let cost = resolve_cost(policy);
    let mut block = state.capacity_for(
        class,
        policy.max_inflight,
        cost,
        request.capability.as_deref(),
        &policies.resources,
        policies.capability_limits(),
    );
    // D08: capacity that opened while a promotion pass is still owed belongs to
    // the queue. Two shapes:
    //  * an earlier arrival of *this* class is queued and runnable — only
    //    waiting for the continuation pass (`queued_head_blocks`);
    //  * a continuation is pending (`promotion_pending`), this class can queue,
    //    and its own queue is empty — the newcomer becomes its class's head and
    //    the pass, not arrival luck in the lock gap, orders it against the
    //    other classes. With a non-empty own queue the first rule decides: a
    //    runnable head is waited for, a head blocked on another capability pool
    //    is not (queueing behind it would pin this request to that pool's
    //    saturation — the head-of-line loss the exception exists to prevent).
    // A class that cannot queue has nowhere to wait and is admitted; the gap is
    // the only moment a runnable head can be overtaken, and only by work that
    // could not have waited.
    if block == CapacityBlock::Ok
        && (state.queued_head_blocks(
            class,
            policy.max_inflight,
            &policies.resources,
            policies.capability_limits(),
        ) || (state.promotion_pending && class_can_queue(policy) && state.queued(class) == 0))
    {
        block = CapacityBlock::QueuedBehind;
    }
    match block {
        CapacityBlock::Ok => {
            state.grant(GrantRequest {
                permit_id: ids.permit_id,
                class,
                root_operation_id: &spec.root_operation_id,
                scope: spec.scope.clone(),
                target_stage: composite::target_stage(spec),
                provenance: Provenance::of(spec),
                capability: request.capability.clone(),
                cost,
                reserved_units: cost.memory_units,
                now_ms,
                pending_ticket: None,
                claim_waker: None,
            });
            retire(effects, waker);
            AdmissionDecision::Admitted {
                permit_id: ids.permit_id,
            }
        }
        CapacityBlock::AccountingFault => {
            // Unreachable by construction today: every aggregate is `u128` and
            // a per-request cost is `u32`, so the checked add cannot fail. It is
            // still wired, because the alternative to a fail-closed branch here
            // is admitting work against a total the engine could not compute.
            // Sticky: a ledger that lost one total is not trusted to have lost
            // only one.
            state.record_accounting_fault("capacity aggregate is not representable");
            retire(effects, waker);
            AdmissionDecision::Rejected(AdmissionVerdict::RuntimeUnavailable)
        }
        // Capacity exists but the queue is ahead. The class demonstrably has a
        // queue (the head is in it), whichever policy opened it — a
        // `Reject`-overflow class whose memory policy is `Queue` queues too —
        // so the newcomer joins it within depth or is told the queue is full.
        CapacityBlock::QueuedBehind => enqueue_or_full(
            state, policy, class, request, waker, ids, now_ms, block, effects,
        ),
        // Inflight, CPU, and capability saturation all obey the class's overflow
        // policy: queue within depth, or shed with the cause — unless the wait
        // is a declared cycle (D12), which neither queueing nor a retry can
        // ever satisfy.
        CapacityBlock::Inflight | CapacityBlock::Cpu | CapacityBlock::Capability => {
            if let Some(held_by_root) = composite::declared_wait_cycle(
                state,
                spec,
                class,
                request.capability.as_ref(),
                block,
            ) {
                retire(effects, waker);
                return AdmissionDecision::Rejected(AdmissionVerdict::NestedWaitCycle {
                    held_by_root,
                });
            }
            if policy.overflow_policy.is_queueable() {
                enqueue_or_full(
                    state, policy, class, request, waker, ids, now_ms, block, effects,
                )
            } else {
                retire(effects, waker);
                AdmissionDecision::Rejected(verdict_for_block(block, retry(policy, state, class)))
            }
        }
        CapacityBlock::Memory => {
            // Degrade first: a fallback with room is not a wait at all, and the
            // fallback's own admission judges its own block. D03: a fallback
            // reclassifies the *resource* account only; the capability the
            // work needs is a property of the work and is carried across.
            if let MemoryOvercommitPolicy::DegradeToLight { fallback_class } =
                &policy.memory_overcommit_policy
            {
                if degrade_allowed {
                    let fallback = fallback_class.clone();
                    return admit_class(
                        state, policies, now_ms, request, &fallback, waker, ids, false, effects,
                    );
                }
            }
            // A queue or a shed: neither helps a declared cycle (D12).
            if let Some(held_by_root) = composite::declared_wait_cycle(
                state,
                spec,
                class,
                request.capability.as_ref(),
                block,
            ) {
                retire(effects, waker);
                return AdmissionDecision::Rejected(AdmissionVerdict::NestedWaitCycle {
                    held_by_root,
                });
            }
            match &policy.memory_overcommit_policy {
                MemoryOvercommitPolicy::Queue => enqueue_or_full(
                    state, policy, class, request, waker, ids, now_ms, block, effects,
                ),
                // Reject, or a degrade that is no longer allowed (already
                // degraded once): both shed with MemorySaturated.
                MemoryOvercommitPolicy::Reject | MemoryOvercommitPolicy::DegradeToLight { .. } => {
                    retire(effects, waker);
                    AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated {
                        retry_after_ms: retry(policy, state, class),
                    })
                }
            }
        }
    }
}

/// Enqueue within `max_queue_depth`, else shed with the blocking cause.
#[allow(
    clippy::too_many_arguments,
    reason = "single admission path threads request + ids + waker + effects together"
)]
fn enqueue_or_full(
    state: &mut GovernedState,
    policy: &ClassPolicy,
    class: &TaskClass,
    request: &AdmissionRequest<'_>,
    waker: Option<Arc<dyn PermitWaker>>,
    ids: Ids,
    now_ms: u64,
    blocked_on: CapacityBlock,
    effects: &mut TransitionEffects,
) -> AdmissionDecision {
    let spec = request.spec;
    if state.queued(class) >= policy.max_queue_depth {
        let retry_after_ms = retry(policy, state, class);
        retire(effects, waker);
        return AdmissionDecision::Rejected(match blocked_on {
            CapacityBlock::Memory => AdmissionVerdict::MemorySaturated { retry_after_ms },
            CapacityBlock::Capability => AdmissionVerdict::SubstrateSaturated { retry_after_ms },
            _ => AdmissionVerdict::QueueFull { retry_after_ms },
        });
    }

    let cost = resolve_cost(policy);
    let target_stage = composite::target_stage(spec);
    let (finish_tag, deadline_ms) =
        fairness::enqueue_tags(state, class, policy, cost.cpu_units, now_ms);
    state.class_mut(class).queue.push_back(PendingRequest {
        ticket: ids.ticket,
        seq_no: ids.seq_no,
        class: class.clone(),
        // Queue/audit only: direct admit and terminal reject paths must not pay
        // this allocation on the governance hot path.
        request_key: crate::shared::RequestKey::from_root(&spec.root_operation_id),
        root_operation_id: spec.root_operation_id.clone(),
        scope: spec.scope.clone(),
        target_stage: target_stage.clone(),
        provenance: Provenance::of(spec),
        cost,
        capability: request.capability.clone(),
        enqueued_at_ms: now_ms,
        deadline_ms,
        finish_tag,
        blocked_on,
        waker,
    });
    state.tickets.insert(
        ids.ticket,
        crate::engine::state::TicketState::Queued {
            class: class.clone(),
        },
    );
    // A queued child occupies the recursion guard too: otherwise a recursive
    // child could bypass the guard by queueing (the guard is consulted on the
    // sync admit path only, and `promote` grants queued heads unconditionally).
    if matches!(spec.scope, TaskScope::Child { .. }) {
        state.occupy_recursion_guard(&spec.root_operation_id, &target_stage);
    }
    AdmissionDecision::Queued { ticket: ids.ticket }
}

fn retry(policy: &ClassPolicy, state: &GovernedState, class: &TaskClass) -> Option<u64> {
    fairness::retry_after(
        policy.retry_after_policy,
        policy.fairness,
        state.queued(class),
        state.inflight(class),
    )
}
