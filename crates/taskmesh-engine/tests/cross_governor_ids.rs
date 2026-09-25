//! BG25-004/B28: cross-Governor identities retain local sequence telemetry
//! without granting authority over another Governor's state.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ExecutionPhase, ManualClock, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AbandonOutcome, AdmissionDecision, AdvanceOutcome, AdvanceRefusal, ClaimOutcome, Governor,
    PermitId, PolicySet, ReleaseOutcome, Ticket,
};

fn class() -> TaskClass {
    TaskClass::new("c")
}

fn governor() -> Governor {
    let classes = BTreeMap::from([(
        class(),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(4)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    )]);
    Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(8).memory_units(8), classes),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy")
}

fn admit(governor: &Governor, operation: &str) -> PermitId {
    match governor.admit(&TaskSpec::blocking(class()).operation(operation)) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn queue(governor: &Governor, operation: &str) -> Ticket {
    match governor.admit(&TaskSpec::blocking(class()).operation(operation)) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected queued ticket, got {other:?}"),
    }
}

fn same_sequence_permits() -> (Governor, Governor, PermitId, PermitId) {
    let left = governor();
    let right = governor();
    let on_left = admit(&left, "left-holder");
    let on_right = admit(&right, "right-holder");
    assert_eq!(on_left.sequence(), on_right.sequence());
    assert_ne!(
        on_left, on_right,
        "authority must distinguish equal sequences"
    );
    (left, right, on_left, on_right)
}

fn same_sequence_tickets() -> (Governor, Governor, PermitId, PermitId, Ticket, Ticket) {
    let (left, right, on_left, on_right) = same_sequence_permits();
    let ticket_left = queue(&left, "left-waiter");
    let ticket_right = queue(&right, "right-waiter");
    assert_eq!(ticket_left.sequence(), ticket_right.sequence());
    assert_ne!(
        ticket_left, ticket_right,
        "authority must distinguish equal sequences"
    );
    (left, right, on_left, on_right, ticket_left, ticket_right)
}

#[test]
fn foreign_release_cannot_refund_a_local_same_sequence_permit() {
    let (left, right, on_left, on_right) = same_sequence_permits();
    let before = right.snapshot();

    assert_eq!(right.release(on_left), ReleaseOutcome::UnknownPermit);
    assert_eq!(right.snapshot(), before);
    assert_eq!(left.release(on_left), ReleaseOutcome::Released);
    assert_eq!(right.release(on_right), ReleaseOutcome::Released);
}

#[test]
fn foreign_advance_cannot_lease_a_local_same_sequence_permit() {
    let (left, right, on_left, on_right) = same_sequence_permits();
    let before = right.snapshot();

    assert_eq!(
        right.advance_phase(on_left, ExecutionPhase::Accepted),
        AdvanceOutcome::Refused(AdvanceRefusal::UnknownPermit)
    );
    assert_eq!(right.snapshot(), before);
    assert_eq!(
        right.phase(on_right),
        Some(ExecutionPhase::DispatchReserved)
    );
    assert_eq!(left.release(on_left), ReleaseOutcome::Released);
    assert_eq!(right.release(on_right), ReleaseOutcome::Released);
}

#[test]
fn foreign_claim_cannot_transfer_a_local_same_sequence_ticket() {
    let (left, right, on_left, on_right, ticket_left, ticket_right) = same_sequence_tickets();
    let before = right.snapshot();

    assert_eq!(right.claim(ticket_left), ClaimOutcome::Invalid);
    assert_eq!(right.snapshot(), before);
    assert_eq!(right.ticket_status(ticket_right), ClaimOutcome::Pending);
    assert_eq!(left.abandon(ticket_left), AbandonOutcome::Abandoned);
    assert_eq!(right.abandon(ticket_right), AbandonOutcome::Abandoned);
    assert_eq!(left.release(on_left), ReleaseOutcome::Released);
    assert_eq!(right.release(on_right), ReleaseOutcome::Released);
}

#[test]
fn foreign_abandon_cannot_remove_a_local_same_sequence_ticket() {
    let (left, right, on_left, on_right, ticket_left, ticket_right) = same_sequence_tickets();
    let before = right.snapshot();

    assert_eq!(right.abandon(ticket_left), AbandonOutcome::Invalid);
    assert_eq!(right.snapshot(), before);
    assert_eq!(right.ticket_status(ticket_right), ClaimOutcome::Pending);
    assert_eq!(left.abandon(ticket_left), AbandonOutcome::Abandoned);
    assert_eq!(right.abandon(ticket_right), AbandonOutcome::Abandoned);
    assert_eq!(left.release(on_left), ReleaseOutcome::Released);
    assert_eq!(right.release(on_right), ReleaseOutcome::Released);
}
