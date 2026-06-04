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
fn stage_boundary_release_returns_units_but_keeps_permit() {
    let (g, _clock, c) = gov();
    let p = admit(&g);
    assert_eq!(g.snapshot().classes[&c].memory_units_held, 10);

    let freed = g.release_stage_memory(p, 4);
    assert_eq!(freed, 4);

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
