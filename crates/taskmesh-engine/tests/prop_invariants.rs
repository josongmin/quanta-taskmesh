//! Property-based invariants (ADR 9000 / P5 + SOTA gap closeout).
//!
//! 1. Resource accounting: across arbitrary admit/release sequences, per-class
//!    inflight always matches the live-permit model, caps are never exceeded, and
//!    the system drains to exactly zero. (Internal `Σpermit == held` consistency
//!    is additionally enforced by `GovernedState::assert_consistent` in debug.)
//! 2. Fairness determinism: identical queued input yields a byte-identical
//!    promotion order on independent runs (no hidden nondeterminism).

use std::collections::BTreeMap;
use std::sync::Arc;

use proptest::collection::vec;
use proptest::prelude::*;
use taskmesh_contract::*;
use taskmesh_engine::*;

fn class_policy(max_inflight: u32, cost: u32) -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(max_inflight)
        .cpu_units(cost)
        .memory_units(cost)
}

fn governor(specs: &[(&'static str, u32, u32)]) -> Governor {
    let classes: BTreeMap<TaskClass, ClassPolicy> = specs
        .iter()
        .map(|(n, mi, c)| (TaskClass::new(*n), class_policy(*mi, *c)))
        .collect();
    Governor::new(
        // Generous global budget so only the per-class inflight cap binds.
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(0)),
    )
}

const CLASSES: [&str; 3] = ["a", "b", "c"];

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    /// Arbitrary admit/release interleavings keep accounting exact and drain clean.
    #[test]
    fn accounting_is_exact_and_drains_to_zero(
        ops in vec((any::<bool>(), 0u8..3, 0u8..255), 0..80)
    ) {
        let g = governor(&[("a", 2, 1), ("b", 3, 2), ("c", 1, 3)]);
        // model of live permits: (permit_id, class_index)
        let mut live: Vec<(u64, usize)> = Vec::new();

        for (step, (is_admit, class_sel, rel_sel)) in ops.into_iter().enumerate() {
            if is_admit {
                let ci = class_sel as usize % CLASSES.len();
                let op = format!("{ci}-{step}"); // unique => no admission-key aliasing
                let spec = TaskSpec::blocking(TaskClass::new(CLASSES[ci].to_string()))
                    .operation(op.clone());
                match g.admit(&spec, RequestKey::new(op)) {
                    AdmissionDecision::Admitted { permit_id } => live.push((permit_id, ci)),
                    AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. }) => {}
                    other => prop_assert!(false, "unexpected non-queueable outcome: {other:?}"),
                }
            } else if !live.is_empty() {
                let idx = rel_sel as usize % live.len();
                let (permit, _) = live.remove(idx);
                g.release(permit);
            }

            // Invariant: snapshot inflight matches the live model, every step.
            let snap = g.snapshot();
            for (ci, cname) in CLASSES.iter().enumerate() {
                let modeled = live.iter().filter(|(_, c)| *c == ci).count() as u32;
                let observed = snap.classes[&TaskClass::new(*cname)].inflight;
                prop_assert_eq!(observed, modeled, "class {} inflight mismatch", cname);
                // Cap is never exceeded.
                let cap = [2u32, 3, 1][ci];
                prop_assert!(observed <= cap, "class {} exceeded cap {}", cname, cap);
            }
        }

        // Drain everything; the system must return to exactly zero.
        for (permit, _) in live {
            g.release(permit);
        }
        let snap = g.snapshot();
        for cname in CLASSES {
            let c = &snap.classes[&TaskClass::new(cname)];
            prop_assert_eq!(c.inflight, 0);
            prop_assert_eq!(c.cpu_units_held, 0);
            prop_assert_eq!(c.memory_units_held, 0);
        }
    }
}

/// Drain a contended weighted-fair governor and return the class promotion order.
fn weighted_drain(classes_seq: &[usize], weights: [u32; 3]) -> Vec<usize> {
    let mut classes: BTreeMap<TaskClass, ClassPolicy> = BTreeMap::new();
    for (ci, name) in CLASSES.iter().enumerate() {
        classes.insert(
            TaskClass::new(*name),
            ClassPolicy::new()
                .max_inflight(100_000)
                .max_queue_depth(1_000_000)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::WeightedFairQueue {
                    weight: weights[ci],
                    burst: 0,
                }),
        );
    }
    classes.insert(
        TaskClass::new("fill"),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(1)
            .cpu_units(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    );
    // Single global unit -> one inflight -> fully observable promotion order.
    let resources = ResourceBudget::new().cpu_units(1).memory_units(1_000_000);
    let g = Governor::new(
        PolicySet::new(resources, classes),
        Arc::new(ManualClock::new(0)),
    );

    let blocking = |class: &str, op: &str| {
        TaskSpec::blocking(TaskClass::new(class.to_string())).operation(op.to_string())
    };
    let fill = match g.admit(&blocking("fill", "f"), RequestKey::new("f")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        _ => unreachable!(),
    };

    let mut tickets: Vec<(u64, usize)> = Vec::new();
    for (i, &ci) in classes_seq.iter().enumerate() {
        let op = format!("{ci}-{i}");
        if let AdmissionDecision::Queued { ticket } =
            g.admit(&blocking(CLASSES[ci], &op), RequestKey::new(op))
        {
            tickets.push((ticket, ci));
        }
    }
    g.release(fill);

    let mut order = Vec::new();
    while !tickets.is_empty() {
        let mut found = None;
        for (idx, (ticket, _)) in tickets.iter().enumerate() {
            if let Some(permit) = g.claim(*ticket) {
                found = Some((idx, permit));
                break;
            }
        }
        let Some((idx, permit)) = found else { break };
        let (_, ci) = tickets.remove(idx);
        order.push(ci);
        g.release(permit);
    }
    order
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 120, ..ProptestConfig::default() })]

    /// Identical queued input + weights => identical promotion order, every run.
    #[test]
    fn fairness_promotion_order_is_deterministic(
        seq in vec(0usize..3, 1..40),
        w0 in 1u32..8, w1 in 1u32..8, w2 in 1u32..8,
    ) {
        let weights = [w0, w1, w2];
        let first = weighted_drain(&seq, weights);
        let second = weighted_drain(&seq, weights);
        prop_assert_eq!(first, second);
    }
}
