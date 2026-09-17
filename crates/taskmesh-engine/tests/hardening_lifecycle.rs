//! TM16-010 regression: a ticket and the permit it was promoted to die in the
//! same transition, and a waiter is always told which.
//!
//! The original defect: the leak sweep removed a permit from `permits` but left
//! the `granted[ticket] -> permit_id` mapping behind. In debug that tripped the
//! consistency assertion; in release `claim` handed the waiter a permit id with
//! nothing behind it, and the waiter then ran ungoverned work — no inflight, no
//! provenance, no accounting. Deleting the mapping alone would have traded that
//! for a waiter parked forever, because a missing mapping and "not promoted
//! yet" were the same `None`.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ManualClock, MemoryReleasePolicy, OverflowPolicy, ResourceBudget, TaskClass,
    TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome, TerminalReason,
};

fn gov(policy: ClassPolicy) -> (Governor, Arc<ManualClock>, TaskClass) {
    let class = TaskClass::new("c");
    let mut classes = BTreeMap::new();
    classes.insert(class.clone(), policy);
    let clock = Arc::new(ManualClock::new(1_000));
    let governor = Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(64).memory_units(64),
            classes,
        ),
        #[allow(
            clippy::clone_on_ref_ptr,
            reason = "Arc<ManualClock> -> Arc<dyn Clock>"
        )]
        clock.clone(),
    )
    .expect("valid policy");
    (governor, clock, class)
}

fn leak_detecting_single_slot() -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(1)
        .max_queue_depth(8)
        .cpu_units(1)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .memory_release_policy(MemoryReleasePolicy::LeakDetecting)
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

fn queue(g: &Governor, op: &str) -> u64 {
    match g.admit(&spec(op)) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected a queued ticket, got {other:?}"),
    }
}

#[test]
fn a_swept_promotion_hands_the_waiter_a_terminal_outcome_not_a_dead_permit() {
    let (g, clock, class) = gov(leak_detecting_single_slot());
    let holder = admit(&g, "holder");
    let ticket = queue(&g, "queued");

    // Releasing the holder promotes the queued request: it now owns a permit it
    // has not claimed.
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    assert!(matches!(g.ticket_status(ticket), ClaimOutcome::Ready(_)));

    // Let the promotion go stale and sweep it.
    clock.advance(10_000);
    let report = g.reap_leaks_with(10);
    assert_eq!(report.reclaimed_permits, 1);

    // The waiter is told the reservation is gone, and why.
    assert_eq!(
        g.claim(ticket),
        ClaimOutcome::Terminal(TerminalReason::Reclaimed)
    );
    // The reason is consumed once; a repeat claim is terminal too, never pending.
    assert_eq!(g.claim(ticket), ClaimOutcome::Invalid);

    let snapshot = g.snapshot();
    assert_eq!(snapshot.classes[&class].inflight, 0);
    assert_eq!(snapshot.classes[&class].queued, 0);
    assert_eq!(snapshot.conservation_violation(), None);
}

#[test]
fn a_released_promotion_is_terminal_rather_than_pending() {
    // The generic release path has to invalidate the ticket exactly like the
    // sweep does; otherwise the same dead-claim reappears under a different name.
    let (g, _clock, _class) = gov(leak_detecting_single_slot());
    let holder = admit(&g, "holder");
    let ticket = queue(&g, "queued");
    assert_eq!(g.release(holder), ReleaseOutcome::Released);

    let ClaimOutcome::Ready(promoted) = g.ticket_status(ticket) else {
        panic!("the queued request must have been promoted");
    };
    // Someone else releases the promoted permit before the waiter claims it.
    assert_eq!(g.release(promoted), ReleaseOutcome::Released);
    assert_eq!(
        g.claim(ticket),
        ClaimOutcome::Terminal(TerminalReason::Released)
    );
}

#[test]
fn an_unpromoted_ticket_reports_pending_not_terminal() {
    // The other half of the contract: "wait" and "give up" must not be the same
    // answer. A queued ticket is pending for as long as it is queued.
    let (g, _clock, _class) = gov(leak_detecting_single_slot());
    let holder = admit(&g, "holder");
    let ticket = queue(&g, "queued");
    assert_eq!(g.claim(ticket), ClaimOutcome::Pending);
    assert_eq!(
        g.claim(ticket),
        ClaimOutcome::Pending,
        "peeking is not consuming"
    );
    assert_eq!(g.release(holder), ReleaseOutcome::Released);
    assert!(matches!(g.claim(ticket), ClaimOutcome::Ready(_)));
}

