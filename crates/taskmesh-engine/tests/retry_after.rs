//! T04: retry-after hints — fixed exactness and deterministic adaptive formula.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn gov(policy: ClassPolicy) -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new("c"), policy);
    Governor::new(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(1000)),
    )
}

fn spec() -> TaskSpec {
    TaskSpec::blocking(TaskClass::new("c")).operation("op")
}

fn saturate_then_reject(g: &Governor) -> AdmissionVerdict {
    assert!(matches!(
        g.admit(&spec(), RequestKey::new("a")),
        AdmissionDecision::Admitted { .. }
    ));
    match g.admit(&spec(), RequestKey::new("b")) {
        AdmissionDecision::Rejected(v) => v,
        other => panic!("expected rejection, got {other:?}"),
    }
}

#[test]
fn fixed_retry_after_is_exact() {
    let g = gov(ClassPolicy::new()
        .max_inflight(1)
        .retry_after_policy(RetryAfterPolicy::FixedMs(500)));
    assert_eq!(saturate_then_reject(&g).retry_after_ms(), Some(500));
}

#[test]
fn no_retry_after_is_none() {
    let g = gov(ClassPolicy::new().max_inflight(1));
    assert_eq!(saturate_then_reject(&g).retry_after_ms(), None);
}

#[test]
fn adaptive_retry_after_is_deterministic() {
    // base(50) + queue_depth(0)*10 + inflight(1)*5 = 55.
    let g = gov(ClassPolicy::new()
        .max_inflight(1)
        .retry_after_policy(RetryAfterPolicy::Adaptive));
    let first = saturate_then_reject(&g).retry_after_ms();
    assert_eq!(first, Some(55));

    // Same observed state yields the same hint.
    let g2 = gov(ClassPolicy::new()
        .max_inflight(1)
        .retry_after_policy(RetryAfterPolicy::Adaptive));
    assert_eq!(saturate_then_reject(&g2).retry_after_ms(), first);
}

fn gov_fair(fairness: FairnessPolicy) -> Governor {
    gov(ClassPolicy::new()
        .max_inflight(1)
        .retry_after_policy(RetryAfterPolicy::Adaptive)
        .fairness(fairness))
}

#[test]
fn adaptive_retry_reflects_fairness_discipline() {
    // base at inflight=1, queue=0 is 55 (50 + 1*5). Each discipline adjusts it
    // deterministically from its own params.
    let weighted = gov_fair(FairnessPolicy::WeightedFairQueue {
        weight: 4,
        burst: 0,
    });
    assert_eq!(
        saturate_then_reject(&weighted).retry_after_ms(),
        Some(55 - 8)
    ); // weight relief

    let deadline = gov_fair(FairnessPolicy::DeadlineAware { slack_ms: 100 });
    assert_eq!(
        saturate_then_reject(&deadline).retry_after_ms(),
        Some(55 + 10)
    ); // slack/10

    let drr = gov_fair(FairnessPolicy::DeficitRoundRobin { quantum: 3 });
    assert_eq!(saturate_then_reject(&drr).retry_after_ms(), Some(55 + 3)); // quantum step

    // FIFO is unchanged (regression: keeps the original 55).
    let fifo = gov_fair(FairnessPolicy::Fifo);
    assert_eq!(saturate_then_reject(&fifo).retry_after_ms(), Some(55));
}
