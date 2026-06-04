//! T06: child permits roll into root accounting; saturation bubbles to a verdict.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov(list: Vec<(&'static str, ClassPolicy)>) -> Governor {
    let classes: BTreeMap<TaskClass, ClassPolicy> = list
        .into_iter()
        .map(|(n, p)| (TaskClass::new(n), p))
        .collect();
    Governor::new_unchecked(
        PolicySet::new(
            ResourceBudget::new().cpu_units(100).memory_units(100),
            classes,
        ),
        Arc::new(ManualClock::new(1000)),
    )
}

fn child(class: &str, root: &str, stage: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new(class.to_string()))
        .child_of(root.to_string(), TaskStage::new(stage.to_string()))
}

#[test]
fn child_permit_rolls_into_root_accounting() {
    let g = gov(vec![(
        "worker",
        ClassPolicy::new()
            .max_inflight(8)
            .cpu_units(2)
            .memory_units(3),
    )]);

    // Two children of the same root on distinct declared lineage stages.
    let c1 =
        TaskSpec::blocking(TaskClass::new("worker")).child_of("root-1", TaskStage::new("stage-a"));
    let c2 = TaskSpec::cpu(TaskClass::new("worker")).child_of("root-1", TaskStage::new("stage-b"));

    assert!(matches!(
        g.admit(&c1, RequestKey::new("root-1")),
        AdmissionDecision::Admitted { .. }
    ));
    assert!(matches!(
        g.admit(&c2, RequestKey::new("root-1")),
        AdmissionDecision::Admitted { .. }
    ));

    let root = g.root_attribution("root-1").expect("root tracked");
    assert_eq!(root.child_inflight, 2);
    assert_eq!(root.cpu_units, 4);
    assert_eq!(root.memory_units, 6);
    assert_eq!(root.active_stages, 2); // "stage-a" and "stage-b"
}

#[test]
fn child_saturation_bubbles_to_root_verdict() {
    let g = gov(vec![(
        "worker",
        ClassPolicy::new().max_inflight(1).cpu_units(1),
    )]);
    // First child occupies the "stage-a" lineage point.
    let c1 =
        TaskSpec::blocking(TaskClass::new("worker")).child_of("root-2", TaskStage::new("stage-a"));
    // Second child declares a distinct stage ("stage-b"), so it is not recursive —
    // it instead saturates the class cap, and that verdict bubbles up.
    let c2 = TaskSpec::cpu(TaskClass::new("worker")).child_of("root-2", TaskStage::new("stage-b"));
    assert!(matches!(
        g.admit(&c1, RequestKey::new("root-2")),
        AdmissionDecision::Admitted { .. }
    ));
    assert!(matches!(
        g.admit(&c2, RequestKey::new("root-2")),
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
    ));
}

#[test]
fn root_attribution_clears_on_release() {
    let g = gov(vec![(
        "worker",
        ClassPolicy::new().max_inflight(4).cpu_units(1),
    )]);
    let permit = match g.admit(&child("worker", "root-3", "s"), RequestKey::new("root-3")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    assert!(g.root_attribution("root-3").is_some());
    g.release(permit);
    assert!(g.root_attribution("root-3").is_none());
}
