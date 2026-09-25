//! Lease custody regressions (H16-003 action A7, ADR 0003 D14).
//!
//! `Governor::release(permit_id)` is public, and permit ids are sequential. An
//! embedder holding the governor could therefore end a permit that a running
//! execution still owns — refunding its capacity under it — by passing the
//! wrong number. That used to be *detected* (the owner's own release later
//! found nothing). It is now *prevented*: the first phase advance out of
//! `DispatchReserved` mints the permit's one `LeaseToken`, and from then on only
//! `release_leased` with that token ends it.
//!
//! Every test compares the governor's whole observable state — snapshot and
//! every permit ledger — before and after a refused release. A refusal that
//! reports correctly but moves a gauge is still a refund.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ExecutionPhase, ManualClock, MemoryReleasePolicy, ResourceBudget, Snapshot,
    TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, AdvanceOutcome, AdvanceRefusal, Governor, LeaseToken, PermitId,
    PermitLedgerView, PolicySet, ReleaseOutcome,
};

fn class() -> TaskClass {
    TaskClass::new("c")
}

/// A governor whose class is leak-detecting (so the sweep has an opinion about
/// every permit) and whose `blocking` pool is gated (so a refund would show up
/// as pool occupancy as well as class inflight).
fn gov() -> (Governor, Arc<ManualClock>) {
    let mut classes = BTreeMap::new();
    classes.insert(
        class(),
        ClassPolicy::new()
            .max_inflight(8)
            .cpu_units(1)
            .memory_units(2)
            .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
    );
    let clock = Arc::new(ManualClock::new(1_000));
    let governor = Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(100).memory_units(100),
            classes,
        )
        .with_capability_limits(BTreeMap::from([("blocking".to_string(), 4)]))
        .expect("blocking is a registered pool"),
        clock.clone(),
    )
    .expect("valid policy");
    (governor, clock)
}

