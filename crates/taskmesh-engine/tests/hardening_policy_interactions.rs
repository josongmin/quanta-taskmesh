//! Combined policy oracles for H07 and D01.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, FairnessPolicy, ManualClock, MemoryOvercommitPolicy,
    OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, CapacityAssessment, ClaimOutcome, Governor, PermitId, PolicySet,
    ReleaseOutcome,
};

fn class(name: &str) -> TaskClass {
    TaskClass::new(name.to_owned())
}

fn io(class_name: &str, operation: &str) -> TaskSpec {
    TaskSpec::io(class(class_name)).operation(operation.to_owned())
}

fn blocking(class_name: &str, operation: &str) -> TaskSpec {
    TaskSpec::blocking(class(class_name)).operation(operation.to_owned())
}

fn admitted(decision: AdmissionDecision) -> PermitId {
    match decision {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("expected admission, got {other:?}"),
    }
}

#[test]
fn primary_scavenger_memory_fallback_and_drop_best_effort_keep_their_contracts() {
    let primary = ClassPolicy::new()
        .max_inflight(4)
        .max_queue_depth(4)
        .cpu_units(1)
        .overflow_policy(OverflowPolicy::QueueWithinDepth);
    let best_effort = ClassPolicy::new()
        .max_inflight(4)
        .max_queue_depth(4)
        .cpu_units(1)
        .fairness(FairnessPolicy::BestEffortScavenger)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .best_effort(true);
    let policy = PolicySet::new(
        ResourceBudget::new().cpu_units(1).memory_units(2),
        BTreeMap::from([
            (
                class("memory-holder"),
                ClassPolicy::new().max_inflight(1).memory_units(2),
            ),
            (
                class("cpu-holder"),
                ClassPolicy::new().max_inflight(1).cpu_units(1),
            ),
            (class("primary"), primary),
            (class("light"), best_effort.clone()),
            (class("scavenger"), best_effort.clone()),
            (
                class("drop"),
                best_effort
                    .overflow_policy(OverflowPolicy::DropBestEffort)
                    .max_queue_depth(0),
            ),
            (
                class("heavy"),
                ClassPolicy::new()
                    .max_inflight(4)
                    .memory_units(2)
                    .memory_overcommit_policy(MemoryOvercommitPolicy::DegradeToLight {
                        fallback_class: class("light"),
                    }),
            ),
        ]),
    );
    let governor = Governor::new(policy, Arc::new(ManualClock::new(0))).expect("valid policy");

    let memory_holder = admitted(governor.admit(&io("memory-holder", "memory-holder")));
    let cpu_holder = admitted(governor.admit(&io("cpu-holder", "cpu-holder")));

    let fallback = match governor.admit(&io("heavy", "fallback-first")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("memory pressure must reclassify then queue: {other:?}"),
    };
    let fallback_view = governor.pending_view(fallback).expect("fallback is queued");
    assert_eq!(fallback_view.class, class("light"));
    let CapacityAssessment::ReversiblyBlocked(fallback_blockers) = fallback_view.assessment else {
        panic!("fallback must expose its re-evaluated blocker")
    };
    assert!(fallback_blockers.cpu);
    assert!(!fallback_blockers.memory);

    let scavenger = match governor.admit(&io("scavenger", "scavenger-second")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("scavenger must queue: {other:?}"),
    };
    let before_drop = governor.snapshot();
    assert_eq!(
        governor.admit(&io("drop", "drop-is-explicit")),
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated {
            retry_after_ms: None,
        })
    );
    assert_eq!(governor.snapshot(), before_drop, "drop is state-neutral");

    let primary = match governor.admit(&io("primary", "primary-last")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("primary must queue: {other:?}"),
    };
    assert_eq!(governor.release(cpu_holder), ReleaseOutcome::Released);
    assert!(matches!(governor.claim(fallback), ClaimOutcome::Pending));
    assert!(matches!(governor.claim(scavenger), ClaimOutcome::Pending));
    let ClaimOutcome::Ready(primary_permit) = governor.claim(primary) else {
        panic!("runnable primary must precede older best-effort work")
    };

    assert_eq!(governor.release(primary_permit), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(fallback_permit) = governor.claim(fallback) else {
        panic!("the older fallback must lead the best-effort tier")
    };
    assert_eq!(
        governor
            .permit_ledger(fallback_permit)
            .expect("live fallback")
            .class,
        class("light")
    );
    assert_eq!(governor.release(fallback_permit), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(scavenger_permit) = governor.claim(scavenger) else {
        panic!("the remaining scavenger must make progress")
    };
    assert_eq!(governor.release(scavenger_permit), ReleaseOutcome::Released);
    assert_eq!(governor.release(memory_holder), ReleaseOutcome::Released);
    assert_eq!(governor.snapshot().conservation_violation(), None);
}

#[test]
fn zero_one_exact_and_plus_one_are_distinct_for_every_capacity_kind() {
    let disabled = Governor::new(
        PolicySet::new(
            ResourceBudget::new(),
            BTreeMap::from([(class("disabled"), ClassPolicy::new().max_inflight(0))]),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("zero quota is a valid disabled class");
    assert_eq!(
        disabled.admit(&io("disabled", "disabled")),
        AdmissionDecision::Rejected(AdmissionVerdict::ClassDisabled)
    );

    let zero_depth = Governor::new(
        PolicySet::new(
            ResourceBudget::new(),
            BTreeMap::from([(
                class("zero-depth"),
                ClassPolicy::new()
                    .max_inflight(1)
                    .max_queue_depth(0)
                    .overflow_policy(OverflowPolicy::QueueWithinDepth),
            )]),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect_err("queueing with zero depth is contradictory");
    assert_eq!(
        zero_depth,
        taskmesh_contract::GovernorError::PolicyViolation(
            "class zero-depth is queueable but has max_queue_depth == 0".into()
        )
    );

    let queue = Governor::new(
        PolicySet::new(
            ResourceBudget::new(),
            BTreeMap::from([(
                class("queue"),
                ClassPolicy::new()
                    .max_inflight(1)
                    .max_queue_depth(1)
                    .overflow_policy(OverflowPolicy::QueueWithinDepth),
            )]),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("one slot and one queue position are valid");
    let holder = admitted(queue.admit(&io("queue", "holder")));
    let queued = match queue.admit(&io("queue", "depth-exact")) {
        AdmissionDecision::Queued { ticket } => ticket,
        other => panic!("the one queue position must be usable: {other:?}"),
    };
    assert_eq!(
        queue.admit(&io("queue", "depth-plus-one")),
        AdmissionDecision::Rejected(AdmissionVerdict::QueueFull {
            retry_after_ms: None,
        })
    );
    assert_eq!(queue.release(holder), ReleaseOutcome::Released);
    let ClaimOutcome::Ready(promoted) = queue.claim(queued) else {
        panic!("the exact-depth waiter must promote")
    };
    assert_eq!(queue.release(promoted), ReleaseOutcome::Released);

    let unbounded_pool_policy = PolicySet::new(
        ResourceBudget::new(),
        BTreeMap::from([(class("pool"), ClassPolicy::new().max_inflight(3))]),
    )
    .with_capability_limits(BTreeMap::from([("blocking".to_owned(), 0)]))
    .expect("zero pool capacity means explicit unbounded");
    let unbounded_pool = Governor::new(unbounded_pool_policy, Arc::new(ManualClock::new(0)))
        .expect("valid unbounded pool");
    let pool_a = admitted(unbounded_pool.admit(&blocking("pool", "pool-a")));
    let pool_b = admitted(unbounded_pool.admit(&blocking("pool", "pool-b")));
    assert_eq!(unbounded_pool.snapshot().capabilities["blocking"].limit, 0);
    assert_eq!(unbounded_pool.release(pool_a), ReleaseOutcome::Released);
    assert_eq!(unbounded_pool.release(pool_b), ReleaseOutcome::Released);

    let bounded_pool_policy = PolicySet::new(
        ResourceBudget::new(),
        BTreeMap::from([(class("pool"), ClassPolicy::new().max_inflight(3))]),
    )
    .with_capability_limits(BTreeMap::from([("blocking".to_owned(), 1)]))
    .expect("one pool slot is valid");
    let bounded_pool = Governor::new(bounded_pool_policy, Arc::new(ManualClock::new(0)))
        .expect("valid bounded pool");
    let pool_holder = admitted(bounded_pool.admit(&blocking("pool", "pool-exact")));
    assert_eq!(
        bounded_pool.admit(&blocking("pool", "pool-plus-one")),
        AdmissionDecision::Rejected(AdmissionVerdict::SubstrateSaturated {
            retry_after_ms: None,
        })
    );
    assert_eq!(bounded_pool.release(pool_holder), ReleaseOutcome::Released);

    let unbounded_budget = Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(0).memory_units(0),
            BTreeMap::from([(
                class("budget"),
                ClassPolicy::new()
                    .max_inflight(2)
                    .cpu_units(u32::MAX)
                    .memory_units(u32::MAX),
            )]),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("zero global budgets mean explicit unbounded");
    let budget_a = admitted(unbounded_budget.admit(&io("budget", "budget-a")));
    let budget_b = admitted(unbounded_budget.admit(&io("budget", "budget-b")));
    assert_eq!(unbounded_budget.release(budget_a), ReleaseOutcome::Released);
    assert_eq!(unbounded_budget.release(budget_b), ReleaseOutcome::Released);

    let exact_budget = Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(1).memory_units(1),
            BTreeMap::from([(
                class("budget"),
                ClassPolicy::new()
                    .max_inflight(2)
                    .cpu_units(1)
                    .memory_units(1),
            )]),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("one-unit exact budget is valid");
    let budget_holder = admitted(exact_budget.admit(&io("budget", "budget-exact")));
    assert_eq!(
        exact_budget.admit(&io("budget", "budget-plus-one")),
        AdmissionDecision::Rejected(AdmissionVerdict::CpuSaturated {
            retry_after_ms: None,
        })
    );
    assert_eq!(
        exact_budget.release(budget_holder),
        ReleaseOutcome::Released
    );

    let oversized = Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(1).memory_units(1),
            BTreeMap::from([(
                class("oversized"),
                ClassPolicy::new()
                    .max_inflight(1)
                    .cpu_units(2)
                    .memory_units(2),
            )]),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect_err("a per-request cost above a finite budget is invalid");
    assert_eq!(
        oversized,
        taskmesh_contract::GovernorError::PolicyViolation(
            "class oversized exceeds global cpu budget".into()
        )
    );
}
