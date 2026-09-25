use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ExecutionPhase, ManualClock, MemoryReleasePolicy, OverflowPolicy, PermitWaker,
    ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AbandonOutcome, AdmissionDecision, ClaimOutcome, Governor, PolicySet, ReleaseOutcome,
    TerminalReason,
};

#[derive(Default)]
struct WakeCount(AtomicUsize);

impl PermitWaker for WakeCount {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn spec(operation: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation(operation.to_owned())
}

#[test]
fn release_promotion_reap_claim_and_abandon_form_one_finite_history() {
    let clock = Arc::new(ManualClock::new(1_000));
    let governor = Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(1),
            BTreeMap::from([(
                TaskClass::new("c"),
                ClassPolicy::new()
                    .max_inflight(1)
                    .max_queue_depth(4)
                    .cpu_units(1)
                    .overflow_policy(OverflowPolicy::QueueWithinDepth)
                    .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
            )]),
        ),
        clock.clone(),
    )
    .expect("valid policy");

    let holder = match governor.admit(&spec("holder")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("holder must admit: {other:?}"),
    };
    let taskmesh_engine::AdvanceOutcome::Leased(holder_lease) =
        governor.advance_phase(holder, ExecutionPhase::Running)
    else {
        panic!("holder must become an active lease")
    };

    let first_wakes = Arc::new(WakeCount::default());
    let first = match governor.admit_waitable(&spec("first"), first_wakes.clone()) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("first must queue: {other:?}"),
    };
    let second_wakes = Arc::new(WakeCount::default());
    let second = match governor.admit_waitable(&spec("second"), second_wakes.clone()) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("second must queue: {other:?}"),
    };

    let before_clock = governor.snapshot();
    clock.advance(100);
    assert_eq!(governor.ticket_status(first), ClaimOutcome::Pending);
    assert_eq!(governor.ticket_status(second), ClaimOutcome::Pending);
    assert_eq!(
        governor.snapshot(),
        before_clock,
        "clock advance and ticket reads alone cannot transition the ledger"
    );
    let retained = governor.reap_leaks_with(10);
    assert_eq!(
        (retained.reclaimed_permits, retained.retained_active),
        (0, 1)
    );
    assert_eq!(governor.ticket_status(first), ClaimOutcome::Pending);
    assert_eq!(governor.ticket_status(second), ClaimOutcome::Pending);
    assert_eq!(
        (
            first_wakes.0.load(Ordering::SeqCst),
            second_wakes.0.load(Ordering::SeqCst)
        ),
        (0, 0)
    );

    assert_eq!(
        governor.release_leased(holder_lease),
        ReleaseOutcome::Released
    );
    assert!(matches!(
        governor.ticket_status(first),
        ClaimOutcome::Ready(_)
    ));
    assert_eq!(governor.ticket_status(second), ClaimOutcome::Pending);
    assert_eq!(
        (
            first_wakes.0.load(Ordering::SeqCst),
            second_wakes.0.load(Ordering::SeqCst)
        ),
        (1, 0)
    );

    let before_second_clock = governor.snapshot();
    clock.advance(100);
    assert!(matches!(
        governor.ticket_status(first),
        ClaimOutcome::Ready(_)
    ));
    assert_eq!(
        governor.snapshot(),
        before_second_clock,
        "the promoted ticket stays ready until a governor transition"
    );
    let reaped = governor.reap_leaks_with(10);
    assert_eq!((reaped.reclaimed_permits, reaped.retained_active), (1, 0));
    assert_eq!(
        governor.claim(first),
        ClaimOutcome::Terminal(TerminalReason::Reclaimed)
    );
    assert_eq!(governor.claim(first), ClaimOutcome::Invalid);
    assert!(matches!(
        governor.ticket_status(second),
        ClaimOutcome::Ready(_)
    ));
    assert_eq!(second_wakes.0.load(Ordering::SeqCst), 1);

    assert_eq!(governor.abandon(second), AbandonOutcome::Abandoned);
    assert_eq!(governor.claim(second), ClaimOutcome::Invalid);
    let final_state = governor.snapshot();
    assert_eq!(
        (
            final_state.classes[&TaskClass::new("c")].inflight,
            final_state.classes[&TaskClass::new("c")].queued
        ),
        (0, 0)
    );
    assert_eq!(final_state.conservation_violation(), None);
}
