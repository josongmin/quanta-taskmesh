//! T03: permit acquire/release is inseparable from inflight accounting.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov(list: Vec<(&'static str, ClassPolicy)>) -> Governor {
    let classes: BTreeMap<TaskClass, ClassPolicy> = list
        .into_iter()
        .map(|(n, p)| (TaskClass::new(n), p))
        .collect();
    Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(100).memory_units(100),
            classes,
        ),
        Arc::new(ManualClock::new(1000)),
    )
}

fn spec(class: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new(class.to_string())).operation(format!("op:{class}"))
}

#[test]
fn permit_and_inflight_move_together() {
    let g = gov(vec![(
        "c",
        ClassPolicy::new()
            .max_inflight(4)
            .cpu_units(2)
            .memory_units(3),
    )]);
    let c = TaskClass::new("c");

    let p1 = match g.admit(&spec("c"), RequestKey::new("a")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    let snap = &g.snapshot().classes[&c];
    assert_eq!(snap.inflight, 1);
    assert_eq!(snap.cpu_units_held, 2);
    assert_eq!(snap.memory_units_held, 3);

    g.release(p1);
    let snap = &g.snapshot().classes[&c];
    assert_eq!(snap.inflight, 0);
    assert_eq!(snap.cpu_units_held, 0);
    assert_eq!(snap.memory_units_held, 0);
}

#[test]
fn release_of_unknown_permit_is_noop() {
    let g = gov(vec![("c", ClassPolicy::new())]);
    g.release(99_999); // must not panic or underflow
    assert_eq!(g.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}

#[test]
fn double_release_does_not_underflow() {
    let g = gov(vec![("c", ClassPolicy::new().max_inflight(2).cpu_units(1))]);
    let p = match g.admit(&spec("c"), RequestKey::new("a")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    g.release(p);
    g.release(p);
    assert_eq!(g.snapshot().classes[&TaskClass::new("c")].inflight, 0);
}