#[test]
fn a_never_issued_ticket_is_invalid_not_pending() {
    let (g, _clock, _class) = gov(leak_detecting_single_slot());
    assert_eq!(g.claim(u64::MAX), ClaimOutcome::Invalid);
}

#[test]
fn ownership_transfers_exactly_once_across_claim_abandon_release_orders() {
    // Every interleaving of the four lifecycle verbs must move ownership at most
    // once and leave the ledger drained.
    for order in ["claim-release", "abandon", "claim-abandon", "release-first"] {
        let (g, _clock, class) = gov(leak_detecting_single_slot());
        let holder = admit(&g, "holder");
        let ticket = queue(&g, "queued");

        match order {
            "claim-release" => {
                assert_eq!(g.release(holder), ReleaseOutcome::Released);
                let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
                    panic!("promoted");
                };
                assert_eq!(g.release(permit), ReleaseOutcome::Released);
            }
            "abandon" => {
                // Abandon while still queued: the holder keeps its permit.
                g.abandon(ticket);
                assert_eq!(g.snapshot().classes[&class].queued, 0);
                assert_eq!(g.snapshot().classes[&class].inflight, 1);
                assert_eq!(g.release(holder), ReleaseOutcome::Released);
            }
            "claim-abandon" => {
                assert_eq!(g.release(holder), ReleaseOutcome::Released);
                let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
                    panic!("promoted");
                };
                // Abandoning an already-claimed ticket must not release the
                // permit the caller now owns.
                g.abandon(ticket);
                assert_eq!(g.snapshot().classes[&class].inflight, 1);
                assert_eq!(g.release(permit), ReleaseOutcome::Released);
            }
            _ => {
                assert_eq!(g.release(holder), ReleaseOutcome::Released);
                // Abandon after promotion releases the promoted permit.
                g.abandon(ticket);
            }
        }

        let snapshot = g.snapshot();
        assert_eq!(snapshot.classes[&class].inflight, 0, "order {order}");
        assert_eq!(snapshot.classes[&class].queued, 0, "order {order}");
        assert_eq!(snapshot.conservation_violation(), None, "order {order}");
    }
}

