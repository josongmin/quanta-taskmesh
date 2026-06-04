//! T02: fail-closed configuration validation.

use std::collections::BTreeMap;

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

#[test]
fn valid_minimal_config_passes() {
    let p = policy(
        ResourceBudget::new().cpu_units(16).memory_units(32),
        vec![("retrieval", ClassPolicy::new().cpu_units(1).memory_units(2))],
    );
    assert!(Governor::validate_policy(&p).is_ok());
}

#[test]
fn class_cpu_over_global_budget_fails() {
    let p = policy(
        ResourceBudget::new().cpu_units(4),
        vec![("c", ClassPolicy::new().cpu_units(8))],
    );
    assert!(Governor::validate_policy(&p).is_err());
}

#[test]
fn class_memory_over_global_budget_fails() {
    let p = policy(
        ResourceBudget::new().memory_units(4),
        vec![("c", ClassPolicy::new().memory_units(8))],
    );
    assert!(Governor::validate_policy(&p).is_err());
}

#[test]
fn per_request_limit_overflow_fails() {
    let p = policy(
        ResourceBudget::new()
            .cpu_units(100)
            .per_request_cpu_units(2),
        vec![("c", ClassPolicy::new().cpu_units(4))],
    );
    assert!(Governor::validate_policy(&p).is_err());
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
    assert!(Governor::validate_policy(&p).is_err());
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
    assert!(Governor::validate_policy(&p).is_err());
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
    assert!(Governor::validate_policy(&p).is_err());
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
    assert!(Governor::validate_policy(&p).is_ok());
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
    assert!(Governor::validate_policy(&p).is_err());

    let ok = policy(
        ResourceBudget::new(),
        vec![(
            "c",
            ClassPolicy::new()
                .fairness(FairnessPolicy::BestEffortScavenger)
                .best_effort(true),
        )],
    );
    assert!(Governor::validate_policy(&ok).is_ok());
}
