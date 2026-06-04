//! T03: bounded admission and queue behavior.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov(resources: ResourceBudget, list: Vec<(&'static str, ClassPolicy)>) -> Governor {
    let classes: BTreeMap<TaskClass, ClassPolicy> = list
        .into_iter()
        .map(|(n, p)| (TaskClass::new(n), p))
        .collect();
    Governor::new(
        PolicySet::new(resources, classes),
        Arc::new(ManualClock::new(1000)),
    )
}

fn spec(class: &str) -> TaskSpec {
    TaskSpec::blocking(TaskClass::new(class.to_string())).operation(format!("op:{class}"))
}

fn key(s: &str) -> RequestKey {
    RequestKey::new(s.to_string())
}

#[test]
fn unknown_class_rejects() {
    let g = gov(ResourceBudget::new(), vec![("known", ClassPolicy::new())]);
    match g.admit(&spec("ghost"), key("k")) {
        AdmissionDecision::Rejected(AdmissionVerdict::UnknownClass { class }) => {
            assert_eq!(class.as_str(), "ghost");
        }
        other => panic!("expected UnknownClass, got {other:?}"),
    }
}

#[test]
fn disabled_class_rejects() {
    let g = gov(
        ResourceBudget::new(),
        vec![("c", ClassPolicy::new().max_inflight(0))],
    );
    assert!(matches!(
        g.admit(&spec("c"), key("k")),
        AdmissionDecision::Rejected(AdmissionVerdict::ClassDisabled)
    ));
}

#[test]
fn inflight_below_cap_admits() {
    let g = gov(
        ResourceBudget::new(),
        vec![("c", ClassPolicy::new().max_inflight(2))],
    );
    assert!(matches!(
        g.admit(&spec("c"), key("k")),
        AdmissionDecision::Admitted { .. }
    ));
}

#[test]
fn inflight_at_cap_queueable_queues() {
    let g = gov(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(4)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )],
    );
    assert!(matches!(
        g.admit(&spec("c"), key("a")),
        AdmissionDecision::Admitted { .. }
    ));
    assert!(matches!(
        g.admit(&spec("c"), key("b")),
        AdmissionDecision::Queued { .. }
    ));
    assert_eq!(g.snapshot().classes[&TaskClass::new("c")].queued, 1);
}

#[test]
fn inflight_at_cap_non_queueable_rejects() {
    let g = gov(
        ResourceBudget::new(),
        vec![("c", ClassPolicy::new().max_inflight(1))],
    );
    assert!(matches!(
        g.admit(&spec("c"), key("a")),
        AdmissionDecision::Admitted { .. }
    ));
    assert!(matches!(
        g.admit(&spec("c"), key("b")),
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
    ));
}

#[test]
fn queue_depth_exceeded_returns_queue_full() {
    let g = gov(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )],
    );
    assert!(matches!(
        g.admit(&spec("c"), key("a")),
        AdmissionDecision::Admitted { .. }
    ));
    assert!(matches!(
        g.admit(&spec("c"), key("b")),
        AdmissionDecision::Queued { .. }
    ));
    assert!(matches!(
        g.admit(&spec("c"), key("d")),
        AdmissionDecision::Rejected(AdmissionVerdict::QueueFull { .. })
    ));
}

#[test]
fn release_promotes_queued_work() {
    let g = gov(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(4)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )],
    );
    let permit = match g.admit(&spec("c"), key("a")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admit, got {other:?}"),
    };
    let ticket = match g.admit(&spec("c"), key("b")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("expected queue, got {other:?}"),
    };
    // Before release: 1 inflight, 1 queued.
    let snap = g.snapshot().classes[&TaskClass::new("c")].clone();
    assert_eq!(snap.inflight, 1);
    assert_eq!(snap.queued, 1);

    g.release(permit);

    // After release: queued work promoted to inflight; claimable by ticket.
    let snap = g.snapshot().classes[&TaskClass::new("c")].clone();
    assert_eq!(snap.inflight, 1);
    assert_eq!(snap.queued, 0);
    assert!(g.claim(ticket).is_some());
}

#[test]
fn queued_tickets_are_stably_ordered() {
    let g = gov(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new()
                .max_inflight(1)
                .max_queue_depth(8)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        )],
    );
    let _ = g.admit(&spec("c"), key("a"));
    let t1 = match g.admit(&spec("c"), key("b")) {
        AdmissionDecision::Queued { ticket } => ticket,
        o => panic!("{o:?}"),
    };
    let t2 = match g.admit(&spec("c"), key("c")) {
        AdmissionDecision::Queued { ticket } => ticket,
        o => panic!("{o:?}"),
    };
    assert!(t1 < t2, "tickets are monotonically increasing");
}
