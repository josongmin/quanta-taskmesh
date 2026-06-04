//! Realized multi-class fairness (ADR 9000 / P5). Drives a contended set of
//! weighted-fair classes and scores the realized dispatch *order* with Jain's
//! index — a behavioral characterization, not just a timing.
//!
//! Fairness lives in the promotion ORDER under contention, not the final totals:
//! every queued request eventually dispatches once the contention clears, so
//! counting completions to the end always yields equal shares (Jain≈1) and would
//! hide a broken scheduler. We therefore score a bounded early window, the same
//! methodology as the engine's own `fairness_weighted` test.

use std::collections::BTreeMap;
use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use taskmesh_bench::metrics::jain_fairness_index;
use taskmesh_contract::{
    ClassPolicy, FairnessPolicy, ManualClock, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet, RequestKey};

/// Drive a contended governor (single inflight slot) and return the realized
/// promotion order — one class name per dispatch, in dispatch sequence.
fn promotion_order(weights: &[(&'static str, u32)], rounds: usize) -> Vec<&'static str> {
    let mut classes: BTreeMap<TaskClass, ClassPolicy> = weights
        .iter()
        .map(|(name, weight)| {
            (
                TaskClass::new(name.to_string()),
                ClassPolicy::new()
                    .max_inflight(100_000)
                    .max_queue_depth(1_000_000)
                    .cpu_units(1)
                    .overflow_policy(OverflowPolicy::QueueWithinDepth)
                    .fairness(FairnessPolicy::WeightedFairQueue {
                        weight: *weight,
                        burst: 0,
                    }),
            )
        })
        .collect();
    classes.insert(
        TaskClass::new("fill"),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(1)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    // Global budget of one unit → single inflight → observable promotion order.
    let resources = ResourceBudget::new().cpu_units(1).memory_units(1_000_000);
    let g = Governor::new_unchecked(
        PolicySet::new(resources, classes),
        Arc::new(ManualClock::new(0)),
    );

    let spec = |class: &str, op: &str| {
        TaskSpec::blocking(TaskClass::new(class.to_string())).operation(op.to_string())
    };
    let fill = match g.admit(&spec("fill", "f"), RequestKey::new("f")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("filler must admit: {other:?}"),
    };

    // Enqueue in per-class BURSTS (all of `a`, then all of `b`, …), not
    // round-robin. This makes fair interleaving a non-trivial scheduler property:
    // a naive FIFO-by-arrival would dispatch the whole first burst before the
    // second and fail the windowed-fairness gate, while WFQ interleaves by finish
    // tag. Round-robin enqueue would let even FIFO look fair (tautological gate).
    let mut tickets: Vec<(u64, &'static str)> = Vec::new();
    for (name, _) in weights {
        for round in 0..rounds {
            let op = format!("{name}-{round}");
            // The single budget unit is held by the filler, so every weighted
            // admit MUST queue; silently dropping a non-Queued verdict would skew
            // the realized order, so assert it.
            match g.admit(&spec(name, &op), RequestKey::new(op)) {
                AdmissionDecision::Queued { ticket } => tickets.push((ticket, name)),
                other => panic!("contended admit must queue, got {other:?}"),
            }
        }
    }

    g.release(fill);

    let mut order = Vec::with_capacity(tickets.len());
    loop {
        let mut found = None;
        for (idx, (ticket, _)) in tickets.iter().enumerate() {
            if let Some(permit) = g.claim(*ticket) {
                found = Some((idx, permit));
                break;
            }
        }
        let Some((idx, permit)) = found else { break };
        let (_, name) = tickets.remove(idx);
        order.push(name);
        g.release(permit);
    }
    order
}

/// Per-class dispatch counts within the first `window` promotions, in `classes`
/// order. This is where fairness is observable.
fn windowed_shares(order: &[&'static str], classes: &[&'static str], window: usize) -> Vec<f64> {
    let w = window.min(order.len());
    classes
        .iter()
        .map(|c| order[..w].iter().filter(|n| *n == c).count() as f64)
        .collect()
}

fn bench(c: &mut Criterion) {
    // SLO gate (non-tautological): equal-weight classes interleave fairly, so the
    // windowed Jain index must be ≥ 0.95. A broken scheduler that favored one
    // class would skew the window and fail here.
    let equal_order = promotion_order(&[("a", 1), ("b", 1), ("c", 1)], 200);
    let equal_shares = windowed_shares(&equal_order, &["a", "b", "c"], 60);
    let equal_index = jain_fairness_index(&equal_shares);
    assert!(
        equal_index >= 0.95,
        "equal-weight windowed Jain below SLO: {equal_index:.4} (shares={equal_shares:?})"
    );

    // Weighted 4:1 must favor heavy in the early window without starving light.
    let weighted_order = promotion_order(&[("heavy", 4), ("light", 1)], 200);
    let weighted_shares = windowed_shares(&weighted_order, &["heavy", "light"], 50);
    assert!(
        weighted_shares[0] > weighted_shares[1],
        "heavy must dominate the early window: {weighted_shares:?}"
    );
    assert!(
        weighted_shares[1] > 0.0,
        "light must not starve: {weighted_shares:?}"
    );
    println!(
        "weighted windowed shares (heavy:light) = {weighted_shares:?}  windowed Jain = {:.4}",
        jain_fairness_index(&weighted_shares)
    );

    c.bench_function("fairness_promotion_order", |b| {
        b.iter(|| black_box(promotion_order(&[("a", 1), ("b", 1), ("c", 1)], 200).len()));
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
