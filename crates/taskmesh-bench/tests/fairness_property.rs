//! Inferno · property fuzz: thousands of randomized class/discipline/queue
//! configurations, each asserted against the invariants that must hold for
//! *every* fairness discipline (and any mix of them):
//!
//!   P1 conservation — every queued request is dispatched exactly once.
//!   P2 within-class FIFO — a class's requests dispatch in arrival order,
//!      regardless of the cross-class discipline.
//!   P3 no permanent starvation — every class with queued work is served.
//!   P4 determinism — identical config ⇒ identical dispatch order.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use taskmesh_bench::workload::{fixture, root_spec, Fixture};
use taskmesh_contract::{ClassPolicy, FairnessPolicy, OverflowPolicy};
use taskmesh_engine::{AdmissionDecision, RequestKey};

fn rand_fairness(rng: &mut StdRng) -> FairnessPolicy {
    match rng.gen_range(0..5) {
        0 => FairnessPolicy::Fifo,
        1 => FairnessPolicy::WeightedFairQueue {
            weight: rng.gen_range(1..=8),
            burst: rng.gen_range(0..4),
        },
        2 => FairnessPolicy::DeficitRoundRobin {
            quantum: rng.gen_range(1..=8),
        },
        3 => FairnessPolicy::DeadlineAware {
            slack_ms: rng.gen_range(0..1_000),
        },
        _ => FairnessPolicy::BestEffortScavenger,
    }
}

fn rand_policy(rng: &mut StdRng) -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(1_000)
        .max_queue_depth(1_000)
        .cpu_units(1)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .fairness(rand_fairness(rng))
        .best_effort(rng.gen_bool(0.25))
}

/// Drain `queue` (class indices) through a single global slot and return the
/// dispatch order as the *enqueue indices* of the requests, in the order the
/// governor promoted them.
fn drain_indices(fx: &Fixture, names: &[String], queue: &[usize]) -> Vec<usize> {
    let g = &*fx.governor;
    fx.clock.set(0);
    let filler = match g.admit(&root_spec(&names[0], "filler"), RequestKey::new("filler")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("filler must occupy the slot, got {o:?}"),
    };
    let mut tickets: Vec<(u64, usize)> = Vec::new();
    for (i, &cidx) in queue.iter().enumerate() {
        let op = format!("q{i}");
        match g.admit(&root_spec(&names[cidx], &op), RequestKey::new(op)) {
            AdmissionDecision::Queued { ticket } => tickets.push((ticket, i)),
            o => panic!("request {i} must queue, got {o:?}"),
        }
    }
    let mut order = Vec::with_capacity(queue.len());
    let mut current = filler;
    loop {
        g.release(current);
        let mut found = None;
        for (ticket, idx) in &tickets {
            if let Some(permit) = g.claim(*ticket) {
                found = Some((permit, *idx, *ticket));
                break;
            }
        }
        match found {
            Some((permit, idx, ticket)) => {
                order.push(idx);
                current = permit;
                tickets.retain(|(t, _)| *t != ticket);
            }
            None => break,
        }
    }
    g.release(current);
    order
}

fn build(
    rng_seed: u64,
    nclasses: usize,
    queue: &[usize],
    disciplines_seed: u64,
) -> (Fixture, Vec<String>) {
    let mut prng = StdRng::seed_from_u64(disciplines_seed);
    let names: Vec<String> = (0..nclasses).map(|i| format!("cls{i}")).collect();
    let policies: Vec<ClassPolicy> = (0..nclasses).map(|_| rand_policy(&mut prng)).collect();
    let classes: Vec<(&str, ClassPolicy)> = names
        .iter()
        .zip(policies)
        .map(|(n, p)| (n.as_str(), p))
        .collect();
    let _ = (rng_seed, queue);
    (fixture(classes, 1, 0), names)
}

#[test]
fn random_configs_satisfy_fairness_invariants() {
    for seed in 0..3_000u64 {
        let mut rng = StdRng::seed_from_u64(seed);
        let nclasses = rng.gen_range(2..=4);
        let qlen = rng.gen_range(1..=30);
        let queue: Vec<usize> = (0..qlen).map(|_| rng.gen_range(0..nclasses)).collect();
        let disc_seed = rng.gen();

        let (fx, names) = build(seed, nclasses, &queue, disc_seed);
        let order = drain_indices(&fx, &names, &queue);

        // P1 conservation: every request dispatched exactly once.
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (0..qlen).collect::<Vec<_>>(),
            "seed={seed}: not every queued request dispatched exactly once"
        );

        // P2 within-class FIFO: a class's indices appear in increasing order.
        for c in 0..nclasses {
            let seen: Vec<usize> = order.iter().copied().filter(|&i| queue[i] == c).collect();
            let mut expect = seen.clone();
            expect.sort_unstable();
            assert_eq!(
                seen, expect,
                "seed={seed}: class {c} violated within-class FIFO: {seen:?}"
            );
        }

        // P3 no permanent starvation: every class that queued work is served.
        for c in 0..nclasses {
            if queue.contains(&c) {
                assert!(
                    order.iter().any(|&i| queue[i] == c),
                    "seed={seed}: class {c} was permanently starved"
                );
            }
        }

        // P4 determinism: rebuild + rerun must reproduce the order byte-for-byte.
        let (fx2, names2) = build(seed, nclasses, &queue, disc_seed);
        let order2 = drain_indices(&fx2, &names2, &queue);
        assert_eq!(
            order, order2,
            "seed={seed}: dispatch order is not deterministic"
        );
    }
}
