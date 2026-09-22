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
use std::sync::{Arc, Barrier};
use std::thread;

use taskmesh_contract::{ClassPolicy, ManualClock, ResourceBudget, TaskClass, TaskSpec};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet, ReleaseOutcome};

fn governor(class: &str, policy: ClassPolicy, cpu_budget: u32) -> Arc<Governor> {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new(class.to_string()), policy);
    let resources = ResourceBudget::new()
        .cpu_units(cpu_budget)
        .memory_units(1_000_000);
    Arc::new(Governor::new_unchecked(
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
                    let mut ids = Vec::with_capacity(OPS);
                    for _ in 0..OPS {
                        match g.admit(&spec) {
                            AdmissionDecision::Admitted { permit_id } => {
                                ids.push(permit_id);
                                assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
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
    let first_start = Arc::new(Barrier::new(16));
    let first_attempt_done = Arc::new(Barrier::new(16));

    thread::scope(|scope| {
        for tid in 0..16 {
            let g = Arc::clone(&g);
            let held = Arc::clone(&held);
            let rejected = Arc::clone(&rejected);
            let first_start = Arc::clone(&first_start);
            let first_attempt_done = Arc::clone(&first_attempt_done);
            scope.spawn(move || {
                let op: &'static str = Box::leak(format!("c{tid}").into_boxed_str());
                let spec = TaskSpec::blocking(TaskClass::new("capped")).operation(op);
                // Force one fully overlapping wave. Admitted permits remain held
                // until every thread has attempted admission, so a serial OS
                // schedule cannot turn the rejection oracle into a flaky guess.
                first_start.wait();
                let first_permit = match g.admit(&spec) {
                    AdmissionDecision::Admitted { permit_id } => {
                        let now = held.fetch_add(1, Ordering::SeqCst) + 1;
                        assert!(now <= CAP as i64, "concurrent permits {now} exceeded cap {CAP}");
                        Some(permit_id)
                    }
                    AdmissionDecision::Rejected(_) => {
                        rejected.fetch_add(1, Ordering::SeqCst);
                        None
                    }
                    AdmissionDecision::Queued { .. } => {
                        panic!("no queue configured; admit must admit or reject")
                    }
                };
                first_attempt_done.wait();
                if let Some(permit_id) = first_permit {
                    held.fetch_sub(1, Ordering::SeqCst);
                    assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
                }

                for _ in 1..1_000 {
                    match g.admit(&spec) {
                        AdmissionDecision::Admitted { permit_id } => {
                            // Incremented only while the permit is held; bounded by
                            // the governor's own inflight, which is capped.
                            let now = held.fetch_add(1, Ordering::SeqCst) + 1;
                            assert!(
                                now <= CAP as i64,
                                "concurrent permits {now} exceeded cap {CAP}"
                            );
                            held.fetch_sub(1, Ordering::SeqCst);
                            assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
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
    // The synchronized first wave admits at most CAP of the 16 attempts.
    assert!(
        rejected.load(Ordering::SeqCst) >= 16 - i64::from(CAP),
        "the synchronized first wave must reject every attempt beyond the cap"
    );
    let snap = g.snapshot();
    assert_eq!(
        snap.classes
            .get(&TaskClass::new("capped"))
            .map_or(0, |c| c.inflight),
        0
    );
}
