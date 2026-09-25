//! H26: a queued multi-pool head fences overlapping followers, but an
//! independent pool may still make progress. Direct admission owns the whole
//! declared requirement set and charges it atomically on promotion.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ManualClock, OverflowPolicy, ResourceBudget, SubstrateHint, TaskClass, TaskSpec,
    TaskStage,
};
use taskmesh_engine::{AdmissionDecision, ClaimOutcome, Governor, PolicySet, ReleaseOutcome};

fn class(name: &str) -> TaskClass {
    TaskClass::new(name.to_owned())
}

fn governor() -> Governor {
    let classes = BTreeMap::from([
        (
            class("c"),
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(8)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        ),
        (class("holder"), ClassPolicy::new().max_inflight(1)),
    ]);
    let policy = PolicySet::new(ResourceBudget::new().cpu_units(8).memory_units(8), classes)
        .with_capability_limits(BTreeMap::from([
            ("large_stack".to_owned(), 1),
            ("blocking".to_owned(), 1),
            ("maintenance".to_owned(), 1),
        ]))
        .expect("finite built-in capability authorities");
    Governor::new(policy, Arc::new(ManualClock::new(0))).expect("valid policy")
}

#[test]
fn multi_pool_head_fences_shared_b_but_independent_c_makes_progress() {
    let g = governor();
    let a_holder =
        TaskSpec::base(class("holder"), SubstrateHint::LargeStackCapability).operation("hold-a");
    let AdmissionDecision::Admitted { permit_id: a } = g.admit(&a_holder) else {
        panic!("pool A holder must admit");
    };

    // The direct plan needs B now and A at a later stage; the engine reserves
    // both, unlike the host's first-dispatch-only contract.
    let head = TaskSpec::blocking(class("c")).operation("head-a-b").stage(
        TaskStage::new("later-a"),
        SubstrateHint::LargeStackCapability,
    );
    let AdmissionDecision::Queued {
        ticket: head_ticket,
    } = g.admit(&head)
    else {
        panic!("the head must wait for occupied A");
    };

    let shared_b = TaskSpec::blocking(class("c")).operation("follower-b");
    let AdmissionDecision::Queued { ticket: b_ticket } = g.admit(&shared_b) else {
        panic!("B-overlapping follower must not bypass the earlier runnable head");
    };
    let independent_c =
        TaskSpec::base(class("c"), SubstrateHint::BackgroundOnly).operation("follower-c");
    let AdmissionDecision::Admitted { permit_id: c } = g.admit(&independent_c) else {
        panic!("unrelated C pool may progress while A is occupied");
    };

    let snapshot = g.snapshot();
    assert_eq!(
        (
            snapshot.classes[&class("c")].inflight,
            snapshot.classes[&class("c")].queued
        ),
        (1, 2)
    );
    assert_eq!(snapshot.capabilities["large_stack"].in_use, 1);
    assert_eq!(snapshot.capabilities["blocking"].in_use, 0);
    assert_eq!(snapshot.capabilities["maintenance"].in_use, 1);
    assert!(matches!(
        g.ticket_status(head_ticket),
        ClaimOutcome::Pending
    ));
    assert!(matches!(g.ticket_status(b_ticket), ClaimOutcome::Pending));

    assert_eq!(g.release(a), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(head_permit) = g.claim(head_ticket) else {
        panic!("A release must promote the earlier A+B head");
    };
    assert!(matches!(g.ticket_status(b_ticket), ClaimOutcome::Pending));
    let promoted = g.snapshot();
    assert_eq!(promoted.capabilities["large_stack"].in_use, 1);
    assert_eq!(promoted.capabilities["blocking"].in_use, 1);
    assert_eq!(promoted.capabilities["maintenance"].in_use, 1);
    assert_eq!(
        g.permit_ledger(head_permit)
            .expect("head permit remains live")
            .capabilities
            .iter()
            .count(),
        2,
        "one head owns both distinct required pools exactly once"
    );

    assert_eq!(g.release(head_permit), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(b) = g.claim(b_ticket) else {
        panic!("B follower must promote after the head returns B");
    };
    let after_head = g.snapshot();
    assert_eq!(after_head.capabilities["large_stack"].in_use, 0);
    assert_eq!(after_head.capabilities["blocking"].in_use, 1);
    assert_eq!(after_head.capabilities["maintenance"].in_use, 1);

    assert_eq!(g.release(b), ReleaseOutcome::Released);
    assert_eq!(g.release(c), ReleaseOutcome::Released);
    let drained = g.snapshot();
    assert_eq!(
        (
            drained.classes[&class("c")].inflight,
            drained.classes[&class("c")].queued
        ),
        (0, 0)
    );
    for pool in ["large_stack", "blocking", "maintenance"] {
        assert_eq!(drained.capabilities[pool].in_use, 0, "{pool} refunded");
    }
    assert_eq!(drained.conservation_violation(), None);
}
