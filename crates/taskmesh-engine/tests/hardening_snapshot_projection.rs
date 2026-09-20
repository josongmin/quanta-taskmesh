//! Observability regressions (H16-013): the snapshot is a *projection* of
//! authoritative state, not a second ledger kept alongside it.
//!
//! Two properties matter and are checked independently of the counters the
//! engine maintains:
//!
//! 1. **Conservation.** `inflight` partitions exactly into the ownership phases,
//!    and `admitted_total = inflight + terminated_total`. A gauge that does not
//!    close is a gauge that is hiding something.
//! 2. **A caller's answer is not a phase change.** A timed-out or cancelled
//!    caller has its result while the work is still `Running` or
//!    `CleanupPending`. If the response moved the gauge, capacity would read as
//!    free while it was still in use.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ExecutionPhase, ManualClock, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, AdvanceOutcome, AdvanceRefusal, ClaimOutcome, Governor, LeaseToken,
    PermitId, PolicySet, ReleaseOutcome,
};

/// The first advance out of `DispatchReserved` hands over the permit's lease.
fn lease(g: &Governor, permit: PermitId, phase: ExecutionPhase) -> LeaseToken {
    match g.advance_phase(permit, phase) {
        AdvanceOutcome::Leased(token) => token,
        other => panic!("the first advance of permit {permit} must lease it, got {other:?}"),
    }
}

fn gov() -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(2)
            .max_queue_depth(8)
            .cpu_units(1)
            .memory_units(2)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(100).memory_units(100),
            classes,
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy")
}

fn spec(op: &str) -> TaskSpec {
    TaskSpec::io(TaskClass::new("c")).operation(op.to_string())
}

