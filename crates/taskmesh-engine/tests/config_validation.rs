//! T02: fail-closed configuration validation.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn classes(list: Vec<(&'static str, ClassPolicy)>) -> BTreeMap<TaskClass, ClassPolicy> {
    list.into_iter()
        .map(|(n, p)| (TaskClass::new(n), p))
        .collect()
}

fn policy(resources: ResourceBudget, list: Vec<(&'static str, ClassPolicy)>) -> PolicySet {
    PolicySet::new(resources, classes(list))
}

fn assert_policy_violation(policy: &PolicySet, expected: &str) {
    let error = Governor::validate_policy(policy).expect_err(expected);
    assert_eq!(
        error,
        GovernorError::PolicyViolation(expected.to_owned().into())
    );
}

#[test]
fn valid_minimal_config_passes() {
    let p = policy(
        ResourceBudget::new().cpu_units(16).memory_units(32),
        vec![("retrieval", ClassPolicy::new().cpu_units(1).memory_units(2))],
    );
    assert_eq!(Governor::validate_policy(&p), Ok(()));
}

#[test]
fn malformed_policy_class_is_rejected_before_governor_state_exists() {
    for invalid in [
        String::new(),
        " leading".to_owned(),
        "trailing ".to_owned(),
        "bad?class".to_owned(),
        "a".repeat(129),
    ] {
        let p = PolicySet::new(
            ResourceBudget::new(),
            BTreeMap::from([(TaskClass::new(invalid.clone()), ClassPolicy::new())]),
        );
        let Err(GovernorError::PolicyViolation(message)) =
            Governor::new(p, Arc::new(ManualClock::new(0)))
        else {
            panic!("malformed class {invalid:?} must fail at construction");
        };
        assert!(
            message.contains("invalid policy class") && message.contains("class"),
            "malformed class {invalid:?} returned unrelated violation: {message}"
        );
    }
}

#[test]
fn class_cpu_over_global_budget_fails() {
    let p = policy(
        ResourceBudget::new().cpu_units(4),
        vec![("c", ClassPolicy::new().cpu_units(8))],
    );
    assert_policy_violation(&p, "class c exceeds global cpu budget");
}

#[test]
fn class_memory_over_global_budget_fails() {
    let p = policy(
        ResourceBudget::new().memory_units(4),
        vec![("c", ClassPolicy::new().memory_units(8))],
    );
    assert_policy_violation(&p, "class c exceeds global memory budget");
}

#[test]
fn per_request_limit_overflow_fails() {
    let p = policy(
        ResourceBudget::new()
            .cpu_units(100)
            .per_request_cpu_units(2),
        vec![("c", ClassPolicy::new().cpu_units(4))],
    );
    assert_policy_violation(&p, "class c exceeds per-request cpu limit");
}

#[test]
fn per_request_memory_limit_overflow_fails() {
    // The per-request memory cap is a separate check from the cpu one: a class
    // whose memory cost exceeds it is impossible to admit under that budget,
    // so it is refused at construction rather than shed on every admit.
    let p = policy(
        ResourceBudget::new()
            .memory_units(100)
            .per_request_memory_units(2),
        vec![("c", ClassPolicy::new().memory_units(4))],
    );
    let error = Governor::validate_policy(&p)
        .expect_err("a class costing more memory than the per-request cap must be rejected");
    assert!(
        matches!(&error, GovernorError::PolicyViolation(message) if message.contains("per-request memory limit")),
        "the rejection names the per-request memory limit, got {error:?}"
    );
}

#[test]
fn per_request_limits_are_inclusive_at_the_boundary() {
    // A cost *equal* to the cap is within it, for cpu and memory alike. The
    // cap bounds a request, it does not require strictly smaller requests.
    let p = policy(
        ResourceBudget::new()
            .cpu_units(100)
            .memory_units(100)
            .per_request_cpu_units(4)
            .per_request_memory_units(2),
        vec![("c", ClassPolicy::new().cpu_units(4).memory_units(2))],
    );
    assert_eq!(
        Governor::validate_policy(&p),
        Ok(()),
        "a cost equal to the per-request cap is within the cap"
    );
}

#[test]
fn measured_mode_requires_bytes_per_unit() {
    // bytes_per_unit defaults to 1, so explicitly zero it out.
    let mut resources = ResourceBudget::new().memory_units(32);
    resources.memory_unit_scale = MemoryUnitScale { bytes_per_unit: 0 };
    let p = policy(
        resources,
        vec![(
            "c",
            ClassPolicy::new()
                .memory_units(1)
                .memory_permit_mode(MemoryPermitMode::Measured),
        )],
    );
    assert_policy_violation(
        &p,
        "class c: measured/hybrid memory requires bytes_per_unit > 0",
    );
}

#[test]
fn queueable_class_requires_queue_depth() {
    let p = policy(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new().overflow_policy(OverflowPolicy::QueueWithinDepth),
        )],
    );
    assert_policy_violation(&p, "class c is queueable but has max_queue_depth == 0");
}

