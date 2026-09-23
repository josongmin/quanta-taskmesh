//! The conservation oracle must be able to say *no*.
//!
//! Every engine test asserts `conservation_violation() == None` on a snapshot
//! the engine produced, which proves only that consistent data is accepted. An
//! oracle that returned `None` for everything would pass every one of them.
//! These tests hand it inconsistent projections built by hand and require the
//! specific identity it names, so the accepting direction is backed by a
//! detecting one.

use std::collections::BTreeMap;

use taskmesh_contract::{CapabilityUsage, ClassSnapshot, Snapshot, TaskClass};

/// A projection that satisfies every identity: two live requests in distinct
/// phases, one of them started, one earlier request terminated.
fn consistent() -> ClassSnapshot {
    ClassSnapshot {
        inflight: 2,
        queued: 1,
        cpu_units_held: 2,
        memory_units_held: 4,
        dispatch_reserved: 1,
        accepted: 0,
        running: 1,
        cleanup_pending: 0,
        admitted_total: 3,
        started_total: 2,
        terminated_total: 1,
    }
}

fn violation_of(class: &ClassSnapshot) -> String {
    class
        .conservation_violation()
        .expect("an inconsistent projection must be reported")
}

#[test]
fn a_consistent_projection_is_accepted() {
    assert_eq!(consistent().conservation_violation(), None);
    assert_eq!(
        ClassSnapshot::default().conservation_violation(),
        None,
        "an idle class holds nothing and violates nothing"
    );
}

#[test]
fn inflight_must_equal_the_phase_sum() {
    // Two live requests, but the phase gauges account for only one of them:
    // one request is holding capacity in no phase at all.
    let violation = violation_of(&ClassSnapshot {
        inflight: 2,
        dispatch_reserved: 1,
        running: 0,
        ..consistent()
    });
    assert!(
        violation.contains("inflight 2 != phase sum 1"),
        "the report names the phase-sum identity, got {violation:?}"
    );
}

#[test]
fn phase_sum_adds_dispatch_reserved_and_accepted_together() {
    // Admission may have reserved the dispatch slot while a previously
    // dispatched request is already accepted. Both phase gauges contribute
    // independently to the inflight partition.
    let class = ClassSnapshot {
        inflight: 2,
        dispatch_reserved: 1,
        accepted: 1,
        admitted_total: 2,
        ..ClassSnapshot::default()
    };
    assert_eq!(class.conservation_violation(), None);
}

#[test]
fn admitted_total_must_equal_inflight_plus_terminated() {
    // Three admitted, two live, none terminated: one request left without
    // being counted as terminated — or was never live in the first place.
    let violation = violation_of(&ClassSnapshot {
        admitted_total: 3,
        inflight: 2,
        dispatch_reserved: 1,
        running: 1,
        terminated_total: 0,
        ..consistent()
    });
    assert!(
        violation.contains("admitted_total 3 != inflight 2 + terminated_total 0"),
        "the report names the admitted-total identity, got {violation:?}"
    );
}

#[test]
fn started_total_must_cover_every_running_or_cleaning_request() {
    // A request is running, but the cumulative started counter never saw it.
    let violation = violation_of(&ClassSnapshot {
        inflight: 2,
        dispatch_reserved: 0,
        running: 1,
        cleanup_pending: 1,
        started_total: 1,
        ..consistent()
    });
    assert!(
        violation.contains("started_total 1 < live started 2"),
        "the report names the started-total identity, got {violation:?}"
    );
}

#[test]
fn a_capability_pool_cannot_exceed_its_limit() {
    let mut snapshot = Snapshot::default();
    snapshot.capabilities.insert(
        "blocking".to_string(),
        CapabilityUsage {
            in_use: 2,
            limit: 1,
        },
    );
    let violation = snapshot
        .conservation_violation()
        .expect("an over-occupied pool must be reported");
    assert!(
        violation.contains("capability blocking: in_use 2 exceeds limit 1"),
        "the report names the pool and the overrun, got {violation:?}"
    );
}

#[test]
fn a_full_capability_pool_is_within_its_limit() {
    // The limit is inclusive: every slot in use is exactly full, not over.
    let mut snapshot = Snapshot::default();
    snapshot.capabilities.insert(
        "blocking".to_string(),
        CapabilityUsage {
            in_use: 1,
            limit: 1,
        },
    );
    // And `0` keeps its published meaning — ungated — however busy the pool is.
    snapshot.capabilities.insert(
        "io".to_string(),
        CapabilityUsage {
            in_use: 500,
            limit: 0,
        },
    );
    assert_eq!(snapshot.conservation_violation(), None);
}

#[test]
fn a_snapshot_reports_the_violating_class_by_name() {
    // The whole-snapshot check runs every class and prefixes the offender, so
    // a multi-class runtime says *which* class broke conservation.
    let mut classes: BTreeMap<TaskClass, ClassSnapshot> = BTreeMap::new();
    classes.insert(TaskClass::new("fine"), consistent());
    classes.insert(
        TaskClass::new("broken"),
        ClassSnapshot {
            inflight: 1,
            ..ClassSnapshot::default()
        },
    );
    let snapshot = Snapshot {
        classes,
        ..Snapshot::default()
    };
    let violation = snapshot
        .conservation_violation()
        .expect("a broken class must be reported");
    assert!(
        violation.starts_with("class broken: "),
        "the report names the offending class, got {violation:?}"
    );
    assert!(
        violation.contains("inflight 1 != phase sum 0"),
        "and carries the class-level identity, got {violation:?}"
    );

    // A fully consistent multi-class snapshot is accepted.
    let mut classes: BTreeMap<TaskClass, ClassSnapshot> = BTreeMap::new();
    classes.insert(TaskClass::new("a"), consistent());
    classes.insert(TaskClass::new("b"), ClassSnapshot::default());
    let snapshot = Snapshot {
        classes,
        ..Snapshot::default()
    };
    assert_eq!(snapshot.conservation_violation(), None);
}
