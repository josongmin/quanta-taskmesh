//! T04: the adaptive retry-after formula, locked exactly across the whole
//! (inflight × queue-depth) grid and for every fairness adjustment.
//!
//! The four point-tests in `retry_after.rs` pin single values; this exhausts the
//! grid so the closed form `50 + 10·queue + 5·inflight + fairness_adj` cannot
//! drift at any contention level, and asserts monotonicity + determinism.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    ClassPolicy, FairnessPolicy, ManualClock, OverflowPolicy, ResourceBudget, RetryAfterPolicy,
    TaskClass, TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet};

/// Build an exact `(inflight = n, queued = d)` state on one adaptive class, then
/// admit one more request and return the retry-after hint of its rejection.
fn hint(fairness: FairnessPolicy, n: u32, d: u32) -> u64 {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new()
            .max_inflight(n)
            .max_queue_depth(d)
            .overflow_policy(OverflowPolicy::QueueWithinDepth)
            .retry_after_policy(RetryAfterPolicy::Adaptive)
            .fairness(fairness),
    );
    // `new_unchecked`: a depth-0 queueable class is intentionally invalid config
    // but a legitimate probe point for the formula.
    let g = Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(1000)),
    );
    let spec = TaskSpec::blocking(TaskClass::new("c")).operation("op");

    for _ in 0..n {
        assert!(matches!(g.admit(&spec), AdmissionDecision::Admitted { .. }));
    }
    for _ in 0..d {
        assert!(matches!(g.admit(&spec), AdmissionDecision::Queued { .. }));
    }
    match g.admit(&spec) {
        AdmissionDecision::Rejected(v) => v.retry_after_ms().expect("adaptive yields a hint"),
        o => panic!("expected a full-queue rejection at n={n} d={d}, got {o:?}"),
    }
}

#[test]
fn adaptive_formula_is_exact_across_the_contention_grid() {
    for n in 1..=8u32 {
        for d in 0..=8u32 {
            let base = 50 + 10 * u64::from(d) + 5 * u64::from(n);
            assert_eq!(hint(FairnessPolicy::Fifo, n, d), base, "fifo n={n} d={d}");
            assert_eq!(
                hint(FairnessPolicy::DeficitRoundRobin { quantum: 3 }, n, d),
                base + 3,
                "drr quantum step n={n} d={d}"
            );
            assert_eq!(
                hint(FairnessPolicy::DeadlineAware { slack_ms: 100 }, n, d),
                base + 10,
                "deadline slack/10 n={n} d={d}"
            );
            assert_eq!(
                hint(FairnessPolicy::BestEffortScavenger, n, d),
                base + 200,
                "scavenger penalty n={n} d={d}"
            );
            assert_eq!(
                hint(
                    FairnessPolicy::WeightedFairQueue {
                        weight: 4,
                        burst: 0
                    },
                    n,
                    d
                ),
                base.saturating_sub(8),
                "wfq weight relief n={n} d={d}"
            );
        }
    }
}

#[test]
fn adaptive_hint_is_strictly_monotonic_and_deterministic() {
    // Strictly increasing in queue depth (fixed inflight), and reproducible.
    for n in 1..=6u32 {
        let mut prev = 0u64;
        for d in 0..=6u32 {
            let h = hint(FairnessPolicy::Fifo, n, d);
            assert_eq!(
                h,
                hint(FairnessPolicy::Fifo, n, d),
                "deterministic n={n} d={d}"
            );
            assert!(d == 0 || h > prev, "monotonic in queue depth: n={n} d={d}");
            prev = h;
        }
    }
    // Strictly increasing in inflight (fixed queue depth).
    for d in 0..=6u32 {
        let mut prev = 0u64;
        for n in 1..=6u32 {
            let h = hint(FairnessPolicy::Fifo, n, d);
            assert!(h > prev, "monotonic in inflight: n={n} d={d}");
            prev = h;
        }
    }
}