fn admit(g: &Governor, op: &str) -> PermitId {
    match g.admit(&TaskSpec::blocking(class()).operation(op.to_string())) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn lease(g: &Governor, permit: PermitId, phase: ExecutionPhase) -> LeaseToken {
    match g.advance_phase(permit, phase) {
        AdvanceOutcome::Leased(token) => token,
        other => panic!("the first advance of permit {permit} must lease it, got {other:?}"),
    }
}

/// Everything a refund could move.
fn observe(g: &Governor) -> (Snapshot, Vec<PermitLedgerView>) {
    (g.snapshot(), g.permit_ledgers())
}

#[test]
fn a_plain_release_cannot_end_a_dispatched_permit() {
    let (g, _) = gov();
    let permit = admit(&g, "owned");
    let token = lease(&g, permit, ExecutionPhase::Accepted);

    for phase in [
        ExecutionPhase::Accepted,
        ExecutionPhase::Running,
        ExecutionPhase::CleanupPending,
    ] {
        if phase != ExecutionPhase::Accepted {
            assert_eq!(g.advance_phase(permit, phase), AdvanceOutcome::Advanced);
        }
        let before = observe(&g);
        assert_eq!(
            g.release(permit),
            ReleaseOutcome::HeldByLease { phase },
            "a plain release must not end a dispatched permit ({phase})"
        );
        assert_eq!(
            observe(&g),
            before,
            "a refused release must change nothing ({phase})"
        );
    }
    let observed = &g.snapshot().classes[&class()];
    assert_eq!(
        (
            observed.inflight,
            observed.cleanup_pending,
            observed.terminated_total
        ),
        (1, 1, 0)
    );
    assert_eq!(g.snapshot().capabilities["blocking"].in_use, 1);

    // The owner is unaffected by the attempts: its lease still ends the permit.
    assert_eq!(g.release_leased(token), ReleaseOutcome::Released);
    assert!(g.permit_ledgers().is_empty());
}

#[test]
fn a_lease_releases_its_permit_exactly_once() {
    // The process-wide first nonce is 1. Spend it before checking an accessor
    // round trip so a constant-1 accessor cannot pass by coincidence.
    let (warmup, _) = gov();
    let warmup_permit = admit(&warmup, "warmup");
    let warmup_lease = lease(&warmup, warmup_permit, ExecutionPhase::Running);
    assert_eq!(
        warmup.release_leased(warmup_lease),
        ReleaseOutcome::Released
    );

    let (g, _) = gov();
    let permit = admit(&g, "owned");
    let token = lease(&g, permit, ExecutionPhase::Running);
    assert_eq!(token.permit_id(), permit);
    // A token is not `Clone`; a twin can only be forged. It stands in for the
    // one way a lease could be presented twice: a caller that kept a copy. It
    // must be an exact twin — the same proof — or "a spent lease is refused"
    // below would be proving something about a different token.
    let twin = LeaseToken::forge(token.permit_id(), token.nonce());
    assert_eq!(
        twin, token,
        "forge(permit_id(), nonce()) must reproduce the token exactly"
    );

    assert_eq!(
        g.release_leased(token),
        ReleaseOutcome::Released,
        "the lease must release its own permit"
    );
    let snapshot = g.snapshot();
    let observed = &snapshot.classes[&class()];
    assert_eq!(
        (
            observed.inflight,
            observed.running,
            observed.terminated_total
        ),
        (0, 0, 1)
    );
    assert_eq!(snapshot.capabilities["blocking"].in_use, 0);
    assert_eq!(snapshot.conservation_violation(), None);

    let before = observe(&g);
    assert_eq!(
        g.release_leased(twin),
        ReleaseOutcome::UnknownPermit,
        "a spent lease finds no permit"
    );
    assert_eq!(observe(&g), before);
    assert_eq!(g.snapshot().classes[&class()].terminated_total, 1);
}

#[test]
fn nonce_accessor_reproduces_distinct_live_lease_proofs() {
    let (g, _) = gov();
    let first_permit = admit(&g, "first");
    let second_permit = admit(&g, "second");
    let first = lease(&g, first_permit, ExecutionPhase::Running);
    let second = lease(&g, second_permit, ExecutionPhase::Running);

    assert_ne!(
        first.nonce(),
        second.nonce(),
        "nonce accessor must distinguish two live lease proofs"
    );
    assert_eq!(LeaseToken::forge(first.permit_id(), first.nonce()), first);
    assert_eq!(
        LeaseToken::forge(second.permit_id(), second.nonce()),
        second
    );
    assert_eq!(g.release_leased(first), ReleaseOutcome::Released);
    assert_eq!(g.release_leased(second), ReleaseOutcome::Released);
}

#[test]
fn a_forged_token_is_refused_and_changes_nothing() {
    let (g, _) = gov();
    let permit = admit(&g, "owned");
    let token = lease(&g, permit, ExecutionPhase::Running);
    // Minted nonces start at 1; zero is never a valid proof. This wrong-token
    // test must not depend on the accessor that has its own exact oracle.
    let forged = LeaseToken::forge(permit, 0);

    let before = observe(&g);
    assert_eq!(
        g.release_leased(forged),
        ReleaseOutcome::HeldByLease {
            phase: ExecutionPhase::Running
        },
        "a token whose nonce is not the permit's lease must not release it"
    );
    assert_eq!(observe(&g), before, "a refused lease changes nothing");
    assert_eq!(g.release_leased(token), ReleaseOutcome::Released);
}

#[test]
fn a_token_from_another_governor_is_not_this_permits_lease() {
    // Two governors hand out the same local sequence, but both the permit and
    // lease token remain bound to the issuing governor authority.
    let (a, _) = gov();
    let (b, _) = gov();
    let on_a = admit(&a, "a");
    let on_b = admit(&b, "b");
    assert_eq!(on_a.sequence(), on_b.sequence());
    assert_ne!(on_a, on_b);
    let lease_a = lease(&a, on_a, ExecutionPhase::Running);
    let lease_b = lease(&b, on_b, ExecutionPhase::Running);

    let before = observe(&b);
    assert_eq!(
        b.release_leased(lease_a),
        ReleaseOutcome::UnknownPermit,
        "another governor's lease must not resolve to this governor's permit"
    );
    assert_eq!(observe(&b), before);
    assert_eq!(b.release_leased(lease_b), ReleaseOutcome::Released);
    // `a`'s permit is still charged: its token was spent on the wrong governor.
    assert_eq!(a.snapshot().classes[&class()].inflight, 1);
}

#[test]
fn only_the_first_advance_mints_the_lease() {
    let (g, _) = gov();
    let permit = admit(&g, "owned");
    let token = lease(&g, permit, ExecutionPhase::Accepted);
    for phase in [ExecutionPhase::Running, ExecutionPhase::CleanupPending] {
        assert_eq!(
            g.advance_phase(permit, phase),
            AdvanceOutcome::Advanced,
            "only the first advance mints a lease; a later one must not reissue it ({phase})"
        );
    }
    // A refused advance does not mint either.
    assert_eq!(
        g.advance_phase(permit, ExecutionPhase::Running),
        AdvanceOutcome::Refused(AdvanceRefusal::NotLater {
            current: ExecutionPhase::CleanupPending
        })
    );
    assert_eq!(
        g.release_leased(token),
        ReleaseOutcome::Released,
        "the lease minted first is still the permit's lease"
    );
}

#[test]
fn the_sweep_leaves_a_leased_permit_to_its_lease() {
    // The sweep reclaims only `DispatchReserved` permits, which are exactly the
    // unleased ones. A leased permit that goes stale is suspected and retained,
    // never reclaimed — its lease remains the one way it ends.
    let (g, clock) = gov();
    let leased = admit(&g, "leased");
    let reserved = admit(&g, "reserved");
    let token = lease(&g, leased, ExecutionPhase::Accepted);

    clock.advance(10_000);
    let report = g.reap_leaks_with(1_000);
    assert_eq!(report.suspected_leaks, 2, "both leases are stale");
    assert_eq!(
        (report.reclaimed_permits, report.retained_active),
        (1, 1),
        "the sweep must reclaim only the unleased reservation and retain the leased permit"
    );
    assert!(g.permit_ledger(reserved).is_none());
    assert_eq!(
        g.permit_ledger(leased).map(|ledger| ledger.phase),
        Some(ExecutionPhase::Accepted)
    );
    assert_eq!(
        g.release(leased),
        ReleaseOutcome::HeldByLease {
            phase: ExecutionPhase::Accepted
        }
    );
    assert_eq!(g.release_leased(token), ReleaseOutcome::Released);
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn an_undispatched_permit_is_still_released_by_id() {
    // D14 "legacy direct Governor admit": an embedder that admits and never
    // declares a phase owns the permit outright, and `release` is its release.
    let (g, _) = gov();
    let permit = admit(&g, "direct");
    assert_eq!(g.phase(permit), Some(ExecutionPhase::DispatchReserved));

    // A lease it never had is not a proof it can present.
    let before = observe(&g);
    assert_eq!(
        g.release_leased(LeaseToken::forge(permit, 1)),
        ReleaseOutcome::HeldByLease {
            phase: ExecutionPhase::DispatchReserved
        },
        "a token cannot release a permit that has no lease"
    );
    assert_eq!(observe(&g), before);

    assert_eq!(g.release(permit), ReleaseOutcome::Released);
    assert_eq!(g.release(permit), ReleaseOutcome::UnknownPermit);
    assert!(g.permit_ledgers().is_empty());
}
