//! BG25-004/B28: executable counterexamples for raw cross-Governor IDs.
//!
//! These tests pin the *current unsafe authority boundary*, not the desired
//! contract. `PermitId` and `Ticket` are numeric aliases. Two fresh Governors
//! issue the same values, so a foreign value can act on local state. Replace
//! these assertions with foreign-handle rejection when D5's public migration
//! is accepted; do not mistake a green result here for a fix.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ExecutionPhase, ManualClock, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, AdvanceOutcome, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome,
    Ticket,
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

fn admit(g: &Governor, operation: &str) -> PermitId {
    match g.admit(&TaskSpec::blocking(class()).operation(operation)) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn queue(g: &Governor, operation: &str) -> Ticket {
    match g.admit(&TaskSpec::blocking(class()).operation(operation)) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected queued ticket, got {other:?}"),
    }
}

fn colliding_permits() -> (Governor, Governor, PermitId, PermitId) {
    let a = governor();
    let b = governor();
    let on_a = admit(&a, "a-holder");
    let on_b = admit(&b, "b-holder");
    assert_eq!(on_a, on_b, "fresh Governors reuse the same raw permit ID");
    (a, b, on_a, on_b)
}

fn colliding_tickets() -> (Governor, Governor, PermitId, PermitId, Ticket, Ticket) {
    let (a, b, on_a, on_b) = colliding_permits();
    let ticket_a = queue(&a, "a-waiter");
    let ticket_b = queue(&b, "b-waiter");
    assert_eq!(
        ticket_a, ticket_b,
        "fresh Governors reuse the same raw ticket"
    );
    (a, b, on_a, on_b, ticket_a, ticket_b)
}

#[test]
fn foreign_raw_release_currently_refunds_the_local_colliding_permit() {
    let (a, b, on_a, on_b) = colliding_permits();
    assert_eq!(b.release(on_a), ReleaseOutcome::Released);
    assert_eq!(b.snapshot().classes[&class()].inflight, 0);
    assert_eq!(a.snapshot().classes[&class()].inflight, 1);
    assert_eq!(a.release(on_a), ReleaseOutcome::Released);
    assert_eq!(b.release(on_b), ReleaseOutcome::UnknownPermit);
}

#[test]
fn foreign_raw_advance_currently_leases_the_local_colliding_permit() {
    let (a, b, on_a, on_b) = colliding_permits();
    let AdvanceOutcome::Leased(local_lease) = b.advance_phase(on_a, ExecutionPhase::Accepted)
    else {
        panic!("foreign raw ID should expose the current local effect");
    };
    assert_eq!(b.phase(on_b), Some(ExecutionPhase::Accepted));
    assert_eq!(a.phase(on_a), Some(ExecutionPhase::DispatchReserved));
    assert_eq!(b.release_leased(local_lease), ReleaseOutcome::Released);
    assert_eq!(a.release(on_a), ReleaseOutcome::Released);
}

#[test]
fn foreign_raw_claim_currently_transfers_the_local_colliding_ticket() {
    let (a, b, on_a, on_b, ticket_a, ticket_b) = colliding_tickets();
    assert_eq!(b.release(on_b), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(on_b_waiter) = b.claim(ticket_a) else {
        panic!("foreign raw ticket should claim the local promotion");
    };
    assert_eq!(b.ticket_status(ticket_b), ClaimOutcome::Invalid);
    assert_eq!(a.ticket_status(ticket_a), ClaimOutcome::Pending);
    assert_eq!(b.release(on_b_waiter), ReleaseOutcome::Released);
    a.abandon(ticket_a);
    assert_eq!(a.release(on_a), ReleaseOutcome::Released);
}

#[test]
fn foreign_raw_abandon_currently_removes_the_local_colliding_ticket() {
    let (a, b, on_a, on_b, ticket_a, ticket_b) = colliding_tickets();
    b.abandon(ticket_a);
    assert_eq!(b.ticket_status(ticket_b), ClaimOutcome::Invalid);
    assert_eq!(b.snapshot().classes[&class()].queued, 0);
    assert_eq!(a.ticket_status(ticket_a), ClaimOutcome::Pending);
    a.abandon(ticket_a);
    assert_eq!(a.release(on_a), ReleaseOutcome::Released);
    assert_eq!(b.release(on_b), ReleaseOutcome::Released);
}