#[test]
fn degrade_to_unknown_fallback_fails() {
    let p = policy(
        ResourceBudget::new().memory_units(8),
        vec![(
            "c",
            ClassPolicy::new()
                .max_queue_depth(4)
                .memory_overcommit_policy(MemoryOvercommitPolicy::DegradeToLight {
                    fallback_class: TaskClass::new("missing"),
                }),
        )],
    );
    assert_policy_violation(&p, "class c degrades to unknown fallback missing");
}

#[test]
fn degrade_to_present_fallback_passes() {
    let p = policy(
        ResourceBudget::new().memory_units(8),
        vec![
            (
                "heavy",
                ClassPolicy::new().memory_units(2).memory_overcommit_policy(
                    MemoryOvercommitPolicy::DegradeToLight {
                        fallback_class: TaskClass::new("light"),
                    },
                ),
            ),
            ("light", ClassPolicy::new().memory_units(1)),
        ],
    );
    assert_eq!(Governor::validate_policy(&p), Ok(()));
}

#[test]
fn degrade_to_a_disabled_fallback_is_rejected() {
    // A degrade promises *some* service under memory pressure. A fallback with
    // `max_inflight = 0` can never dispatch, so every degraded request would be
    // shed as `ClassDisabled` — a verdict that names the wrong class and hides
    // the real cause (memory). Contradictory configuration is refused at boot.
    let p = policy(
        ResourceBudget::new().memory_units(8),
        vec![
            (
                "heavy",
                ClassPolicy::new().memory_units(2).memory_overcommit_policy(
                    MemoryOvercommitPolicy::DegradeToLight {
                        fallback_class: TaskClass::new("light"),
                    },
                ),
            ),
            ("light", ClassPolicy::new().max_inflight(0).memory_units(1)),
        ],
    );
    let error = Governor::validate_policy(&p).expect_err("disabled fallback must be rejected");
    assert!(
        matches!(&error, GovernorError::PolicyViolation(message) if message.contains("disabled fallback light")),
        "got {error:?}"
    );
}

#[test]
fn a_degrade_chain_is_rejected() {
    // A degrade is one hop (D03): a request that already degraded is admitted
    // against the fallback's account without consulting the fallback's own
    // overcommit policy. A fallback that itself degrades therefore declares a
    // second hop the engine never takes — silently, on a chain (a → b → c), or
    // back to the class that just failed, on a cycle (a → b → a). Both are
    // refused at construction, not discovered as an unexplained shed.
    fn degrade_to(fallback: &'static str) -> ClassPolicy {
        ClassPolicy::new().memory_units(1).memory_overcommit_policy(
            MemoryOvercommitPolicy::DegradeToLight {
                fallback_class: TaskClass::new(fallback),
            },
        )
    }
    let chain = policy(
        ResourceBudget::new().memory_units(8),
        vec![
            ("a", degrade_to("b")),
            ("b", degrade_to("c")),
            ("c", ClassPolicy::new().memory_units(1)),
        ],
    );
    let cycle = policy(
        ResourceBudget::new().memory_units(8),
        vec![("a", degrade_to("b")), ("b", degrade_to("a"))],
    );
    for (label, p) in [("a → b → c", chain), ("a → b → a", cycle)] {
        let error = Governor::validate_policy(&p)
            .expect_err("a fallback that itself degrades must be rejected");
        assert!(
            matches!(&error, GovernorError::PolicyViolation(message) if message.contains("which itself degrades")),
            "{label}: the rejection names the chained degrade, got {error:?}"
        );
    }
}

