use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ExecutionPhase, ManualClock, MemoryOvercommitPolicy, MemoryPermitMode,
    MemoryReleasePolicy, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PolicySet, ReleaseOutcome, StageReleaseOutcome,
};

fn held(governor: &Governor) -> u128 {
    governor
        .snapshot()
        .classes
        .values()
        .map(|class| class.memory_units_held)
        .sum()
}

fn spec(class: &str, operation: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new(class.to_owned())).operation(operation.to_owned())
}

#[test]
fn stage_reconcile_promotion_and_sweep_match_an_input_derived_ledger() {
    let clock = Arc::new(ManualClock::new(1_000));
    let governor = Governor::new(
        PolicySet::new(
            ResourceBudget::new().memory_units(10).memory_unit_scale(10),
            BTreeMap::from([
                (
                    TaskClass::new("c"),
                    ClassPolicy::new()
                        .max_inflight(2)
                        .max_queue_depth(2)
                        .memory_units(6)
                        .memory_permit_mode(MemoryPermitMode::Hybrid)
                        .memory_release_policy(MemoryReleasePolicy::OnStageBoundary)
                        .memory_overcommit_policy(MemoryOvercommitPolicy::Queue)
                        .overflow_policy(OverflowPolicy::QueueWithinDepth),
                ),
                (
                    TaskClass::new("leak"),
                    ClassPolicy::new()
                        .max_inflight(2)
                        .memory_units(1)
                        .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
                ),
            ]),
        ),
        clock.clone(),
    )
    .expect("valid policy");

    let first = match governor.admit(&spec("c", "first")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("first must admit: {other:?}"),
    };
    let second = match governor.admit(&spec("c", "second")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("second must wait for memory: {other:?}"),
    };
    let mut expected_first = 6_u128;
    let mut expected_second = 0_u128;
    assert_eq!(held(&governor), expected_first + expected_second);

    assert_eq!(
        governor.release_stage_memory(first, 4),
        StageReleaseOutcome::Released { freed_units: 4 }
    );
    expected_first = 2;
    expected_second = 6;
    assert_eq!(held(&governor), expected_first + expected_second);
    let ClaimOutcome::Ready(second_permit) = governor.claim(second) else {
        panic!("stage return must promote the waiter")
    };

    assert!(governor.reconcile_memory(first, 90).is_applied());
    expected_first = 9;
    assert_eq!(held(&governor), expected_first + expected_second);
    let blocked = match governor.admit(&spec("c", "blocked-by-measurement")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("measured overcommit must queue: {other:?}"),
    };

    let taskmesh_engine::AdvanceOutcome::Leased(second_lease) =
        governor.advance_phase(second_permit, ExecutionPhase::Running)
    else {
        panic!("second permit must become active")
    };
    assert_eq!(governor.release(first), ReleaseOutcome::Released);
    expected_first = 0;
    assert_eq!(held(&governor), expected_first + expected_second);

    assert_eq!(
        governor.release_leased(second_lease),
        ReleaseOutcome::Released
    );
    expected_second = 0;
    let ClaimOutcome::Ready(third) = governor.claim(blocked) else {
        panic!("release must promote the measurement-blocked waiter")
    };
    assert_eq!(held(&governor), 6);
    assert_eq!(governor.release(third), ReleaseOutcome::Released);
    assert_eq!(held(&governor), expected_first + expected_second);

    let stale = match governor.admit(&spec("leak", "stale")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("stale control must admit: {other:?}"),
    };
    let active = match governor.admit(&spec("leak", "active")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("active control must admit: {other:?}"),
    };
    let taskmesh_engine::AdvanceOutcome::Leased(active_lease) =
        governor.advance_phase(active, ExecutionPhase::Running)
    else {
        panic!("active control must lease")
    };
    assert_eq!(held(&governor), 2);
    clock.advance(100);
    let report = governor.reap_leaks_with(10);
    assert_eq!((report.reclaimed_permits, report.retained_active), (1, 1));
    assert_eq!(governor.phase(stale), None);
    assert_eq!(held(&governor), 1);
    assert_eq!(
        governor.release_leased(active_lease),
        ReleaseOutcome::Released
    );
    assert_eq!(held(&governor), 0);
    assert_eq!(governor.snapshot().conservation_violation(), None);
}