fn admit(g: &Governor, op: &str) -> PermitId {
    match g.admit(&spec(op)) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

/// Recompute every gauge from the per-permit ledgers and compare. The engine
/// maintains these incrementally; this derives them from scratch, so a drift in
/// the incremental path cannot hide behind itself.
fn assert_projection_matches_records(g: &Governor, what: &str) {
    let ledgers = g.permit_ledgers();
    let snapshot = g.snapshot();
    assert_eq!(snapshot.conservation_violation(), None, "{what}");

    let class = TaskClass::new("c");
    let observed = &snapshot.classes[&class];
    let mut phases = [0u32; 4];
    let mut cpu: u128 = 0;
    let mut memory: u128 = 0;
    for ledger in &ledgers {
        let slot = match ledger.phase {
            ExecutionPhase::DispatchReserved => 0,
            ExecutionPhase::Accepted => 1,
            ExecutionPhase::Running => 2,
            _ => 3,
        };
        phases[slot] += 1;
        cpu += u128::from(ledger.cpu_units);
        memory += u128::from(ledger.effective_units);
    }
    assert_eq!(observed.dispatch_reserved, phases[0], "{what}: reserved");
    assert_eq!(observed.accepted, phases[1], "{what}: accepted");
    assert_eq!(observed.running, phases[2], "{what}: running");
    assert_eq!(observed.cleanup_pending, phases[3], "{what}: cleanup");
    assert_eq!(
        observed.inflight,
        u32::try_from(ledgers.len()).expect("fits"),
        "{what}: inflight"
    );
    assert_eq!(observed.cpu_units_held, cpu, "{what}: cpu");
    assert_eq!(observed.memory_units_held, memory, "{what}: memory");
}

#[test]
fn every_transition_leaves_the_projection_consistent() {
    let g = gov();
    assert_projection_matches_records(&g, "empty");

    let first = admit(&g, "first");
    assert_projection_matches_records(&g, "after admit");

    let first_lease = lease(&g, first, ExecutionPhase::Accepted);
    assert_projection_matches_records(&g, "after phase advance");
    for phase in [ExecutionPhase::Running, ExecutionPhase::CleanupPending] {
        assert_eq!(g.advance_phase(first, phase), AdvanceOutcome::Advanced);
        assert_projection_matches_records(&g, "after phase advance");
    }

    let second = admit(&g, "second");
    assert_projection_matches_records(&g, "second admit");

    let ticket = match g.admit(&spec("queued")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    };
    assert_eq!(g.snapshot().classes[&TaskClass::new("c")].queued, 1);
    assert_projection_matches_records(&g, "queued");

    assert_eq!(g.release_leased(first_lease), ReleaseOutcome::Released);
    assert_projection_matches_records(&g, "after release + promotion");
    let ClaimOutcome::Ready(promoted) = g.claim(ticket) else {
        panic!("the queued request must have been promoted");
    };

    assert_eq!(g.release(second), ReleaseOutcome::Released);
    assert_eq!(g.release(promoted), ReleaseOutcome::Released);
    assert_projection_matches_records(&g, "drained");

    let final_snapshot = g.snapshot();
    let observed = &final_snapshot.classes[&TaskClass::new("c")];
    assert_eq!(observed.inflight, 0);
    assert_eq!(observed.admitted_total, 3);
    assert_eq!(observed.terminated_total, 3);
    assert_eq!(observed.started_total, 1, "only the first reached Running");
}

#[test]
fn phases_are_monotonic_and_partition_inflight() {
    let g = gov();
    let permit = admit(&g, "op");
    assert_eq!(g.phase(permit), Some(ExecutionPhase::DispatchReserved));

    let token = lease(&g, permit, ExecutionPhase::Running);
    // Backwards and repeated declarations change nothing — an adapter that
    // reports an event twice must not double-count it — and say why.
    for phase in [ExecutionPhase::Accepted, ExecutionPhase::Running] {
        assert_eq!(
            g.advance_phase(permit, phase),
            AdvanceOutcome::Refused(AdvanceRefusal::NotLater {
                current: ExecutionPhase::Running
            })
        );
    }
    assert_eq!(g.phase(permit), Some(ExecutionPhase::Running));

    let observed = &g.snapshot().classes[&TaskClass::new("c")];
    assert_eq!(observed.running, 1);
    assert_eq!(observed.accepted, 0);
    assert_eq!(observed.dispatch_reserved, 0);
    assert_eq!(observed.started_total, 1, "counted once, not twice");
    assert_projection_matches_records(&g, "monotonic");

    // An unknown permit cannot be advanced.
    assert_eq!(
        g.advance_phase(u64::MAX, ExecutionPhase::Running),
        AdvanceOutcome::Refused(AdvanceRefusal::UnknownPermit)
    );
    assert_eq!(g.release_leased(token), ReleaseOutcome::Released);
}

#[test]
fn accepted_work_is_visible_rather_than_hidden_as_pending() {
    // An adapter that has taken custody but not started is its own phase. Rolling
    // it into "pending" would understate live commitments; rolling it into
    // "running" would overstate them.
    let g = gov();
    let permit = admit(&g, "op");
    let token = lease(&g, permit, ExecutionPhase::Accepted);

    let observed = &g.snapshot().classes[&TaskClass::new("c")];
    assert_eq!(observed.accepted, 1);
    assert_eq!(observed.inflight, 1, "accepted work is inflight");
    assert_eq!(observed.queued, 0, "and it is not queued");
    assert_eq!(
        observed.started_total, 0,
        "accepted is not started: the adapter has it, the work has not begun"
    );
    assert_projection_matches_records(&g, "accepted");
    assert_eq!(g.release_leased(token), ReleaseOutcome::Released);
}

#[test]
fn terminated_counters_survive_the_records_they_counted() {
    // Retired permits leave the live gauges and stay in the cumulative counters:
    // an operator asking "how much has this class run?" is asking a different
    // question from "what is it running now?".
    let g = gov();
    for round in 0..50 {
        let permit = admit(&g, &format!("op{round}"));
        let token = lease(&g, permit, ExecutionPhase::Running);
        assert_eq!(g.release_leased(token), ReleaseOutcome::Released);
    }
    let observed = &g.snapshot().classes[&TaskClass::new("c")];
    assert_eq!(observed.inflight, 0);
    assert_eq!(observed.admitted_total, 50);
    assert_eq!(observed.started_total, 50);
    assert_eq!(observed.terminated_total, 50);
    assert_eq!(observed.conservation_violation(), None);
}

#[test]
fn capability_occupancy_is_reported_against_its_limit() {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(8)
            .max_queue_depth(8)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    let g = Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(100), classes)
            .with_capability_limits(BTreeMap::from([("blocking".to_string(), 2)]))
            .expect("blocking is a registered pool"),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy");

    let first = match g.admit(&TaskSpec::blocking(TaskClass::new("c")).operation("op-1")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("{other:?}"),
    };
    let snapshot = g.snapshot();
    assert_eq!(snapshot.capabilities["blocking"].in_use, 1);
    assert_eq!(snapshot.capabilities["blocking"].limit, 2);
    assert_eq!(snapshot.conservation_violation(), None);

    let second = match g.admit(&TaskSpec::blocking(TaskClass::new("c")).operation("op-2")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("{other:?}"),
    };
    assert_eq!(g.snapshot().capabilities["blocking"].in_use, 2);
    // The third has to wait for the pool, not for the class.
    let AdmissionDecision::Queued { ticket } =
        g.admit(&TaskSpec::blocking(TaskClass::new("c")).operation("op-3"))
    else {
        panic!("the third must queue on the pool");
    };
    assert_eq!(
        g.snapshot().capabilities["blocking"].in_use,
        2,
        "a queued request does not occupy the pool it is waiting for"
    );

    // Freeing one slot promotes exactly the one waiter: the pool stays full.
    assert_eq!(g.release(first), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(third) = g.claim(ticket) else {
        panic!("one freed slot promotes the waiter");
    };
    assert_eq!(g.snapshot().capabilities["blocking"].in_use, 2);
    // Freeing the second leaves exactly the promoted one.
    assert_eq!(g.release(second), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().capabilities["blocking"].in_use, 1);
    assert_eq!(g.release(third), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().capabilities["blocking"].in_use, 0);
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn queue_diagnostics_name_what_a_request_is_waiting_on() {
    // A timeout that cannot say whether the class quota or the worker pool was
    // the constraint sends an operator to the wrong dial.
    use taskmesh_engine::CapacityBlock;

    let g = gov();
    let _first = admit(&g, "first");
    let _second = admit(&g, "second");
    let ticket = match g.admit(&spec("blocked")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    };
    let view = g.pending_view(ticket).expect("the ticket is queued");
    assert_eq!(view.class, TaskClass::new("c"));
    assert_eq!(view.blocked_on, CapacityBlock::Inflight);
    assert_eq!(
        g.pending_block_reason(ticket),
        Some(CapacityBlock::Inflight)
    );

    // Once it stops being queued, there is no pending view to read.
    g.abandon(ticket);
    assert!(g.pending_view(ticket).is_none());
}
