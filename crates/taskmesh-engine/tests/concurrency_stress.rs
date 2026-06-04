//! Concurrency stress over the *real* `Governor` (ADR 9000 / P6, real-code track).
//!
//! loom (`loom_governance.rs`) exhaustively checks the concurrency *design* on a
//! faithful model; this exercises the actual governor under many OS threads and
//! asserts the safety invariants that must hold regardless of interleaving:
//! permit ids are globally unique (no double-grant), inflight returns to zero,
//! and the `max_inflight` cap is never exceeded concurrently. These invariants
//! are scheduling-independent, so the test is not flaky.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::thread;

use taskmesh_contract::{ClassPolicy, ManualClock, ResourceBudget, TaskClass, TaskSpec};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet, RequestKey};

fn governor(class: &str, policy: ClassPolicy, cpu_budget: u32) -> Arc<Governor> {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new(class.to_string()), policy);
    let resources = ResourceBudget::new()
        .cpu_units(cpu_budget)
        .memory_units(1_000_000);
    Arc::new(Governor::new(
        PolicySet::new(resources, classes),
        Arc::new(ManualClock::new(0)),
    ))
}

#[test]
fn concurrent_admit_release_grants_globally_unique_permits() {
    // Uncontended capacity: every admit succeeds. Each thread uses a distinct,
    // statically-keyed op so the recursion guard never collides; within a thread
    // the op is reused sequentially (release clears the guard).
    let g = governor(
        "work",
        ClassPolicy::new().max_inflight(u32::MAX).cpu_units(0),
        0,
    );
    const THREADS: usize = 8;
    const OPS: usize = 500;

    let all: Vec<Vec<u64>> = thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|tid| {
                let g = Arc::clone(&g);
                scope.spawn(move || {
                    let op: &'static str = Box::leak(format!("w{tid}").into_boxed_str());
                    let spec = TaskSpec::blocking(TaskClass::new("work")).operation(op);
                    let key = RequestKey::new(op);
                    let mut ids = Vec::with_capacity(OPS);
                    for _ in 0..OPS {
                        match g.admit(&spec, key.clone()) {
                            AdmissionDecision::Admitted { permit_id } => {
                                ids.push(permit_id);
                                g.release(permit_id);
                            }
                            other => panic!("admit must succeed, got {other:?}"),
                        }
                    }
                    ids
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Every permit id ever granted is globally unique (no double-grant).
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0;
    for ids in &all {
        for &id in ids {
            assert!(seen.insert(id), "duplicate permit id {id}");
            total += 1;
        }
    }
    assert_eq!(total, THREADS * OPS);
    // All permits released → no class holds inflight.
    let snap = g.snapshot();
    let inflight = snap
        .classes
        .get(&TaskClass::new("work"))
        .map_or(0, |c| c.inflight);
    assert_eq!(inflight, 0, "all permits must be released");
}

#[test]
fn inflight_cap_is_never_exceeded_under_contention() {
    // Hard cap of 4; reject (no queue) on overflow. A shared counter mirrors the
    // permits actually held and must never exceed the cap — the governor must not
    // grant a 5th concurrent permit no matter how admits interleave.
    const CAP: u32 = 4;
    let g = governor(
        "capped",
        ClassPolicy::new().max_inflight(CAP).cpu_units(1),
        1_000_000,
    );
    let held = Arc::new(AtomicI64::new(0));
    let rejected = Arc::new(AtomicI64::new(0));

    thread::scope(|scope| {
        for tid in 0..16 {
            let g = Arc::clone(&g);
            let held = Arc::clone(&held);
            let rejected = Arc::clone(&rejected);
            scope.spawn(move || {
                let op: &'static str = Box::leak(format!("c{tid}").into_boxed_str());
                let spec = TaskSpec::blocking(TaskClass::new("capped")).operation(op);
                let key = RequestKey::new(op);
                for _ in 0..1_000 {
                    match g.admit(&spec, key.clone()) {
                        AdmissionDecision::Admitted { permit_id } => {
                            // Incremented only while the permit is held; bounded by
                            // the governor's own inflight, which is capped.
                            let now = held.fetch_add(1, Ordering::SeqCst) + 1;
                            assert!(
                                now <= CAP as i64,
                                "concurrent permits {now} exceeded cap {CAP}"
                            );
                            held.fetch_sub(1, Ordering::SeqCst);
                            g.release(permit_id);
                        }
                        AdmissionDecision::Rejected(_) => {
                            rejected.fetch_add(1, Ordering::SeqCst);
                        }
                        AdmissionDecision::Queued { .. } => {
                            panic!("no queue configured; admit must admit or reject")
                        }
                    }
                }
            });
        }
    });

    assert_eq!(held.load(Ordering::SeqCst), 0, "all permits released");
    // Under 16 threads vs a cap of 4, contention must have produced rejections.
    assert!(
        rejected.load(Ordering::SeqCst) > 0,
        "expected some fail-closed rejections under contention"
    );
    let snap = g.snapshot();
    assert_eq!(
        snap.classes
            .get(&TaskClass::new("capped"))
            .map_or(0, |c| c.inflight),
        0
    );
}
