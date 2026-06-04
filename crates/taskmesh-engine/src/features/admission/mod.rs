//! Admission (T03): the bounded, fail-closed decision that turns a `TaskSpec`
//! into an `Admitted` / `Queued` / `Rejected` outcome. This slice orchestrates
//! capacity, overflow, memory overcommit (T05), the recursion guard (T06), and
//! fairness enqueue tags (T04); each of those rules lives in its own slice.

use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, MemoryOvercommitPolicy, PermitWaker, TaskClass, TaskScope,
    TaskSpec,
};

use crate::engine::state::{CapacityBlock, GovernedState, PendingRequest};
use crate::features::{composite, fairness};
use crate::shared::{
    AdmissionDecision, PermitId, PolicySet, Provenance, RequestKey, ResolvedCost, Seq, Ticket,
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

// ---- service (stateful) ---------------------------------------------------

/// Pre-allocated identities for one admission attempt. Ids are cheap and unique;
/// over-allocating on a rejected/queued path is harmless.
#[derive(Clone, Copy)]
pub struct Ids {
    pub permit_id: PermitId,
    pub ticket: Ticket,
    pub seq_no: Seq,
}

/// Admit `spec`. Fail-closed throughout: unknown/disabled classes and recursive
/// loops reject before any capacity is considered.
pub fn admit(
    state: &mut GovernedState,
    policies: &PolicySet,
    now_ms: u64,
    spec: &TaskSpec,
    key: RequestKey,
    waker: Option<Arc<dyn PermitWaker>>,
    ids: Ids,
) -> AdmissionDecision {
    // Fail-closed on a malformed plan before any capacity is considered:
    //  - zero-stage / inconsistent per-stage class (`validate_shape`), and
    //  - a fan-out stage without a complete deterministic reduce policy (T06).
    // Either makes the spec unshippable, so it must never be admitted.
    if composite::validate_shape(spec).is_err() || composite::reduce::validate_spec(spec).is_err() {
        return AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask);
    }
    admit_class(
        state,
        policies,
        now_ms,
        spec,
        &spec.class,
        key,
        waker,
        ids,
        true,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "single admission path threads request + ids + waker together"
)]
fn admit_class(
    state: &mut GovernedState,
    policies: &PolicySet,
    now_ms: u64,
    spec: &TaskSpec,
    class: &TaskClass,
    key: RequestKey,
    waker: Option<Arc<dyn PermitWaker>>,
    ids: Ids,
    degrade_allowed: bool,
) -> AdmissionDecision {
    let Some(policy) = policies.class(class) else {
        return AdmissionDecision::Rejected(AdmissionVerdict::UnknownClass {
            class: class.clone(),
        });
    };
    if policy.is_disabled() {
        return AdmissionDecision::Rejected(AdmissionVerdict::ClassDisabled);
    }
    if composite::is_recursive(state, spec) {
        return AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission);
    }

    let cost = resolve_cost(policy);
    match state.capacity_for(class, policy.max_inflight, cost, &policies.resources) {
        CapacityBlock::Ok => {
            state.grant(
                ids.permit_id,
                class,
                &spec.root_operation_id,
                spec.scope.clone(),
                composite::target_stage(spec),
                Provenance::of(spec),
                cost,
                cost.memory_units,
                now_ms,
            );
            AdmissionDecision::Admitted {
                permit_id: ids.permit_id,
            }
        }
        CapacityBlock::Inflight | CapacityBlock::Cpu => {
            saturated(state, policy, class, spec, key, waker, ids, now_ms)
        }
        CapacityBlock::Memory => match &policy.memory_overcommit_policy {
            MemoryOvercommitPolicy::Queue => {
                enqueue_or_full(state, policy, class, spec, key, waker, ids, now_ms, true)
            }
            MemoryOvercommitPolicy::DegradeToLight { fallback_class } if degrade_allowed => {
                let fallback = fallback_class.clone();
                admit_class(
                    state, policies, now_ms, spec, &fallback, key, waker, ids, false,
                )
            }
            // Reject, or a degrade that is no longer allowed (already degraded
            // once): both shed with MemorySaturated.
            MemoryOvercommitPolicy::Reject | MemoryOvercommitPolicy::DegradeToLight { .. } => {
                AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated {
                    retry_after_ms: retry(policy, state, class),
                })
            }
        },
    }
}

/// Handle an inflight/cpu saturation: queue if the class is queueable, else
/// reject with a `CpuSaturated` verdict.
#[allow(
    clippy::too_many_arguments,
    reason = "single admission path threads request + ids + waker together"
)]
fn saturated(
    state: &mut GovernedState,
    policy: &ClassPolicy,
    class: &TaskClass,
    spec: &TaskSpec,
    key: RequestKey,
    waker: Option<Arc<dyn PermitWaker>>,
    ids: Ids,
    now_ms: u64,
) -> AdmissionDecision {
    if policy.overflow_policy.is_queueable() {
        enqueue_or_full(state, policy, class, spec, key, waker, ids, now_ms, false)
    } else {
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated {
            retry_after_ms: retry(policy, state, class),
        })
    }
}

/// Enqueue within `max_queue_depth`, else `QueueFull`. `memory_overcommit`
/// distinguishes the memory-driven queue path (which uses `MemorySaturated` for
/// the full case) from the capacity-driven one.
#[allow(
    clippy::too_many_arguments,
    reason = "single admission path threads request + ids + waker together"
)]
fn enqueue_or_full(
    state: &mut GovernedState,
    policy: &ClassPolicy,
    class: &TaskClass,
    spec: &TaskSpec,
    key: RequestKey,
    waker: Option<Arc<dyn PermitWaker>>,
    ids: Ids,
    now_ms: u64,
    memory_overcommit: bool,
) -> AdmissionDecision {
    if state.queued(class) >= policy.max_queue_depth {
        let retry_after_ms = retry(policy, state, class);
        return AdmissionDecision::Rejected(if memory_overcommit {
            AdmissionVerdict::MemorySaturated { retry_after_ms }
        } else {
            AdmissionVerdict::QueueFull { retry_after_ms }
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
        request_key: key,
        root_operation_id: spec.root_operation_id.clone(),
        scope: spec.scope.clone(),
        target_stage: target_stage.clone(),
        provenance: Provenance::of(spec),
        cost,
        enqueued_at_ms: now_ms,
        deadline_ms,
        finish_tag,
        waker,
    });
    // A queued child occupies the recursion guard too: otherwise a recursive
    // child could bypass the guard by queueing (the guard is consulted on the
    // sync admit path only, and `promote` grants queued heads unconditionally).
    if matches!(spec.scope, TaskScope::Child { .. }) {
        state
            .active_recursion
            .insert((spec.root_operation_id.clone(), target_stage));
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
