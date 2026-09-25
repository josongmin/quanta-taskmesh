//! Regressions for the memory-lifecycle findings (H16-005).
//!
//! * **TM16-011** — an `Estimated` reconcile restored the *original* reservation,
//!   so `reserve 8 → release 6 → reconcile` went back to holding 8. A reconcile
//!   reports a measurement; it must not re-reserve returned capacity.
//! * **TM16-009** — a stage-boundary release did not touch the lease, so work
//!   that demonstrably reached a stage boundary two milliseconds ago was
//!   reclaimed as leaked.
//! * **TM16-026** — the clock was sampled before the state lock, so a thread
//!   delayed between the two stamped a brand-new lease with a timestamp older
//!   than leases already committed, and a sweep reclaimed it immediately.
//! * **TM16-005** — `MemoryReleasePolicy` was documentation: a stage release
//!   succeeded on an `OnTaskCompletion` class, which is the one policy that says
//!   it should not.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, ManualClock, MemoryOvercommitPolicy, MemoryPermitMode, MemoryReleasePolicy,
    ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, CapacityBlock, ClaimOutcome, Governor, PermitId, PolicySet,
    ReconcileOutcome, ReleaseOutcome, StageReleaseOutcome,
};

fn gov(policy: ClassPolicy) -> (Governor, Arc<ManualClock>) {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new("c"), policy);
    let clock = Arc::new(ManualClock::new(1_000));
    let governor = Governor::new(
        PolicySet::new(
            ResourceBudget::new()
                .memory_units(10_000)
                .cpu_units(10_000)
                .memory_unit_scale(1),
            classes,
        ),
        #[allow(
            clippy::clone_on_ref_ptr,
            reason = "Arc<ManualClock> -> Arc<dyn Clock>"
        )]
        clock.clone(),
    )
    .expect("valid policy");
    (governor, clock)
}

