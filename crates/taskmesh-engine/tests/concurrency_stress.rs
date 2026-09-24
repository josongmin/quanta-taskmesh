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
    let maximum_held = Arc::new(AtomicI64::new(0));
    let rejected = Arc::new(AtomicI64::new(0));
    let unexpectedly_queued = Arc::new(AtomicI64::new(0));
    let failed_releases = Arc::new(AtomicI64::new(0));
    let first_start = Arc::new(Barrier::new(16));
    let first_attempt_done = Arc::new(Barrier::new(16));
    let first_release_done = Arc::new(Barrier::new(16));

    thread::scope(|scope| {
        for tid in 0..16 {
            let g = Arc::clone(&g);
            let held = Arc::clone(&held);
            let maximum_held = Arc::clone(&maximum_held);
            let rejected = Arc::clone(&rejected);
            let unexpectedly_queued = Arc::clone(&unexpectedly_queued);
            let failed_releases = Arc::clone(&failed_releases);
            let first_start = Arc::clone(&first_start);
            let first_attempt_done = Arc::clone(&first_attempt_done);
            let first_release_done = Arc::clone(&first_release_done);
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
                        maximum_held.fetch_max(now, Ordering::SeqCst);
                        Some(permit_id)
                    }
                    AdmissionDecision::Rejected(_) => {
                        rejected.fetch_add(1, Ordering::SeqCst);
                        None
                    }
                    AdmissionDecision::Queued { .. } => {
                        unexpectedly_queued.fetch_add(1, Ordering::SeqCst);
                        None
                    }
                };
                first_attempt_done.wait();
                if let Some(permit_id) = first_permit {
                    if g.release(permit_id) != ReleaseOutcome::Released {
                        failed_releases.fetch_add(1, Ordering::SeqCst);
                    }
                    held.fetch_sub(1, Ordering::SeqCst);
                }
                // Do not let a second-wave admission race the mirror update for
                // the synchronized first wave. Past this point the governor's
                // own snapshot is the authority for concurrent occupancy.
                first_release_done.wait();

                for _ in 1..1_000 {
                    match g.admit(&spec) {
                        AdmissionDecision::Admitted { permit_id } => {
                            let now = g.snapshot().classes[&TaskClass::new("capped")].inflight;
                            assert!(now <= CAP, "concurrent permits {now} exceeded cap {CAP}");
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
    assert_eq!(
        maximum_held.load(Ordering::SeqCst),
        i64::from(CAP),
        "the synchronized first wave must fill, but never exceed, cap {CAP}"
    );
    assert_eq!(
        unexpectedly_queued.load(Ordering::SeqCst),
        0,
        "no queue is configured; first-wave admission must admit or reject"
    );
    assert_eq!(
        failed_releases.load(Ordering::SeqCst),
        0,
        "every first-wave permit must release exactly once"
    );
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
