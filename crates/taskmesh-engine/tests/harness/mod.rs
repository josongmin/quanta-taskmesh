//! Shared test harness for fairness/scheduling scenarios.
//!
//! Contention is created with a global budget of one cpu unit and per-permit
//! cost of one: only a single permit is inflight at a time, so every other
//! request queues and promotion order is fully observable.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn base() -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(100)
        .max_queue_depth(100)
        .cpu_units(1)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
}

pub fn fifo_class() -> ClassPolicy {
    base().fairness(FairnessPolicy::Fifo)
}

pub fn weighted_class(weight: u32) -> ClassPolicy {
    base().fairness(FairnessPolicy::WeightedFairQueue { weight, burst: 0 })
}

pub fn drr_class(quantum: u32) -> ClassPolicy {
    base().fairness(FairnessPolicy::DeficitRoundRobin { quantum })
}

pub fn deadline_class(slack_ms: u64) -> ClassPolicy {
    base().fairness(FairnessPolicy::DeadlineAware { slack_ms })
}

pub fn best_effort_class(best_effort: bool) -> ClassPolicy {
    base().best_effort(best_effort)
}

/// A governor with the given classes plus an internal `fill` class, contended to
/// a single inflight permit.
pub fn contended(list: Vec<(&'static str, ClassPolicy)>) -> Governor {
    let mut classes: BTreeMap<TaskClass, ClassPolicy> = list
        .into_iter()
        .map(|(n, p)| (TaskClass::new(n), p))
        .collect();
    classes.insert(TaskClass::new("fill"), fifo_class());
    let resources = ResourceBudget::new().cpu_units(1).memory_units(1_000_000);
    Governor::new(
        PolicySet::new(resources, classes),
        Arc::new(ManualClock::new(1000)),
    )
}

fn spec_for(class: &str, op: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new(class.to_string())).operation(op.to_string())
}

/// Occupy the single budget unit with a `fill` permit.
pub fn admit_filler(g: &Governor) -> PermitId {
    admit_filler_on(g, "fill")
}

pub fn admit_filler_on(g: &Governor, class: &str) -> PermitId {
    match g.admit(&spec_for(class, "filler"), RequestKey::new("filler")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("filler must admit, got {other:?}"),
    }
}

/// Enqueue one request on `class`, asserting it queues; returns `(ticket, class)`.
pub fn queue(g: &Governor, class: &'static str, op: &str) -> (u64, String) {
    match g.admit(&spec_for(class, op), RequestKey::new(op.to_string())) {
        AdmissionDecision::Queued { ticket } => (ticket, class.to_string()),
        other => panic!("expected queue for {class}, got {other:?}"),
    }
}

/// Drain promotions in order: repeatedly find the promoted ticket, record its
/// class, and release it (which triggers the next promotion). Returns the class
/// promotion order.
pub fn drain(g: &Governor, tickets: &[(u64, String)]) -> Vec<String> {
    let mut remaining: Vec<(u64, String)> = tickets.to_vec();
    let mut order = Vec::new();
    while !remaining.is_empty() {
        let mut found = None;
        for (idx, (ticket, _)) in remaining.iter().enumerate() {
            if let Some(permit_id) = g.claim(*ticket) {
                found = Some((idx, permit_id));
                break;
            }
        }
        let Some((idx, permit_id)) = found else {
            break; // nothing more promotable
        };
        let (_, class) = remaining.remove(idx);
        order.push(class);
        g.release(permit_id);
    }
    order
}
