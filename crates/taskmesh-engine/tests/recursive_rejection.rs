//! T06: a child re-entering an active (root, stage) is a recursive loop -> reject.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov() -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("worker"),
        ClassPolicy::new().max_inflight(8).cpu_units(1),
    );
    Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new().cpu_units(100), classes),
        Arc::new(ManualClock::new(1000)),
    )
}

// A blocking child occupies stage "blocking" under its root.
fn child(root: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("worker"))
        .child_of(root.to_string(), TaskStage::new("parent"))
}

#[test]
fn recursive_same_root_same_stage_rejects() {
    let g = gov();
    assert!(matches!(
        g.admit(&child("root-r"), RequestKey::new("root-r")),
        AdmissionDecision::Admitted { .. }
    ));
    // Re-entry of the same (root, stage) while active is rejected.
    assert!(matches!(
        g.admit(&child("root-r"), RequestKey::new("root-r")),
        AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission)
    ));
}

#[test]
fn root_tasks_never_trip_recursion_guard() {
    let g = gov();
    let root = TaskSpec::blocking(TaskClass::new("worker")).operation("plain");
    assert!(matches!(
        g.admit(&root, RequestKey::new("plain")),
        AdmissionDecision::Admitted { .. }
    ));
    // A second independent root op is fine.
    let root2 = TaskSpec::blocking(TaskClass::new("worker")).operation("plain-2");
    assert!(matches!(
        g.admit(&root2, RequestKey::new("plain-2")),
        AdmissionDecision::Admitted { .. }
    ));
}

#[test]
fn distinct_stages_under_same_root_are_allowed() {
    let g = gov();
    let a = TaskSpec::blocking(TaskClass::new("worker")).child_of("root-x", TaskStage::new("p1"));
    let b = TaskSpec::cpu(TaskClass::new("worker")).child_of("root-x", TaskStage::new("p2"));
    assert!(matches!(
        g.admit(&a, RequestKey::new("root-x")),
        AdmissionDecision::Admitted { .. }
    ));
    assert!(matches!(
        g.admit(&b, RequestKey::new("root-x")),
        AdmissionDecision::Admitted { .. }
    ));
}

// --- F3 regression: the queue path must not bypass the recursion guard. ---

fn gov_one_slot() -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("worker"),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(8)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new().cpu_units(100), classes),
        Arc::new(ManualClock::new(1000)),
    )
}

#[test]
fn queued_recursive_child_is_rejected_not_bypassed() {
    let g = gov_one_slot();
    // Occupy the single slot with a non-recursive root so children must queue.
    let occ = TaskSpec::blocking(TaskClass::new("worker")).operation("occ");
    let _p = match g.admit(&occ, RequestKey::new("occ")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    // First recursive child queues (and now occupies the recursion guard).
    assert!(matches!(
        g.admit(&child("root"), RequestKey::new("root")),
        AdmissionDecision::Queued { .. }
    ));
    // Second identical child must be rejected as recursive, NOT queued.
    assert!(matches!(
        g.admit(&child("root"), RequestKey::new("root")),
        AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission)
    ));
}

#[test]
fn queued_child_promotes_then_guard_clears_on_release() {
    let g = gov_one_slot();
    let occ = TaskSpec::blocking(TaskClass::new("worker")).operation("occ");
    let occ_permit = match g.admit(&occ, RequestKey::new("occ")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    let ticket = match g.admit(&child("root"), RequestKey::new("root")) {
        AdmissionDecision::Queued { ticket } => ticket,
        o => panic!("{o:?}"),
    };
    // Promote the queued child by releasing the occupant.
    g.release(occ_permit);
    let child_permit = g.claim(ticket).expect("child promoted");
    // Releasing the child clears the recursion guard, so a fresh child admits.
    g.release(child_permit);
    assert!(matches!(
        g.admit(&child("root"), RequestKey::new("root")),
        AdmissionDecision::Admitted { .. }
    ));
}

#[test]
fn abandoning_queued_child_clears_the_guard() {
    let g = gov_one_slot();
    let occ = TaskSpec::blocking(TaskClass::new("worker")).operation("occ");
    let _p = match g.admit(&occ, RequestKey::new("occ")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    let ticket = match g.admit(&child("root"), RequestKey::new("root")) {
        AdmissionDecision::Queued { ticket } => ticket,
        o => panic!("{o:?}"),
    };
    g.abandon(ticket);
    // After abandoning the queued child, a new child may queue again.
    assert!(matches!(
        g.admit(&child("root"), RequestKey::new("root")),
        AdmissionDecision::Queued { .. }
    ));
}
