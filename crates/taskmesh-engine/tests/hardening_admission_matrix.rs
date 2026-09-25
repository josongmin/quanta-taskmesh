use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ManualClock, MemoryOvercommitPolicy, OverflowPolicy,
    ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, CapacityAssessment, CapacityBlock, ClaimOutcome, Governor, PermitId,
    PolicySet, ReleaseOutcome,
};

fn spec(class: &str, operation: &str, blocking: bool) -> TaskSpec {
    if blocking {
        TaskSpec::blocking(TaskClass::new(class.to_owned())).operation(operation.to_owned())
    } else {
        TaskSpec::io(TaskClass::new(class.to_owned())).operation(operation.to_owned())
    }
}

fn admitted(decision: AdmissionDecision) -> PermitId {
    match decision {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

#[test]
fn compound_blockers_have_one_stable_primary_and_zero_rejected_side_effects() {
    let queueing = ClassPolicy::new()
        .max_inflight(1)
        .max_queue_depth(4)
        .cpu_units(2)
        .memory_units(2)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .memory_overcommit_policy(MemoryOvercommitPolicy::Queue);
    let rejecting = ClassPolicy::new()
        .max_inflight(4)
        .cpu_units(1)
        .memory_units(1)
        .overflow_policy(OverflowPolicy::Reject);
    let policy = PolicySet::new(
        ResourceBudget::new().cpu_units(2).memory_units(2),
        BTreeMap::from([
            (TaskClass::new("queueing"), queueing),
            (TaskClass::new("rejecting"), rejecting),
        ]),
    )
    .with_capability_limits(BTreeMap::from([("blocking".to_owned(), 1)]))
    .expect("built-in blocking authority");
    let governor = Governor::new(policy, Arc::new(ManualClock::new(10))).expect("valid policy");

    let holder = admitted(governor.admit(&spec("queueing", "holder", true)));
    let ticket = match governor.admit(&spec("queueing", "all-blocked", true)) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("compound pressure must queue: {other:?}"),
    };
    let Some(CapacityAssessment::ReversiblyBlocked(blockers)) = governor.pending_assessment(ticket)
    else {
        panic!("queued request must expose all current blockers")
    };
    assert!(blockers.class_inflight);
    assert_eq!(blockers.capabilities.len(), 1);
    assert!(blockers.cpu);
    assert!(blockers.memory);
    assert_eq!(blockers.primary(), CapacityBlock::Inflight);
    assert_eq!(
        governor.pending_block_reason(ticket),
        Some(CapacityBlock::Inflight)
    );

    let before = governor.snapshot();
    assert!(matches!(
        governor.admit(&spec("rejecting", "rejected", true)),
        AdmissionDecision::Rejected(AdmissionVerdict::SubstrateSaturated { .. })
    ));
    assert_eq!(governor.snapshot(), before);

    assert_eq!(governor.release(holder), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(promoted) = governor.claim(ticket) else {
        panic!("release must promote the queued request")
    };
    let snapshot = governor.snapshot();
    let queueing = &snapshot.classes[&TaskClass::new("queueing")];
    assert_eq!((queueing.inflight, queueing.queued), (1, 0));
    assert_eq!(
        (queueing.cpu_units_held, queueing.memory_units_held),
        (2, 2)
    );
    assert_eq!(snapshot.capabilities["blocking"].in_use, 1);
    assert_eq!(governor.release(promoted), ReleaseOutcome::Released);
}

#[test]
fn class_primary_then_memory_primary_follow_their_distinct_queue_policies() {
    let candidate = ClassPolicy::new()
        .max_inflight(1)
        .max_queue_depth(2)
        .memory_units(4)
        .overflow_policy(OverflowPolicy::Reject)
        .memory_overcommit_policy(MemoryOvercommitPolicy::Queue);
    let pressure = ClassPolicy::new().max_inflight(1).memory_units(4);
    let governor = Governor::new(
        PolicySet::new(
            ResourceBudget::new().memory_units(4),
            BTreeMap::from([
                (TaskClass::new("candidate"), candidate),
                (TaskClass::new("pressure"), pressure),
            ]),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");

    let own = admitted(governor.admit(&spec("candidate", "own", false)));
    assert!(matches!(
        governor.admit(&spec("candidate", "class-and-memory", false)),
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
    ));
    assert_eq!(governor.release(own), ReleaseOutcome::Released);

    let pressure = admitted(governor.admit(&spec("pressure", "pressure", false)));
    let ticket = match governor.admit(&spec("candidate", "memory-only", false)) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("memory-primary policy must queue: {other:?}"),
    };
    assert_eq!(
        governor.pending_block_reason(ticket),
        Some(CapacityBlock::Memory)
    );
    assert_eq!(governor.release(pressure), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(promoted) = governor.claim(ticket) else {
        panic!("memory release must promote")
    };
    assert_eq!(governor.release(promoted), ReleaseOutcome::Released);
}

#[test]
fn capacity_zero_one_exact_and_plus_one_are_unambiguous() {
    for (budget, cost, expected) in [
        (0, u32::MAX, true),
        (1, 1, true),
        (4, 4, true),
        (4, 5, false),
    ] {
        let governor = Governor::new(
            PolicySet::new(
                ResourceBudget::new().cpu_units(budget),
                BTreeMap::from([(
                    TaskClass::new("c"),
                    ClassPolicy::new().max_inflight(1).cpu_units(cost),
                )]),
            ),
            Arc::new(ManualClock::new(0)),
        );
        if expected {
            let governor = governor.expect("representable policy");
            let permit = admitted(governor.admit(&spec("c", "boundary", false)));
            assert_eq!(governor.release(permit), ReleaseOutcome::Released);
        } else {
            let Err(taskmesh_contract::GovernorError::PolicyViolation(message)) = governor else {
                panic!("per-request cost above the hard budget must be a policy violation");
            };
            assert!(
                message.contains("exceeds global cpu budget"),
                "the failure must name the exceeded hard budget: {message}"
            );
        }
    }
}
