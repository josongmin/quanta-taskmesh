//! BG25-008: compound capacity decisions against an input-derived ledger.
//!
//! Expected costs are fixed by this fixture's submitted classes and events;
//! they are never read back from `permit_ledgers()` or a production snapshot.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ManualClock, MemoryOvercommitPolicy, OverflowPolicy,
    ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PermitId, PolicySet, ReleaseOutcome};

fn class(name: &str) -> TaskClass {
    TaskClass::new(name.to_owned())
}

fn policy(max_inflight: u32, cpu: u32, memory: u32) -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(max_inflight)
        .cpu_units(cpu)
        .memory_units(memory)
        .overflow_policy(OverflowPolicy::Reject)
        .memory_overcommit_policy(MemoryOvercommitPolicy::Reject)
}

fn governor(
    cpu_budget: u32,
    memory_budget: u32,
    blocking_slots: u32,
    classes: &[(&str, u32, u32, u32)],
) -> Governor {
    let classes = classes
        .iter()
        .map(|&(name, max_inflight, cpu, memory)| (class(name), policy(max_inflight, cpu, memory)))
        .collect();
    let policy = PolicySet::new(
        ResourceBudget::new()
            .cpu_units(cpu_budget)
            .memory_units(memory_budget),
        classes,
    )
    .with_capability_limits(BTreeMap::from([("blocking".to_owned(), blocking_slots)]))
    .expect("built-in blocking pool exists");
    Governor::new(policy, Arc::new(ManualClock::new(0))).expect("valid policy")
}

fn submit(g: &Governor, name: &str, operation: &str) -> AdmissionDecision {
    g.admit(&TaskSpec::blocking(class(name)).operation(operation))
}

fn admitted(g: &Governor, name: &str, operation: &str) -> PermitId {
    match submit(g, name, operation) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("{name}/{operation} should admit, got {other:?}"),
    }
}

#[derive(Clone, Copy)]
struct HeldInput {
    class: &'static str,
    cpu: u32,
    memory: u32,
}

fn assert_input_ledger(g: &Governor, held: &BTreeMap<PermitId, HeldInput>) {
    let snapshot = g.snapshot();
    for name in ["a", "b", "c"] {
        let expected: Vec<_> = held.values().filter(|item| item.class == name).collect();
        let observed = &snapshot.classes[&class(name)];
        assert_eq!(
            observed.inflight as usize,
            expected.len(),
            "{name} inflight"
        );
        assert_eq!(
            observed.cpu_units_held,
            expected.iter().map(|item| u128::from(item.cpu)).sum(),
            "{name} CPU"
        );
        assert_eq!(
            observed.memory_units_held,
            expected.iter().map(|item| u128::from(item.memory)).sum(),
            "{name} memory"
        );
        assert_eq!(observed.queued, 0, "{name} queue");
    }
    assert_eq!(
        snapshot.capabilities["blocking"].in_use as usize,
        held.len(),
        "blocking occupancy follows admitted inputs only"
    );
    assert_eq!(snapshot.conservation_violation(), None);
}

#[test]
fn cross_class_global_and_pool_limits_follow_only_admitted_events() {
    let g = governor(4, 6, 2, &[("a", 2, 2, 3), ("b", 2, 2, 3), ("c", 2, 1, 1)]);
    let mut held = BTreeMap::new();
    let a = admitted(&g, "a", "a1");
    held.insert(
        a,
        HeldInput {
            class: "a",
            cpu: 2,
            memory: 3,
        },
    );
    assert_input_ledger(&g, &held);
    let b = admitted(&g, "b", "b1");
    held.insert(
        b,
        HeldInput {
            class: "b",
            cpu: 2,
            memory: 3,
        },
    );
    assert_input_ledger(&g, &held);

    // c has free class quota, but the shared pool, CPU, and memory are all
    // full. Capability is the accepted primary blocker; rejection changes no
    // input ledger or occupancy.
    assert!(matches!(
        submit(&g, "c", "blocked"),
        AdmissionDecision::Rejected(AdmissionVerdict::SubstrateSaturated { .. })
    ));
    assert_input_ledger(&g, &held);

    assert_eq!(g.release(a), ReleaseOutcome::Released);
    held.remove(&a);
    assert_input_ledger(&g, &held);
    let c = admitted(&g, "c", "after-release");
    held.insert(
        c,
        HeldInput {
            class: "c",
            cpu: 1,
            memory: 1,
        },
    );
    assert_input_ledger(&g, &held);
    assert_eq!(g.release(b), ReleaseOutcome::Released);
    held.remove(&b);
    assert_eq!(g.release(c), ReleaseOutcome::Released);
    held.remove(&c);
    assert_input_ledger(&g, &held);
}