#[test]
fn a_foreign_or_repeated_release_does_not_free_another_execution() {
    let (g, _clock, class) = gov(ClassPolicy::new().max_inflight(4).cpu_units(1));
    let first = admit(&g, "first");
    let second = admit(&g, "second");
    assert_eq!(g.snapshot().classes[&class].inflight, 2);

    assert_eq!(g.release(first), ReleaseOutcome::Released);
    // Releasing the same permit again refunds nothing — and says so.
    assert_eq!(g.release(first), ReleaseOutcome::UnknownPermit);
    // A permit id that was never issued cannot free anything either.
    assert_eq!(g.release(u64::MAX), ReleaseOutcome::UnknownPermit);
    assert_eq!(g.snapshot().classes[&class].inflight, 1);

    assert_eq!(g.release(second), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().classes[&class].inflight, 0);
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn terminal_records_stay_bounded_under_repeated_churn() {
    // Terminal outcomes are retained for late claimers, so retention has to be
    // bounded or it is a leak. Past the bound the answer degrades from "why" to
    // "unknown" — still terminal, still never "keep waiting".
    let (g, clock, class) = gov(leak_detecting_single_slot());
    let churn = taskmesh_engine::MAX_TERMINAL_TICKETS + 64;
    let mut first_ticket = None;
    for i in 0..churn {
        // Each cycle promotes a queued request and then reclaims it unclaimed,
        // which is the transition that retains a terminal reason.
        let holder = admit(&g, &format!("h{i}"));
        let ticket = queue(&g, &format!("q{i}"));
        if first_ticket.is_none() {
            first_ticket = Some(ticket);
        }
        assert_eq!(g.release(holder), ReleaseOutcome::Released);
        clock.advance(100);
        assert_eq!(g.reap_leaks_with(10).reclaimed_permits, 1);
    }

    // The oldest outcomes were evicted to stay inside the retention bound. They
    // read as `Invalid` — still terminal, so a waiter stops rather than parks;
    // only the specific reason is gone.
    assert_eq!(
        g.claim(first_ticket.expect("at least one ticket")),
        ClaimOutcome::Invalid
    );
    assert!(
        g.retained_terminal_tickets() <= taskmesh_engine::MAX_TERMINAL_TICKETS,
        "retention {} exceeds the bound",
        g.retained_terminal_tickets()
    );
    let snapshot = g.snapshot();
    assert_eq!(snapshot.classes[&class].inflight, 0);
    assert_eq!(snapshot.classes[&class].queued, 0);
    assert_eq!(snapshot.conservation_violation(), None);
}

#[test]
fn observing_a_terminal_record_releases_its_retention_slot() {
    // Retention is for waiters that have not looked yet. Once a waiter has read
    // its terminal reason the record is gone — from the index *and* from the
    // eviction order — so a long-lived governor does not accumulate ghosts and
    // a later look reads `Invalid` (still terminal), not the stale reason.
    let (g, clock, _class) = gov(leak_detecting_single_slot());
    let mut tickets = Vec::new();
    for i in 0..16 {
        let holder = admit(&g, &format!("h{i}"));
        tickets.push(queue(&g, &format!("q{i}")));
        assert_eq!(g.release(holder), ReleaseOutcome::Released);
        clock.advance(100);
        assert_eq!(g.reap_leaks_with(10).reclaimed_permits, 1);
    }
    assert_eq!(g.retained_terminal_tickets(), 16);
    // Observe the middle ones out of order: removal must not depend on FIFO
    // position.
    for ticket in [tickets[7], tickets[3], tickets[12]] {
        assert_eq!(
            g.claim(ticket),
            ClaimOutcome::Terminal(TerminalReason::Reclaimed)
        );
        assert_eq!(g.claim(ticket), ClaimOutcome::Invalid);
    }
    assert_eq!(g.retained_terminal_tickets(), 13);
    for ticket in tickets {
        // Observed already, or observed now: either way nothing is retained after.
        let _outcome_is_terminal_either_way = matches!(
            g.claim(ticket),
            ClaimOutcome::Terminal(TerminalReason::Reclaimed) | ClaimOutcome::Invalid
        );
    }
    assert_eq!(g.retained_terminal_tickets(), 0);
}

#[test]
fn a_claim_restarts_the_lease_staleness_clock() {
    // A permit sits promoted-but-unclaimed while its waiter is parked. When the
    // waiter finally claims it, that claim is proof of life: the sweep's
    // staleness window restarts at the claim, not at the promotion. Otherwise a
    // permit could be reclaimed *while the host is dispatching it* — the exact
    // window in which the host believes it holds a live lease.
    let (g, clock, class) = gov(leak_detecting_single_slot());
    let holder = admit(&g, "holder");
    let ticket = queue(&g, "queued");
    assert_eq!(g.release(holder), ReleaseOutcome::Released); // promoted at t=1000
    clock.advance(9);
    let ClaimOutcome::Ready(permit) = g.claim(ticket) else {
        panic!("promoted a moment ago");
    };
    let ledger = g.permit_ledger(permit).expect("live permit");
    assert_eq!(ledger.leased_at_ms, 1_000, "promotion time is unchanged");
    assert_eq!(ledger.last_touched_ms, 1_009, "the claim is activity");

    // t=1015: the promotion is 15 old (stale for a 10 window) but the claim is
    // only 6 old — the lease is live and stays charged.
    clock.advance(6);
    let report = g.reap_leaks_with(10);
    assert_eq!(report.reclaimed_permits, 0, "claimed lease must survive");
    assert_eq!(report.suspected_leaks, 0);
    assert_eq!(g.snapshot().classes[&class].inflight, 1);

    // t=1021: now the *claim* is stale too, and nothing has advanced the phase.
    clock.advance(6);
    assert_eq!(g.reap_leaks_with(10).reclaimed_permits, 1);
    // The reclaimed permit is gone; releasing it reports exactly that.
    assert_eq!(g.release(permit), ReleaseOutcome::UnknownPermit);
    assert_eq!(g.snapshot().classes[&class].inflight, 0);
}
