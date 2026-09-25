use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ExecutionPhase, ManualClock, OverflowPolicy, PermitWaker,
    ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AbandonOutcome, AdmissionDecision, AdvanceOutcome, AdvanceRefusal, ClaimOutcome, Governor,
    PolicySet, ReleaseOutcome,
};

fn governor() -> Governor {
    let classes = BTreeMap::from([(
        TaskClass::new("work"),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(4)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    )]);
    Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(1).memory_units(100),
            classes,
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy")
}

fn spec(operation: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new("work")).operation(operation.to_owned())
}

fn permit(governor: &Governor, operation: &str) -> taskmesh_engine::PermitId {
    match governor.admit(&spec(operation)) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn ticket(governor: &Governor, operation: &str) -> taskmesh_engine::Ticket {
    match governor.admit(&spec(operation)) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected queueing, got {other:?}"),
    }
}

#[derive(Default)]
struct CountingWaker(AtomicUsize);

impl PermitWaker for CountingWaker {
    fn wake(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn foreign_handles_never_alias_same_sequence_local_state() {
    let left = governor();
    let right = governor();

    let left_permit = permit(&left, "left-holder");
    let right_permit = permit(&right, "right-holder");
    assert_eq!(left_permit.sequence(), right_permit.sequence());
    assert_ne!(left_permit, right_permit);

    let right_before = right.snapshot();
    assert_eq!(right.release(left_permit), ReleaseOutcome::UnknownPermit);
    assert_eq!(
        right.advance_phase(left_permit, ExecutionPhase::Accepted),
        AdvanceOutcome::Refused(AdvanceRefusal::UnknownPermit)
    );
    assert_eq!(right.snapshot(), right_before);

    let left_ticket = ticket(&left, "left-queued");
    let wakes = Arc::new(CountingWaker::default());
    let right_ticket = match right.admit_waitable(&spec("right-queued"), wakes.clone()) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected queueing, got {other:?}"),
    };
    assert_eq!(left_ticket.sequence(), right_ticket.sequence());
    assert_ne!(left_ticket, right_ticket);

    let right_before = right.snapshot();
    assert_eq!(right.claim(left_ticket), ClaimOutcome::Invalid);
    assert_eq!(right.abandon(left_ticket), AbandonOutcome::Invalid);
    assert_eq!(right.snapshot(), right_before);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    assert_eq!(right.ticket_status(right_ticket), ClaimOutcome::Pending);

    assert_eq!(right.release(right_permit), ReleaseOutcome::Released);
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    let ClaimOutcome::Ready(promoted) = right.claim(right_ticket) else {
        panic!("right ticket must retain local ownership")
    };
    assert_eq!(right.release(promoted), ReleaseOutcome::Released);

    assert_eq!(left.abandon(left_ticket), AbandonOutcome::Abandoned);
    assert_eq!(left.release(left_permit), ReleaseOutcome::Released);
}

#[test]
fn identity_counter_exhaustion_rejects_without_state_or_wake() {
    for counters in [(u64::MAX, 1, 1), (1, u64::MAX, 1), (1, 1, u64::MAX)] {
        let governor = governor();
        governor.set_identity_counters_for_test(counters.0, counters.1, counters.2);
        let before = governor.snapshot();
        let wakes = Arc::new(CountingWaker::default());

        assert_eq!(
            governor.admit_waitable(&spec("exhausted"), wakes.clone()),
            AdmissionDecision::Rejected(AdmissionVerdict::IdentityExhausted)
        );
        assert_eq!(governor.snapshot(), before);
        assert_eq!(wakes.0.load(Ordering::SeqCst), 0);

        assert_eq!(
            governor.admit(&spec("still-exhausted")),
            AdmissionDecision::Rejected(AdmissionVerdict::IdentityExhausted),
            "an exhausted counter must never wrap into a reusable identity"
        );
    }
}