#[test]
fn selected_compound_blockers_follow_class_pool_cpu_memory_priority() {
    struct Case {
        name: &'static str,
        contender_class: &'static str,
        holder_class_limit: u32,
        cpu_budget: u32,
        memory_budget: u32,
        pool_slots: u32,
        expected: fn(&AdmissionDecision) -> bool,
    }
    let cases = [
        Case {
            name: "class before pool, CPU, memory",
            contender_class: "a",
            holder_class_limit: 1,
            cpu_budget: 1,
            memory_budget: 1,
            pool_slots: 1,
            expected: |decision| {
                matches!(
                    decision,
                    AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
                )
            },
        },
        Case {
            name: "pool before CPU and memory",
            contender_class: "b",
            holder_class_limit: 2,
            cpu_budget: 1,
            memory_budget: 1,
            pool_slots: 1,
            expected: |decision| {
                matches!(
                    decision,
                    AdmissionDecision::Rejected(AdmissionVerdict::SubstrateSaturated { .. })
                )
            },
        },
        Case {
            name: "CPU before memory",
            contender_class: "b",
            holder_class_limit: 2,
            cpu_budget: 1,
            memory_budget: 1,
            pool_slots: 2,
            expected: |decision| {
                matches!(
                    decision,
                    AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
                )
            },
        },
        Case {
            name: "memory only",
            contender_class: "b",
            holder_class_limit: 2,
            cpu_budget: 2,
            memory_budget: 1,
            pool_slots: 2,
            expected: |decision| {
                matches!(
                    decision,
                    AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated { .. })
                )
            },
        },
    ];
    for case in cases {
        let g = governor(
            case.cpu_budget,
            case.memory_budget,
            case.pool_slots,
            &[
                ("a", case.holder_class_limit, 1, 1),
                ("b", 2, 1, 1),
                ("c", 2, 1, 1),
            ],
        );
        let holder = admitted(&g, "a", "holder");
        let before = g.snapshot();
        let decision = submit(&g, case.contender_class, "contender");
        assert!(
            (case.expected)(&decision),
            "{}: got {decision:?}",
            case.name
        );
        assert_eq!(g.snapshot(), before, "{} changed capacity", case.name);
        assert_eq!(g.release(holder), ReleaseOutcome::Released);
    }
}

#[test]
fn each_capacity_dimension_saturates_without_help_from_another_dimension() {
    struct Case {
        name: &'static str,
        governor: Governor,
        contender_class: &'static str,
        expected: fn(&AdmissionDecision) -> bool,
    }

    let cases = [
        Case {
            name: "class quota",
            governor: governor(10, 10, 10, &[("a", 1, 1, 1), ("b", 4, 1, 1)]),
            contender_class: "a",
            expected: |decision| {
                matches!(
                    decision,
                    AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
                )
            },
        },
        Case {
            name: "capability pool",
            governor: governor(10, 10, 1, &[("a", 4, 1, 1), ("b", 4, 1, 1)]),
            contender_class: "b",
            expected: |decision| {
                matches!(
                    decision,
                    AdmissionDecision::Rejected(AdmissionVerdict::SubstrateSaturated { .. })
                )
            },
        },
        Case {
            name: "CPU budget",
            governor: governor(1, 10, 10, &[("a", 4, 1, 1), ("b", 4, 1, 1)]),
            contender_class: "b",
            expected: |decision| {
                matches!(
                    decision,
                    AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
                )
            },
        },
        Case {
            name: "memory budget",
            governor: governor(10, 1, 10, &[("a", 4, 1, 1), ("b", 4, 1, 1)]),
            contender_class: "b",
            expected: |decision| {
                matches!(
                    decision,
                    AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated { .. })
                )
            },
        },
    ];

    for case in cases {
        let holder = admitted(&case.governor, "a", "holder");
        let before = case.governor.snapshot();
        let decision = submit(&case.governor, case.contender_class, "contender");
        assert!(
            (case.expected)(&decision),
            "{}: got {decision:?}",
            case.name
        );
        assert_eq!(
            case.governor.snapshot(),
            before,
            "{} changed state",
            case.name
        );
        assert_eq!(
            case.governor.release(holder),
            ReleaseOutcome::Released,
            "{} holder release",
            case.name
        );
        assert_eq!(case.governor.snapshot().conservation_violation(), None);
    }
}

#[test]
fn class_full_precedes_memory_then_memory_becomes_primary_after_class_release() {
    let g = governor(3, 1, 3, &[("a", 1, 1, 1), ("b", 1, 1, 1), ("c", 1, 1, 1)]);
    let a = admitted(&g, "a", "a-holder");
    let mut held = BTreeMap::from([(
        a,
        HeldInput {
            class: "a",
            cpu: 1,
            memory: 1,
        },
    )]);
    assert_input_ledger(&g, &held);
    assert!(matches!(
        submit(&g, "a", "class-full"),
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated { .. })
    ));
    assert_input_ledger(&g, &held);

    assert_eq!(g.release(a), ReleaseOutcome::Released);
    held.remove(&a);
    let b = admitted(&g, "b", "memory-holder");
    held.insert(
        b,
        HeldInput {
            class: "b",
            cpu: 1,
            memory: 1,
        },
    );
    assert_input_ledger(&g, &held);
    assert!(matches!(
        submit(&g, "a", "memory-full"),
        AdmissionDecision::Rejected(AdmissionVerdict::MemorySaturated { .. })
    ));
    assert_input_ledger(&g, &held);
    assert_eq!(g.release(b), ReleaseOutcome::Released);
    held.remove(&b);
    assert_input_ledger(&g, &held);
}
