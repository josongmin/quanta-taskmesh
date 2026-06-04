//! T05: memory overcommit — reject / queue / degrade-to-light paths.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov(resources: ResourceBudget, list: Vec<(&'static str, ClassPolicy)>) -> Governor {
    let classes: BTreeMap<TaskClass, ClassPolicy> = list
        .into_iter()
        .map(|(n, p)| (TaskClass::new(n), p))
        .collect();
    Governor::new_unchecked(
        PolicySet::new(resources, classes),
        Arc::new(ManualClock::new(1000)),
    )
}

fn admit(g: &Governor, class: &str, op: &str) -> AdmissionDecision {
    let spec = TaskSpec::blocking(TaskClass::new(class.to_string())).operation(op.to_string());
    g.admit(&spec, RequestKey::new(op.to_string()))
}

#[test]
fn overcommit_reject_path() {
    let g = gov(
        ResourceBudget::new().memory_units(10),
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(8)
                .memory_units(10)
                .memory_overcommit_policy(MemoryOvercommitPolicy::Reject),
        )],
    );
    assert!(matches!(
        admit(&g, "c", "a"),
        AdmissionDecision::Admitted { .. }
    ));
    assert!(matches!(
        admit(&g, "c", "b"),
        AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated { .. })
    ));
}

#[test]
fn overcommit_queue_path() {
    let g = gov(
        ResourceBudget::new().memory_units(10),
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(8)
                .max_queue_depth(4)
                .memory_units(10)
                .memory_overcommit_policy(MemoryOvercommitPolicy::Queue),
        )],
    );
    assert!(matches!(
        admit(&g, "c", "a"),
        AdmissionDecision::Admitted { .. }
    ));
    assert!(matches!(
        admit(&g, "c", "b"),
        AdmissionDecision::Queued { .. }
    ));
    assert_eq!(g.snapshot().classes[&TaskClass::new("c")].queued, 1);
}

#[test]
fn overcommit_degrade_uses_fallback_class() {
    // Budget 9: a second heavy (8) won't fit, but a light (1) will.
    let g = gov(
        ResourceBudget::new().memory_units(9),
        vec![
            (
                "heavy",
                ClassPolicy::new()
                    .max_inflight(8)
                    .memory_units(8)
                    .memory_overcommit_policy(MemoryOvercommitPolicy::DegradeToLight {
                        fallback_class: TaskClass::new("light"),
                    }),
            ),
            ("light", ClassPolicy::new().max_inflight(8).memory_units(1)),
        ],
    );
    assert!(matches!(
        admit(&g, "heavy", "h1"),
        AdmissionDecision::Admitted { .. }
    ));
    // Second heavy overcommits, degrades to light, which fits.
    assert!(matches!(
        admit(&g, "heavy", "h2"),
        AdmissionDecision::Admitted { .. }
    ));

    let snap = g.snapshot();
    assert_eq!(snap.classes[&TaskClass::new("light")].inflight, 1);
    assert_eq!(snap.classes[&TaskClass::new("light")].memory_units_held, 1);
    assert_eq!(snap.classes[&TaskClass::new("heavy")].inflight, 1);
}