fn admit(g: &Governor, op: &str) -> PermitId {
    match g.admit(&TaskSpec::io(TaskClass::new("c")).operation(op.to_string())) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

fn held(g: &Governor) -> u128 {
    g.snapshot().classes[&TaskClass::new("c")].memory_units_held
}

// ---- TM16-011: a reconcile must not resurrect a returned reservation --------

#[test]
fn estimated_reconcile_does_not_restore_released_stage_units() {
    let (g, _clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(8)
        .memory_permit_mode(MemoryPermitMode::Estimated)
        .memory_release_policy(MemoryReleasePolicy::OnStageBoundary));
    let permit = admit(&g, "op");
    assert_eq!(held(&g), 8);

    assert_eq!(
        g.release_stage_memory(permit, 6),
        StageReleaseOutcome::Released { freed_units: 6 }
    );
    assert_eq!(held(&g), 2);

    // The reconcile is accounting-neutral for `Estimated`: it reports a
    // measurement, and the reservation still held is 2 — not the original 8.
    assert!(g.reconcile_memory(permit, 2).is_applied());
    assert_eq!(held(&g), 2, "reconcile must not re-reserve returned units");

    // Repeating it stays stable rather than ratcheting back up.
    for _ in 0..5 {
        assert!(g.reconcile_memory(permit, 2).is_applied());
        assert_eq!(held(&g), 2);
    }

    // The original estimate is still visible as provenance; it is simply not a
    // live claim any more.
    let ledger = g.permit_ledger(permit).expect("live permit");
    assert_eq!(ledger.original_estimate_units, 8);
    assert_eq!(ledger.remaining_reservation_units, 2);
    assert_eq!(ledger.effective_units, 2);

    assert_eq!(g.release(permit), ReleaseOutcome::Released);
    assert_eq!(held(&g), 0);
}

#[test]
fn hybrid_floor_follows_the_remaining_reservation_not_the_original() {
    let (g, _clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(8)
        .memory_permit_mode(MemoryPermitMode::Hybrid)
        .memory_release_policy(MemoryReleasePolicy::OnStageBoundary));
    let permit = admit(&g, "op");
    assert_eq!(held(&g), 8);
    assert_eq!(
        g.release_stage_memory(permit, 6),
        StageReleaseOutcome::Released { freed_units: 6 }
    );
    assert_eq!(held(&g), 2);

    // Measured below the remaining reservation: the floor is 2, not 8.
    assert!(g.reconcile_memory(permit, 1).is_applied());
    assert_eq!(held(&g), 2);
    // Measured above it: measurement wins, as `Hybrid` promises.
    assert!(g.reconcile_memory(permit, 5).is_applied());
    assert_eq!(held(&g), 5);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}

#[test]
fn measured_mode_tracks_the_reading_across_a_stage_release() {
    let (g, _clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(8)
        .memory_permit_mode(MemoryPermitMode::Measured)
        .memory_release_policy(MemoryReleasePolicy::OnStageBoundary));
    let permit = admit(&g, "op");
    assert!(g.reconcile_memory(permit, 6).is_applied());
    assert_eq!(held(&g), 6);
    assert_eq!(
        g.release_stage_memory(permit, 4),
        StageReleaseOutcome::Released { freed_units: 4 }
    );
    assert_eq!(held(&g), 2);
    assert!(g.reconcile_memory(permit, 2).is_applied());
    assert_eq!(held(&g), 2);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}

// ---- TM16-005: the release policy is enforced, and says so ------------------

#[test]
fn stage_release_is_governed_by_the_class_release_policy() {
    for (policy, expected_release) in [
        (MemoryReleasePolicy::OnStageBoundary, true),
        (MemoryReleasePolicy::LeakDetecting, true),
        (MemoryReleasePolicy::OnTaskCompletion, false),
    ] {
        let (g, _clock) = gov(ClassPolicy::new()
            .max_inflight(4)
            .memory_units(10)
            .memory_release_policy(policy));
        let permit = admit(&g, "op");
        let outcome = g.release_stage_memory(permit, 10);
        if expected_release {
            assert_eq!(
                outcome,
                StageReleaseOutcome::Released { freed_units: 10 },
                "{policy:?} permits a stage-boundary release"
            );
            assert_eq!(held(&g), 0);
        } else {
            // Reported, not folded into "freed 0 units": a caller cannot tell an
            // enforced policy from a permit that held nothing.
            assert_eq!(
                outcome,
                StageReleaseOutcome::PolicyForbids { policy },
                "{policy:?} returns units at task completion only"
            );
            assert_eq!(held(&g), 10, "{policy:?} keeps the units held");
        }
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
        assert_eq!(held(&g), 0);
    }
}

#[test]
fn stage_release_reports_unknown_and_duplicate_events() {
    let (g, _clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(10)
        .memory_release_policy(MemoryReleasePolicy::OnStageBoundary));
    assert_eq!(
        g.release_stage_memory(PermitId::forge(0, u64::MAX), 1),
        StageReleaseOutcome::UnknownPermit
    );

    let permit = admit(&g, "op");
    assert_eq!(
        g.release_stage_memory_seq(permit, 3, 1),
        StageReleaseOutcome::Released { freed_units: 3 }
    );
    // A replayed or reordered stage event must not be applied twice.
    assert_eq!(
        g.release_stage_memory_seq(permit, 3, 1),
        StageReleaseOutcome::StaleSequence {
            current_sequence: 1
        }
    );
    assert_eq!(
        g.release_stage_memory_seq(permit, 3, 0),
        StageReleaseOutcome::StaleSequence {
            current_sequence: 1
        }
    );
    assert_eq!(held(&g), 7, "only the first delta applied");
    assert_eq!(
        g.release_stage_memory_seq(permit, 2, 2),
        StageReleaseOutcome::Released { freed_units: 2 }
    );
    assert_eq!(held(&g), 5);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}

// ---- R3-mutants: a stage release hands its units to queued work -------------

/// A governor whose memory budget fits exactly two permits of class `c`; a
/// third queues on memory and is only promotable by units coming back.
fn memory_contended() -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(8)
            .max_queue_depth(4)
            .memory_units(5)
            .memory_release_policy(MemoryReleasePolicy::OnStageBoundary)
            .memory_overcommit_policy(MemoryOvercommitPolicy::Queue),
    );
    Governor::new(
        PolicySet::new(
            ResourceBudget::new()
                .memory_units(10)
                .cpu_units(10_000)
                .memory_unit_scale(1),
            classes,
        ),
        Arc::new(ManualClock::new(1_000)),
    )
    .expect("valid policy")
}

