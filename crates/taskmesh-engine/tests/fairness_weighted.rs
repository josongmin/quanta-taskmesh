//! T04: weighted-fair scheduling favors the heavier class without starving the
//! lighter one.

mod harness;
use harness::*;

use taskmesh_contract::{ClassPolicy, FairnessPolicy, OverflowPolicy};

#[test]
fn weighted_favors_heavy_without_starving_light() {
    let g = contended(vec![
        ("heavy", weighted_class(4)),
        ("light", weighted_class(1)),
    ]);
    let filler = admit_filler(&g);

    let mut tickets = Vec::new();
    for i in 0..5 {
        tickets.push(queue(&g, "heavy", &format!("h{i}")));
        tickets.push(queue(&g, "light", &format!("l{i}")));
    }

    g.release(filler);
    let order = drain(&g, &tickets);

    // Over the first six promotions the 4:1 weighting should give heavy the
    // clear majority, but light must appear (no starvation).
    let window = &order[..6];
    let heavy = window.iter().filter(|c| *c == "heavy").count();
    let light = window.iter().filter(|c| *c == "light").count();
    assert!(heavy >= 4, "heavy should dominate early window: {order:?}");
    assert!(light >= 1, "light must not starve: {order:?}");
    assert!(heavy > light, "{order:?}");
}

#[test]
fn weighted_dispatch_order_is_exact_virtual_finish_time() {
    // lo: weight 1, hi: weight 2, equal backlog. The virtual-finish-time order is
    // fully determined (asymmetric weights — the case a symmetric test misses):
    // hi has the smaller finish-tag increment, so it leads and interleaves ~2:1.
    let g = contended(vec![("lo", weighted_class(1)), ("hi", weighted_class(2))]);
    let filler = admit_filler(&g);

    let mut tickets = Vec::new();
    for i in 0..3 {
        tickets.push(queue(&g, "lo", &format!("lo{i}")));
    }
    for i in 0..3 {
        tickets.push(queue(&g, "hi", &format!("hi{i}")));
    }

    g.release(filler);
    let order = drain(&g, &tickets);
    assert_eq!(
        order,
        vec!["hi", "lo", "hi", "hi", "lo", "lo"],
        "WFQ must follow exact virtual-finish-time order"
    );
}

#[test]
fn wfq_burst_is_a_documented_no_op() {
    // `burst` is reserved in the contract (not yet read by the scheduler).
    // Varying it must not change dispatch at all — locks that documented promise.
    let order_with = |burst: u32| {
        let pol = |weight: u32| {
            ClassPolicy::new()
                .max_inflight(100)
                .max_queue_depth(100)
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth)
                .fairness(FairnessPolicy::WeightedFairQueue { weight, burst })
        };
        let g = contended(vec![("lo", pol(1)), ("hi", pol(2))]);
        let filler = admit_filler(&g);
        let mut tickets = Vec::new();
        for i in 0..3 {
            tickets.push(queue(&g, "lo", &format!("lo{i}")));
        }
        for i in 0..3 {
            tickets.push(queue(&g, "hi", &format!("hi{i}")));
        }
        g.release(filler);
        drain(&g, &tickets)
    };
    assert_eq!(
        order_with(0),
        order_with(9_999),
        "burst is reserved and must not affect scheduling"
    );
}
