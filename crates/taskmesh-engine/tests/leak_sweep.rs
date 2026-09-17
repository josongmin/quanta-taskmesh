//! T05: leak-detecting is a real sweep path, and stage-boundary early release
//! returns memory units while keeping the permit alive.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov() -> (Governor, Arc<ManualClock>, TaskClass) {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(8)
            .cpu_units(1)
            .memory_units(10)
            // Only LeakDetecting classes are force-reclaimed by the sweep.
            .memory_release_policy(MemoryReleasePolicy::LeakDetecting),
    );
    let clock = Arc::new(ManualClock::new(1000));
    let resources = ResourceBudget::new().cpu_units(100).memory_units(1000);
    (
        Governor::new_unchecked(PolicySet::new(resources, classes), clock.clone()),
        clock,
        TaskClass::new("c"),
    )
}

fn admit(g: &Governor) -> PermitId {
    let spec = TaskSpec::blocking(TaskClass::new("c")).operation("op");
    match g.admit(&spec) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    }
}

#[test]
fn leak_sweep_reclaims_stale_permits() {
    let (g, clock, c) = gov();
    let _p = admit(&g);
    assert_eq!(g.snapshot().classes[&c].inflight, 1);

    // Not yet stale.
    let report = g.reap_leaks_with(60_000);
    assert_eq!(report.suspected_leaks, 0);
    assert_eq!(g.snapshot().classes[&c].inflight, 1);

    // Advance past the staleness window.
    clock.advance(60_001);
    let report = g.reap_leaks_with(60_000);
    assert_eq!(report.suspected_leaks, 1);
    assert_eq!(report.reclaimed_permits, 1);
    assert_eq!(g.snapshot().classes[&c].inflight, 0);
}

#[test]
fn default_leak_window_is_exposed() {
    assert_eq!(DEFAULT_LEAK_STALE_MS, 60_000);
}

#[test]
fn the_default_sweep_window_is_exclusive_at_its_boundary() {
    // `reap_leaks()` is `reap_leaks_with(DEFAULT_LEAK_STALE_MS)`, and staleness
    // is "untouched for *longer than* the window": a lease exactly the window
    // old is not yet stale, one millisecond older is.
    let (g, clock, c) = gov();
    let permit = admit(&g);
    clock.advance(DEFAULT_LEAK_STALE_MS);
    let at_boundary = g.reap_leaks();
    assert_eq!(
        at_boundary.reclaimed_permits, 0,
        "a lease exactly DEFAULT_LEAK_STALE_MS old is not stale: the window is exclusive"
    );
    assert_eq!(at_boundary.suspected_leaks, 0);
    assert_eq!(g.snapshot().classes[&c].inflight, 1);

    clock.advance(1);
    let past = g.reap_leaks();
    assert_eq!(
        past.reclaimed_permits, 1,
        "one millisecond past the default window the lease is reclaimed"
    );
    assert!(
        g.permit_ledger(permit).is_none(),
        "the reclaimed permit is gone"
    );
    assert_eq!(g.snapshot().classes[&c].inflight, 0);
}

#[test]
fn a_reclaimed_leak_promotes_work_queued_on_memory() {
    // Reclaiming a leaked lease returns its memory, and that memory belongs to
    // whoever is queued for it — in the same sweep. A sweep that only unwound
    // the ledger would leave the queued request waiting: nothing else frees
    // memory, because the other holder is live and running.
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(8)
            .max_queue_depth(4)
            .memory_units(5)
            .memory_release_policy(MemoryReleasePolicy::LeakDetecting)
            .memory_overcommit_policy(MemoryOvercommitPolicy::Queue),
    );
    let clock = Arc::new(ManualClock::new(1_000));
    let g = Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(100).memory_units(10),
            classes,
        ),
        clock.clone(),
    )
    .expect("valid policy");
    let leaked = admit(&g);
    let running = admit(&g);
    assert!(g.advance_phase(running, ExecutionPhase::Running));
    let AdmissionDecision::Queued { ticket } =
        g.admit(&TaskSpec::blocking(TaskClass::new("c")).operation("third"))
    else {
        panic!("the third request does not fit a budget of two");
    };
    assert_eq!(g.pending_block_reason(ticket), Some(CapacityBlock::Memory));

    clock.advance(10_000);
    let report = g.reap_leaks_with(1_000);
    assert_eq!(
        report.reclaimed_permits, 1,
        "only the lease that never reached an executor is reclaimed"
    );
    assert_eq!(report.retained_active, 1, "the running lease stays charged");
    let status = g.ticket_status(ticket);
    assert!(
        matches!(status, ClaimOutcome::Ready(_)),
        "a sweep that reclaimed 5 units must promote the request queued on memory, got {status:?}"
    );
    assert!(g.permit_ledger(leaked).is_none());
    let ClaimOutcome::Ready(third) = g.claim(ticket) else {
        panic!("promoted a moment ago");
    };
    assert_eq!(
        g.snapshot().classes[&TaskClass::new("c")].memory_units_held,
        10
    );
    for permit in [running, third] {
        assert_eq!(g.release(permit), ReleaseOutcome::Released);
    }
    assert_eq!(g.snapshot().conservation_violation(), None);
}

#[test]
fn stage_boundary_release_returns_units_but_keeps_permit() {
    let (g, _clock, c) = gov();
    let p = admit(&g);
    assert_eq!(g.snapshot().classes[&c].memory_units_held, 10);

    let freed = g.release_stage_memory(p, 4);
    assert_eq!(freed, StageReleaseOutcome::Released { freed_units: 4 });

    let snap = g.snapshot();
    assert_eq!(snap.classes[&c].memory_units_held, 6);
    assert_eq!(snap.classes[&c].inflight, 1, "permit stays alive");
}

#[test]
fn non_leak_detecting_class_is_not_reaped() {
    // A class with the default OnTaskCompletion release policy must NOT be
    // force-reclaimed by the sweep, even when stale.
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(8)
            .cpu_units(1)
            .memory_units(10)
            .memory_release_policy(MemoryReleasePolicy::OnTaskCompletion),
    );
    let clock = Arc::new(ManualClock::new(1000));
    let g = Governor::new_unchecked(
        PolicySet::new(
            ResourceBudget::new().cpu_units(100).memory_units(1000),
            classes,
        ),
        clock.clone(),
    );
    let spec = TaskSpec::blocking(TaskClass::new("c")).operation("op");
    let _p = match g.admit(&spec) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    clock.advance(60_001);
    let report = g.reap_leaks_with(60_000);
    assert_eq!(
        report.suspected_leaks, 0,
        "non-leak-detecting class must be exempt"
    );
    assert_eq!(report.reclaimed_permits, 0);
    assert_eq!(g.snapshot().classes[&TaskClass::new("c")].inflight, 1);
}