#[test]
fn a_capability_limit_on_an_authority_only_pool_is_rejected() {
    // A limit gates the executing substrate that provides the pool. An
    // `AuthorityOnly` substrate executes nothing, so a limit on the pool it
    // names is consulted by no dispatch path: the registry accepts the record
    // (it may name a pool), the limit lookup finds a provider, and only the
    // whole-inventory validation at construction sees that the provider can
    // never occupy a slot.
    let error = PolicySet::new(
        ResourceBudget::new(),
        classes(vec![("c", ClassPolicy::new())]),
    )
    .with_substrates(vec![SubstrateRecord::new(
        "gpu",
        SubstrateKind::AuthorityOnly,
        Some("gpu"),
    )])
    .expect("an authority-only record may name a pool")
    .with_capability_limits(BTreeMap::from([("gpu".to_string(), 2)]))
    .expect_err("authority-only substrates cannot own execution capacity");
    assert!(
        matches!(&error, GovernorError::PolicyViolation(message) if message.contains("unregistered pool")),
        "the rejection says the pool has no executing provider, got {error:?}"
    );
}

#[test]
fn scavenger_fairness_requires_best_effort() {
    let p = policy(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new().fairness(FairnessPolicy::BestEffortScavenger), // best_effort defaults false
        )],
    );
    assert_policy_violation(
        &p,
        "class c uses BestEffortScavenger fairness but is not best_effort",
    );

    let ok = policy(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new()
                .fairness(FairnessPolicy::BestEffortScavenger)
                .best_effort(true),
        )],
    );
    assert_eq!(Governor::validate_policy(&ok), Ok(()));
}

#[test]
fn mixed_fairness_disciplines_in_a_tier_reject() {
    // Fifo + DeadlineAware in the same (non-best-effort) tier is rejected, so the
    // scheduler never silently imposes the lexically-first class's discipline.
    let p = policy(
        ResourceBudget::new(),
        vec![
            ("a", ClassPolicy::new().fairness(FairnessPolicy::Fifo)),
            (
                "z",
                ClassPolicy::new().fairness(FairnessPolicy::DeadlineAware { slack_ms: 1 }),
            ),
        ],
    );
    assert_policy_violation(
        &p,
        "class z mixes a different fairness discipline within its scheduling tier; all classes in a tier must share one discipline",
    );
}

#[test]
fn same_discipline_different_weights_is_ok() {
    // WFQ with different weights is fine — same discipline kind, different params.
    let p = policy(
        ResourceBudget::new(),
        vec![
            (
                "a",
                ClassPolicy::new().fairness(FairnessPolicy::WeightedFairQueue {
                    weight: 4,
                    burst: 0,
                }),
            ),
            (
                "b",
                ClassPolicy::new().fairness(FairnessPolicy::WeightedFairQueue {
                    weight: 1,
                    burst: 0,
                }),
            ),
        ],
    );
    assert_eq!(Governor::validate_policy(&p), Ok(()));
}

