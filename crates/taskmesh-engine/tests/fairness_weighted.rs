//! T04: weighted-fair scheduling favors the heavier class without starving the
//! lighter one.

mod harness;
use harness::*;

use taskmesh_contract::{ClassPolicy, FairnessPolicy, OverflowPolicy};
use taskmesh_engine::Governor;

/// Release the current holder, claim whoever the governor just promoted, record
/// its class, and return the new permit. Returns `None` when nothing promotes.
fn step(
    g: &Governor,
    current: u64,
    pool: &mut Vec<(u64, String)>,
    order: &mut Vec<String>,
) -> Option<u64> {
    g.release(current);
    for i in 0..pool.len() {
        if let Some(permit) = g.claim(pool[i].0) {
            order.push(pool[i].1.clone());
            pool.remove(i);
            return Some(permit);
        }
    }
    None
}

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

#[test]
fn wfq_idle_reset_prevents_a_returning_class_from_jumping_the_queue() {
    // Both equal weight. `a` monopolizes while `b` is idle, advancing virtual
    // time. When `b` finally arrives, WFQ's `last_finish_tag.max(virtual_time)`
    // pins b's finish tag to *now*, so b slots into its fair position — it does
    // NOT jump to the front to "make up" for idle time, and it is NOT starved.
    let g = contended(vec![("a", weighted_class(1)), ("b", weighted_class(1))]);
    let filler = admit_filler(&g);

    let mut pool: Vec<(u64, String)> = Vec::new();
    for i in 0..4 {
        pool.push(queue(&g, "a", &format!("a{i}")));
    }

    let mut order = Vec::new();
    // Dispatch two `a`s — virtual time advances to ~2 quanta while b is idle.
    let mut current = step(&g, filler, &mut pool, &mut order).expect("a0 promotes");
    current = step(&g, current, &mut pool, &mut order).expect("a1 promotes");

    // `b` arrives only now, after virtual time has moved on.
    pool.push(queue(&g, "b", "b0"));

    // Drain the rest.
    while let Some(next) = step(&g, current, &mut pool, &mut order) {
        current = next;
    }
    g.release(current);

    // b lands between a2 and a3 — its fair slot — not at the front.
    assert_eq!(
        order,
        vec!["a", "a", "a", "b", "a"],
        "idle-reset must place a returning class fairly, not ahead of in-flight backlog"
    );
}
