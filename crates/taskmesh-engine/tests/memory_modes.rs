//! T05: Estimated/Measured/Hybrid all have executable accounting semantics.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov(mode: MemoryPermitMode, memory_units: u32) -> (Governor, TaskClass) {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(8)
            .memory_units(memory_units)
            .memory_permit_mode(mode),
    );
    let resources = ResourceBudget::new()
        .memory_units(1000)
        .memory_unit_scale(10);
    (
        Governor::new_unchecked(
            PolicySet::new(resources, classes),
            Arc::new(ManualClock::new(1000)),
        ),
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

fn held(g: &Governor, c: &TaskClass) -> u32 {
    g.snapshot().classes[c].memory_units_held
}

#[test]
fn estimated_mode_uses_configured_units() {
    let (g, c) = gov(MemoryPermitMode::Estimated, 3);
    let p = admit(&g);
    assert_eq!(held(&g, &c), 3);
    // Reconcile is a no-op for estimated mode.
    g.reconcile_memory(p, 9_999);
    assert_eq!(held(&g, &c), 3);
}

#[test]
fn measured_mode_converts_bytes_to_units() {
    let (g, c) = gov(MemoryPermitMode::Measured, 5);
    let p = admit(&g);
    assert_eq!(held(&g, &c), 5); // initial estimate reserved
                                 // 25 bytes / 10 bytes-per-unit = ceil = 3 units.
    assert!(g.reconcile_memory(p, 25));
    assert_eq!(held(&g, &c), 3);
}

#[test]
fn hybrid_reconcile_keeps_max_of_estimate_and_measured() {
    let (g, c) = gov(MemoryPermitMode::Hybrid, 5);
    let p = admit(&g);
    assert_eq!(held(&g, &c), 5);

    // measured 80 bytes -> 8 units; max(5, 8) = 8.
    g.reconcile_memory(p, 80);
    assert_eq!(held(&g, &c), 8);

    // measured 30 bytes -> 3 units; max(5, 3) = 5 (never drops below estimate).
    g.reconcile_memory(p, 30);
    assert_eq!(held(&g, &c), 5);
}