#[test]
fn a_stage_release_promotes_work_queued_on_memory() {
    // Returning memory at a stage boundary is a capacity-freeing transition
    // like a release or a downward reconcile: the units it returns belong to
    // whoever is queued for them, in the same transition. A stage release that
    // only updated the ledger would leave the queued request waiting for an
    // event that never comes — the holder is still running, so no
    // task-completion release is due, and nothing else frees memory.
    let g = memory_contended();
    let first = admit(&g, "first");
    let second = admit(&g, "second");
    let AdmissionDecision::Queued { ticket } =
        g.admit(&TaskSpec::io(TaskClass::new("c")).operation("third"))
    else {
        panic!("the third request does not fit a budget of two");
    };
    assert_eq!(g.pending_block_reason(ticket), Some(CapacityBlock::Memory));

    assert_eq!(
        g.release_stage_memory(first, 5),
        StageReleaseOutcome::Released { freed_units: 5 }
    );
    let status = g.ticket_status(ticket);
    assert!(
        matches!(status, ClaimOutcome::Ready(_)),
        "a stage release that freed 5 units must promote the request queued on memory, got {status:?}"
    );
    let ClaimOutcome::Ready(third) = g.claim(ticket) else {
        panic!("promoted a moment ago");
    };
    assert_eq!(
        held(&g),
        10,
        "the freed units are held by the promoted request"
    );
    for permit in [first, second, third] {
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn stage_release_outcomes_report_their_freed_units() {
    // The accessors are how a host folds the outcome into its own accounting:
    // only a real release reports units, and every refusal reports zero *and*
    // says it was not a release.
    let released = StageReleaseOutcome::Released { freed_units: 6 };
    assert_eq!(released.freed_units(), 6);
    assert!(released.is_released());
    let forbidden = StageReleaseOutcome::PolicyForbids {
        policy: MemoryReleasePolicy::OnTaskCompletion,
    };
    assert_eq!(
        forbidden.freed_units(),
        0,
        "a refused release freed nothing"
    );
    assert!(
        !forbidden.is_released(),
        "a refused release is not a release"
    );
    assert_eq!(StageReleaseOutcome::UnknownPermit.freed_units(), 0);
    assert!(!StageReleaseOutcome::UnknownPermit.is_released());
    let stale = StageReleaseOutcome::StaleSequence {
        current_sequence: 3,
    };
    assert_eq!(stale.freed_units(), 0);
    assert!(!stale.is_released());
}

#[test]
fn reconciling_an_unknown_permit_applies_nothing() {
    let (g, _clock) = gov(ClassPolicy::new().max_inflight(4).memory_units(4));
    assert!(
        !g.reconcile_memory(PermitId::forge(0, u64::MAX), 1)
            .is_applied(),
        "a reconcile against a permit that does not exist is not applied"
    );
    assert_eq!(
        g.reconcile_memory_at(PermitId::forge(0, u64::MAX), 1, 1),
        ReconcileOutcome::UnknownPermit
    );
    assert!(!ReconcileOutcome::UnknownPermit.is_applied());
    assert!(ReconcileOutcome::Applied { held_units: 1 }.is_applied());
}

// ---- TM16-009: stage activity is a heartbeat --------------------------------

#[test]
fn stage_activity_keeps_a_lease_alive_through_the_sweep() {
    let (g, clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(10)
        .memory_release_policy(MemoryReleasePolicy::LeakDetecting));
    let permit = admit(&g, "op"); // leased at 1_000

    clock.set(60_999);
    assert_eq!(
        g.release_stage_memory(permit, 1),
        StageReleaseOutcome::Released { freed_units: 1 }
    );

    // Two milliseconds after demonstrable stage activity, with a 60s window.
    clock.set(61_001);
    let report = g.reap_leaks_with(60_000);
    assert_eq!(
        report.reclaimed_permits, 0,
        "recent stage activity is activity"
    );
    assert_eq!(report.suspected_leaks, 0);
    assert_eq!(g.snapshot().classes[&TaskClass::new("c")].inflight, 1);

    // Past the window with no further activity, it is reclaimed.
    clock.set(121_001);
    assert_eq!(g.reap_leaks_with(60_000).reclaimed_permits, 1);
    assert_eq!(g.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[test]
fn a_zero_unit_stage_release_still_counts_as_activity() {
    // Reaching a stage boundary and having nothing to return is still evidence
    // the work is alive.
    let (g, clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(10)
        .memory_release_policy(MemoryReleasePolicy::LeakDetecting));
    let permit = admit(&g, "op");
    clock.set(60_999);
    assert_eq!(
        g.release_stage_memory(permit, 0),
        StageReleaseOutcome::Released { freed_units: 0 }
    );
    clock.set(61_001);
    assert_eq!(g.reap_leaks_with(60_000).reclaimed_permits, 0);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}

// ---- TM16-026: commit time is monotonic ------------------------------------

#[test]
fn a_late_clock_sample_cannot_backdate_a_new_lease() {
    // Models the observed interleaving: one admission reads the clock and is
    // delayed before committing, while another admission commits at a later
    // time. The delayed request's sample is now older than state already
    // committed — and it must not produce a lease that is stale on arrival.
    let (g, clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(1)
        .memory_release_policy(MemoryReleasePolicy::LeakDetecting));
    clock.set(1_000);
    let committed_first = admit(&g, "b"); // commits at 1_000

    // The delayed thread's sample, taken long before, lands now.
    clock.set(0);
    let delayed = admit(&g, "a");

    clock.set(1_000);
    let report = g.reap_leaks_with(100);
    assert_eq!(
        report.reclaimed_permits, 0,
        "a lease created now is not 900ms old"
    );

    let first_ledger = g.permit_ledger(committed_first).expect("live");
    let delayed_ledger = g.permit_ledger(delayed).expect("live");
    assert_eq!(delayed_ledger.leased_at_ms, 1_000);
    assert!(
        delayed_ledger.leased_at_ms >= first_ledger.leased_at_ms,
        "a later commit cannot carry an earlier logical time"
    );

    assert_eq!(g.release(committed_first), ReleaseOutcome::Released);
    assert_eq!(g.release(delayed), ReleaseOutcome::Released);
}

#[test]
fn a_late_heartbeat_cannot_move_activity_backwards() {
    let (g, clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(4)
        .memory_permit_mode(MemoryPermitMode::Measured)
        .memory_release_policy(MemoryReleasePolicy::LeakDetecting));
    let permit = admit(&g, "op");

    clock.set(2_000);
    assert_eq!(
        g.reconcile_memory_at(permit, 2, 2),
        ReconcileOutcome::Applied { held_units: 2 }
    );
    let fresh = g.permit_ledger(permit).expect("live").last_touched_ms;
    assert_eq!(fresh, 2_000);

    // An older reading commits late. It is rejected outright by epoch...
    clock.set(1_500);
    assert_eq!(
        g.reconcile_memory_at(permit, 9, 1),
        ReconcileOutcome::StaleEpoch { current_epoch: 2 }
    );
    let ledger = g.permit_ledger(permit).expect("live");
    assert_eq!(
        ledger.last_touched_ms, fresh,
        "activity did not go backwards"
    );
    assert_eq!(ledger.effective_units, 2, "stale reading was not applied");

    // ...and even an accepted newer epoch carrying an older sample cannot
    // backdate the lease, because commit time is monotonic.
    assert_eq!(
        g.reconcile_memory_at(permit, 3, 3),
        ReconcileOutcome::Applied { held_units: 3 }
    );
    assert!(g.permit_ledger(permit).expect("live").last_touched_ms >= fresh);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}

#[test]
fn an_unordered_reconcile_takes_the_epoch_after_whatever_is_recorded() {
    // `reconcile_memory` (no explicit epoch) is applied against the epoch the
    // *same* transition reads — never one read earlier under a different lock.
    // Observable consequence: it always lands, and it lands right after the
    // newest explicit epoch rather than at a "next" computed before it.
    let (g, _clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .cpu_units(1)
        .memory_units(8)
        .memory_permit_mode(MemoryPermitMode::Measured));
    // The harness scale is one byte per unit.
    let permit = admit(&g, "measured");
    assert_eq!(
        g.reconcile_memory_at(permit, 3, 5),
        ReconcileOutcome::Applied { held_units: 3 }
    );
    assert!(
        g.reconcile_memory(permit, 2).is_applied(),
        "auto epoch 6 follows 5"
    );
    let ledger = g.permit_ledger(permit).expect("live");
    assert_eq!(ledger.measurement_epoch, 6);
    assert_eq!(ledger.effective_units, 2);
    // An explicit epoch at or below the auto-assigned one is now stale.
    assert_eq!(
        g.reconcile_memory_at(permit, 7, 6),
        ReconcileOutcome::StaleEpoch { current_epoch: 6 }
    );
    assert!(
        g.reconcile_memory(permit, 1).is_applied(),
        "auto epoch 7 follows 6"
    );
    assert_eq!(g.permit_ledger(permit).expect("live").measurement_epoch, 7);
    assert_eq!(g.release(permit), ReleaseOutcome::Released);
}

#[test]
fn terminal_measurement_sequence_never_wraps_or_mutates_on_exhaustion() {
    let (g, _clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .cpu_units(1)
        .memory_units(8)
        .memory_permit_mode(MemoryPermitMode::Measured));
    let permit = admit(&g, "terminal-sequence");

    assert_eq!(
        g.reconcile_memory_at(permit, 3, u64::MAX - 1),
        ReconcileOutcome::Applied { held_units: 3 }
    );
    assert_eq!(
        g.reconcile_memory(permit, 4),
        ReconcileOutcome::Applied { held_units: 4 },
        "checked implicit successor accepts the terminal value exactly once"
    );
    let ledger_at_max = g.permit_ledger(permit).expect("live permit");
    let snapshot_at_max = g.snapshot();
    assert_eq!(ledger_at_max.measurement_epoch, u64::MAX);

    for outcome in [
        g.reconcile_memory(permit, 1),
        g.reconcile_memory_at(permit, 9, u64::MAX),
        g.reconcile_memory_at(permit, 9, 1),
    ] {
        assert_eq!(
            outcome,
            ReconcileOutcome::EpochExhausted {
                current_epoch: u64::MAX
            }
        );
        assert_eq!(
            g.permit_ledger(permit),
            Some(ledger_at_max.clone()),
            "ledger, measurement and activity remain unchanged"
        );
        assert_eq!(
            g.snapshot(),
            snapshot_at_max,
            "class, capability and global aggregates remain unchanged"
        );
    }

    assert_eq!(g.release(permit), ReleaseOutcome::Released);
    assert_eq!(held(&g), 0, "terminal sequence never prevents release");
}

#[test]
fn a_running_lease_is_suspected_but_not_reclaimed() {
    // Stale is not dead. Once work has reached an executor, an untouched lease
    // is a quiet task, not a dead one; reclaiming it would hand its capacity to
    // a second task while the first is still using it.
    use taskmesh_contract::ExecutionPhase;

    let (g, clock) = gov(ClassPolicy::new()
        .max_inflight(4)
        .memory_units(4)
        .memory_release_policy(MemoryReleasePolicy::LeakDetecting));
    let running = admit(&g, "running");
    let never_started = admit(&g, "reserved");
    let taskmesh_engine::AdvanceOutcome::Leased(running_lease) =
        g.advance_phase(running, ExecutionPhase::Running)
    else {
        panic!("the first advance of a live permit leases it");
    };

    clock.set(200_000);
    let report = g.reap_leaks_with(1_000);
    assert_eq!(report.suspected_leaks, 2, "both leases are stale");
    assert_eq!(
        report.reclaimed_permits, 1,
        "only the one that never started is reclaimed"
    );
    assert_eq!(report.retained_active, 1);
    assert!(
        g.permit_ledger(running).is_some(),
        "running work stays charged"
    );
    assert!(g.permit_ledger(never_started).is_none());
    assert_eq!(g.release_leased(running_lease), ReleaseOutcome::Released);
}
