//! Child-scope lifecycle regressions (H16-003).
//!
//! A child occupies its `(root, stage)` recursion-guard slot and its root's
//! attribution from the moment it is *queued*, not just when it runs. Every way
//! a child can leave — abandoned while queued, promoted then reclaimed,
//! released after running — must give both back exactly once, or the root's
//! next child at that stage is refused as "recursive" forever.
//!
//! A permit handle from a *different* governor is also pinned here: ids are
//! per-governor, so a handle that happens to collide with a live id elsewhere
//! must not free that other execution.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ManualClock, MemoryReleasePolicy, OverflowPolicy,
    ResourceBudget, TaskClass, TaskSpec, TaskStage,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome, TerminalReason,
    Ticket,
};

fn governor(max_inflight: u32) -> (Governor, Arc<ManualClock>) {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("worker"),
        ClassPolicy::new()
            .max_inflight(max_inflight)
            .max_queue_depth(8)
            .cpu_units(1)
            .memory_units(2)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
    );
    let clock = Arc::new(ManualClock::new(1_000));
    let g = Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(100).memory_units(100),
            classes,
        ),
        #[allow(
            clippy::clone_on_ref_ptr,
            reason = "Arc<ManualClock> -> Arc<dyn Clock>"
        )]
        clock.clone(),
    )
    .expect("valid policy");
    (g, clock)
}

fn child(root: &str, stage: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("worker"))
        .child_of(
            root.to_string(),
            root.to_string(),
            TaskStage::new(stage.to_string()),
        )
        .operation(format!("child-{stage}"))
}

fn root(op: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("worker")).operation(op.to_string())
}

fn admit(g: &Governor, spec: &TaskSpec) -> PermitId {
    match g.admit(spec) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn queue(g: &Governor, spec: &TaskSpec) -> Ticket {
    match g.admit(spec) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    }
}

#[test]
fn an_abandoned_queued_child_frees_its_recursion_slot() {
    let (g, _clock) = governor(1);
    let holder = admit(&g, &root("holder"));
    let ticket = queue(&g, &child("R", "map"));
    // Queued, and already guarding: a second child at the same stage is
    // recursive right now, even though the first has not run.
    assert!(matches!(
        g.admit(&child("R", "map")),
        AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission)
    ));
    // Attribution starts at grant, not at queue: nothing is charged yet.
    assert!(g.root_attribution("R").is_none());

    let _ = g.abandon(ticket);
    // The slot is free again: the same child can be queued afresh.
    let again = queue(&g, &child("R", "map"));
    assert!(matches!(g.ticket_status(again), ClaimOutcome::Pending));

    // And it runs to completion normally once capacity frees.
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(permit) = g.claim(again) else {
        panic!("promoted");
    };
    let attribution = g
        .root_attribution("R")
        .expect("a granted child is attributed");
    assert_eq!(attribution.child_inflight, 1);
    assert_eq!(attribution.cpu_units, 1);
    assert_eq!(attribution.memory_units, 2);
    assert_eq!(attribution.active_stages, 1);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
    assert!(
        g.root_attribution("R").is_none(),
        "last child out clears the root"
    );
    assert!(matches!(
        g.admit(&child("R", "map")),
        AdmissionDecision::Admitted { .. }
    ));
}

#[test]
fn a_promoted_child_that_is_reclaimed_gives_back_slot_and_attribution() {
    let (g, clock) = governor(1);
    let holder = admit(&g, &root("holder"));
    let ticket = queue(&g, &child("R", "reduce"));
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    // Promoted but never claimed: attributed, guarded, and stale after a while.
    assert_eq!(
        g.root_attribution("R")
            .expect("promoted child")
            .child_inflight,
        1
    );
    assert!(matches!(
        g.admit(&child("R", "reduce")),
        AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission)
    ));

    clock.advance(11);
    assert_eq!(g.reap_leaks_with(10).reclaimed_permits, 1);
    // The waiter is told; the root is clean; the stage is free.
    assert_eq!(
        g.claim(ticket),
        ClaimOutcome::Terminal(TerminalReason::Reclaimed)
    );
    assert!(g.root_attribution("R").is_none());
    assert!(matches!(
        g.admit(&child("R", "reduce")),
        AdmissionDecision::Admitted { .. }
    ));
    let snapshot = g.snapshot();
    assert_eq!(snapshot.classes[&TaskClass::new("worker")].inflight, 1);
    assert_eq!(snapshot.conservation_violation(), None);
}

#[test]
fn a_permit_handle_from_another_governor_frees_nothing_here() {
    // Ids are per governor and both start at 1, so a handle from `other` is
    // numerically identical to a live permit in `g`. Presenting it to `g` is a
    // caller bug: it must be reported as unknown... except it cannot be told
    // apart by value. What *is* guaranteed: the other governor's own release
    // never touches `g`, and `g`'s accounting only moves for `g`'s releases.
    let (g, _) = governor(4);
    let (other, _) = governor(4);
    let mine = admit(&g, &root("mine"));
    let theirs = admit(&other, &root("theirs"));
    assert_eq!(mine, theirs, "the collision this test is about");

    // Releasing `theirs` on its own governor leaves `g` untouched.
    assert_eq!(other.release(theirs), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().classes[&TaskClass::new("worker")].inflight, 1);
    assert_eq!(
        other.snapshot().classes[&TaskClass::new("worker")].inflight,
        0
    );
    // A second presentation of the foreign handle to its own governor is
    // reported, not absorbed.
    assert_eq!(other.release(theirs), ReleaseOutcome::UnknownPermit);
    // And `g` still owns exactly its own permit.
    assert_eq!(g.release(mine), ReleaseOutcome::Released);
    assert_eq!(g.release(mine), ReleaseOutcome::UnknownPermit);
    assert_eq!(g.snapshot().conservation_violation(), None);
    assert_eq!(other.snapshot().conservation_violation(), None);
}
