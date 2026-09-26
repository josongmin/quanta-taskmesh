//! Inferno · property fuzz: thousands of randomized class/discipline/queue
//! configurations, each asserted against the invariants that must hold for
//! *every* fairness discipline (and any mix of them):
//!
//!   P1 conservation — every queued request is dispatched exactly once.
//!   P2 within-class FIFO — a class's requests dispatch in arrival order,
//!      regardless of the cross-class discipline.
//!   P3 no permanent starvation follows from P1 for every queued class.
//!   P4 determinism — identical config ⇒ identical dispatch order.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use taskmesh_bench::workload::{fixture, root_spec, Fixture};
use taskmesh_contract::{ClassPolicy, FairnessPolicy, OverflowPolicy};
use taskmesh_engine::{AdmissionDecision, ClaimOutcome, ReleaseOutcome};

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
    let filler = match g.admit(&root_spec(&names[0], "filler")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("filler must occupy the slot, got {o:?}"),
    };
    let mut tickets: Vec<(taskmesh_engine::Ticket, usize)> = Vec::new();
    for (i, &cidx) in queue.iter().enumerate() {
        let op = format!("q{i}");
        match g.admit(&root_spec(&names[cidx], &op)) {
            AdmissionDecision::Queued { ticket } => tickets.push((ticket, i)),
            o => panic!("request {i} must queue, got {o:?}"),
        }
    }
    let mut order = Vec::with_capacity(queue.len());
    let mut current = filler;
    loop {
        assert_eq!(g.release(current), ReleaseOutcome::Released);
        let mut found = None;
        for (ticket, idx) in &tickets {
            match g.claim(*ticket) {
                ClaimOutcome::Ready(permit) => {
                    found = Some((permit, *idx, *ticket));
                    break;
                }
                ClaimOutcome::Pending => {}
                other => panic!("property benchmark ticket must not terminate: {other:?}"),
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
    // The loop released `current` before finding nothing further to promote.
    assert_eq!(g.release(current), ReleaseOutcome::UnknownPermit);
    order
}

fn build(nclasses: usize, disciplines_seed: u64) -> (Fixture, Vec<String>) {
    let mut prng = StdRng::seed_from_u64(disciplines_seed);
    let names: Vec<String> = (0..nclasses).map(|i| format!("cls{i}")).collect();
    let policies: Vec<ClassPolicy> = (0..nclasses).map(|_| rand_policy(&mut prng)).collect();
    let classes: Vec<(&str, ClassPolicy)> = names
        .iter()
        .zip(policies)
        .map(|(n, p)| (n.as_str(), p))
        .collect();
    (fixture(classes, 1, 0), names)
}

fn check_fairness_seeds(seeds: std::ops::Range<u64>) {
    for seed in seeds {
        let mut rng = StdRng::seed_from_u64(seed);
        let nclasses = rng.gen_range(2..=4);
        let qlen = rng.gen_range(1..=30);
        let queue: Vec<usize> = (0..qlen).map(|_| rng.gen_range(0..nclasses)).collect();
        let disc_seed = rng.gen();

        let (fx, names) = build(nclasses, disc_seed);
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

        // P4 determinism: rebuild + rerun must reproduce the order byte-for-byte.
        let (fx2, names2) = build(nclasses, disc_seed);
        let order2 = drain_indices(&fx2, &names2, &queue);
        assert_eq!(
            order, order2,
            "seed={seed}: dispatch order is not deterministic"
        );
    }
}

// The original 3,000 seeds remain mandatory; bounded test populations share
// the same oracle and retain the normal nextest timeout.

#[test]
fn fairness_seeds_0000_0499() {
    check_fairness_seeds(0..500);
}

#[test]
fn fairness_seeds_0500_0999() {
    check_fairness_seeds(500..1000);
}

#[test]
fn fairness_seeds_1000_1499() {
    check_fairness_seeds(1000..1500);
}

#[test]
fn fairness_seeds_1500_1999() {
    check_fairness_seeds(1500..2000);
}

#[test]
fn fairness_seeds_2000_2499() {
    check_fairness_seeds(2000..2500);
}

#[test]
fn fairness_seeds_2500_2999() {
    check_fairness_seeds(2500..3000);
}
