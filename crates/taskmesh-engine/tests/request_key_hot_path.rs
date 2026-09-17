use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use taskmesh_contract::{
    ClassPolicy, ManualClock, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    request_key_derive_count, reset_request_key_derive_count, AdmissionDecision, Governor,
    PolicySet,
};

static REQUEST_KEY_COUNTER_LOCK: Mutex<()> = Mutex::new(());

fn gov(policy: ClassPolicy) -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new("retrieval"), policy);
    Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(0)),
    )
}

#[test]
fn direct_admit_does_not_derive_request_key() {
    let _guard = REQUEST_KEY_COUNTER_LOCK.lock().expect("lock");
    reset_request_key_derive_count();

    let g = gov(ClassPolicy::new()
        .max_inflight(8)
        .cpu_units(1)
        .memory_units(1));
    let spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("op");

    assert!(matches!(g.admit(&spec), AdmissionDecision::Admitted { .. }));
    assert_eq!(request_key_derive_count(), 0);
}

#[test]
fn unknown_class_reject_does_not_derive_request_key() {
    let _guard = REQUEST_KEY_COUNTER_LOCK.lock().expect("lock");
    reset_request_key_derive_count();

    let g = gov(ClassPolicy::new().max_inflight(8));
    let spec = TaskSpec::blocking(TaskClass::new("ghost")).operation("op");

    assert!(matches!(
        g.admit(&spec),
        AdmissionDecision::Rejected(taskmesh_contract::AdmissionVerdict::UnknownClass { .. })
    ));
    assert_eq!(request_key_derive_count(), 0);
}

#[test]
fn registered_capability_names_are_interned_once() {
    // Admission resolves a request's pool to a shared name. For a registered
    // pool that is a clone of one interned `Arc` — the same allocation every
    // time — which is what keeps the governance hot path allocation-free for
    // every built-in hint. A pool no substrate provides has nothing interned.
    let g = gov(ClassPolicy::new().max_inflight(8));
    let first = g
        .policy()
        .capability_name("blocking")
        .expect("the built-in blocking pool is registered, so its name is interned");
    let second = g
        .policy()
        .capability_name("blocking")
        .expect("a second lookup finds the same registered pool");
    assert!(
        Arc::ptr_eq(&first, &second),
        "two lookups of a registered pool must share one interned allocation"
    );
    assert_eq!(&*first, "blocking");
    assert!(
        g.policy().capability_name("gpu").is_none(),
        "an unregistered pool has no interned name"
    );
}

#[test]
fn queued_request_derives_request_key_once() {
    let _guard = REQUEST_KEY_COUNTER_LOCK.lock().expect("lock");
    reset_request_key_derive_count();

    let g = gov(ClassPolicy::new()
        .max_inflight(1)
        .max_queue_depth(4)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .cpu_units(1)
        .memory_units(1));
    let first = TaskSpec::blocking(TaskClass::new("retrieval")).operation("a");
    let second = TaskSpec::blocking(TaskClass::new("retrieval")).operation("b");

    assert!(matches!(
        g.admit(&first),
        AdmissionDecision::Admitted { .. }
    ));
    assert_eq!(request_key_derive_count(), 0);

    assert!(matches!(g.admit(&second), AdmissionDecision::Queued { .. }));
    assert_eq!(request_key_derive_count(), 1);
}