#[test]
fn different_discipline_across_tiers_is_ok() {
    // A best-effort class forms its own tier, so it may differ from the primary.
    let p = policy(
        ResourceBudget::new(),
        vec![
            (
                "interactive",
                ClassPolicy::new().fairness(FairnessPolicy::DeadlineAware { slack_ms: 5 }),
            ),
            (
                "batch",
                ClassPolicy::new()
                    .fairness(FairnessPolicy::BestEffortScavenger)
                    .best_effort(true),
            ),
        ],
    );
    assert_eq!(Governor::validate_policy(&p), Ok(()));
}

/// The three `validate_policy` rejections the coverage report showed no test
/// reaching. Each one asserts the *named* violation: `is_err()` alone would be
/// satisfied by any earlier rule tripping on the same fixture.
fn violation_message(p: &PolicySet, rule: &str) -> String {
    let Err(GovernorError::PolicyViolation(message)) = Governor::validate_policy(p) else {
        panic!("{rule} must be refused, but validation accepted the policy");
    };
    message.to_string()
}

#[test]
fn a_weighted_fair_queue_weight_of_zero_is_rejected_not_coerced() {
    // Weight 0 has no proportional meaning. Coercing it to 1 (the old `max(1)`
    // floor) made an obviously wrong config look deliberate, and gave every
    // "zero" class the same share as weight 1.
    let p = policy(
        ResourceBudget::new(),
        vec![(
            "zero-share",
            ClassPolicy::new().fairness(FairnessPolicy::WeightedFairQueue {
                weight: 0,
                burst: 0,
            }),
        )],
    );
    let message = violation_message(&p, "a zero WFQ weight");
    assert!(
        message.contains("weight 0") && message.contains("zero-share"),
        "a zero WFQ weight must be refused by name, got: {message}"
    );
    // Weight 1 on the same fixture is fine — the rule is exactly `== 0`.
    let ok = policy(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new().fairness(FairnessPolicy::WeightedFairQueue {
                weight: 1,
                burst: 0,
            }),
        )],
    );
    assert_eq!(
        Governor::validate_policy(&ok),
        Ok(()),
        "weight 1 is the smallest valid weight"
    );
}

#[test]
fn queueing_on_memory_overcommit_requires_a_queue_to_wait_in() {
    // `MemoryOvercommitPolicy::Queue` promises "wait for memory". With
    // `max_queue_depth == 0` there is nowhere to wait: every memory-blocked
    // arrival would be shed as QueueFull while the operator believes it queues.
    let p = policy(
        ResourceBudget::new().memory_units(8),
        vec![(
            "m",
            ClassPolicy::new()
                .memory_units(4)
                .memory_overcommit_policy(MemoryOvercommitPolicy::Queue),
        )],
    );
    let message = violation_message(&p, "memory Queue without a queue");
    assert!(
        message.contains("queues on overcommit") && message.contains("max_queue_depth == 0"),
        "memory Queue without a queue must be refused by name, got: {message}"
    );
    let ok = policy(
        ResourceBudget::new().memory_units(8),
        vec![(
            "m",
            ClassPolicy::new()
                .memory_units(4)
                .max_queue_depth(1)
                .memory_overcommit_policy(MemoryOvercommitPolicy::Queue),
        )],
    );
    assert_eq!(
        Governor::validate_policy(&ok),
        Ok(()),
        "depth 1 is enough to queue"
    );
}

#[test]
fn a_class_that_degrades_to_itself_is_rejected() {
    // A self-fallback is a one-hop degrade that lands where it started: the
    // engine would re-evaluate the same policy and either loop or admit the
    // request it just found too heavy. Distinct from the chain rule (a→b→c),
    // which needs a *second* class to trip.
    let p = policy(
        ResourceBudget::new().memory_units(8),
        vec![(
            "a",
            ClassPolicy::new().memory_units(4).memory_overcommit_policy(
                MemoryOvercommitPolicy::DegradeToLight {
                    fallback_class: TaskClass::new("a"),
                },
            ),
        )],
    );
    let message = violation_message(&p, "a self-fallback");
    assert!(
        message.contains("degrades to itself"),
        "a self-fallback must be refused by name, got: {message}"
    );
}
