//! Regression tests for the audit-driven hardening pass (engine side):
//! fail-closed `Governor::new`, spec-shape validation, empty-pool rejection,
//! and reconcile-downward promotion.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

fn classes(list: Vec<(&'static str, ClassPolicy)>) -> BTreeMap<TaskClass, ClassPolicy> {
    list.into_iter()
        .map(|(n, p)| (TaskClass::new(n), p))
        .collect()
}

// ---- D: Governor::new is fail-closed --------------------------------------

#[test]
fn governor_new_rejects_invalid_policy() {
    // class cpu cost exceeds the global budget -> construction must fail.
    let policy = PolicySet::new(
        ResourceBudget::new().cpu_units(4),
        classes(vec![("c", ClassPolicy::new().cpu_units(8))]),
    );
    assert!(Governor::new(policy, Arc::new(ManualClock::new(0))).is_err());
}

#[test]
fn governor_new_rejects_mixed_tier_fairness() {
    let policy = PolicySet::new(
        ResourceBudget::new(),
        classes(vec![
            ("a", ClassPolicy::new().fairness(FairnessPolicy::Fifo)),
            (
                "z",
                ClassPolicy::new().fairness(FairnessPolicy::DeadlineAware { slack_ms: 1 }),
            ),
        ]),
    );
    assert!(Governor::new(policy, Arc::new(ManualClock::new(0))).is_err());
}

#[test]
fn governor_new_accepts_valid_policy() {
    let policy = PolicySet::new(
        ResourceBudget::new().cpu_units(16),
        classes(vec![("c", ClassPolicy::new().cpu_units(1))]),
    );
    assert!(Governor::new(policy, Arc::new(ManualClock::new(0))).is_ok());
}

// ---- C: spec-shape validation (zero-stage, inconsistent per-stage class) ----

fn gov(list: Vec<(&'static str, ClassPolicy)>) -> Governor {
    Governor::new(
        PolicySet::new(
            ResourceBudget::new().cpu_units(100).memory_units(100),
            classes(list),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid policy")
}

#[test]
fn zero_stage_spec_is_rejected() {
    let g = gov(vec![("c", ClassPolicy::new().max_inflight(4).cpu_units(1))]);
    let mut spec = TaskSpec::blocking(TaskClass::new("c")).operation("op");
    spec.stages.clear(); // malformed wire payload: no stages
    assert!(matches!(
        g.admit(&spec),
        AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask)
    ));
}

#[test]
fn inconsistent_per_stage_class_is_rejected() {
    let g = gov(vec![("c", ClassPolicy::new().max_inflight(4).cpu_units(1))]);
    let mut spec = TaskSpec::blocking(TaskClass::new("c")).operation("op");
    // Tamper a stage's class to differ from the task class (fake authority).
    spec.stages[0].class = TaskClass::new("other");
    assert!(matches!(
        g.admit(&spec),
        AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask)
    ));
}

// ---- inventory: empty capability-pool name is rejected ---------------------

#[test]
fn empty_capability_pool_name_is_rejected() {
    let bad = SubstrateRecord::new("rogue", SubstrateKind::CompetingExecution, Some("  "));
    let policy = PolicySet::new(
        ResourceBudget::new(),
        classes(vec![("c", ClassPolicy::new())]),
    )
    .with_substrates(vec![bad]);
    assert!(
        policy.is_err(),
        "Some(\"\") pool must be rejected like None"
    );
}

// ---- B: reconcile downward promotes queued work ----------------------------

#[test]
fn downward_reconcile_memory_promotes_queued_work() {
    // Measured class, global memory budget 10, scale 1. Each permit reserves the
    // estimate (8), so one is inflight and a second must queue (8 + 8 > 10). A
    // downward `reconcile_memory` of the first to 2 bytes (= 2 units) frees budget
    // and MUST promote the queued second (the bug: reconcile never promoted).
    let g = Governor::new(
        PolicySet::new(
            ResourceBudget::new().memory_units(10).memory_unit_scale(1),
            classes(vec![(
                "m",
                ClassPolicy::new()
                    .max_inflight(8)
                    .max_queue_depth(8)
                    .memory_units(8)
                    .memory_permit_mode(MemoryPermitMode::Measured)
                    .memory_overcommit_policy(MemoryOvercommitPolicy::Queue)
                    .overflow_policy(OverflowPolicy::QueueWithinDepth),
            )]),
        ),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid");

    let spec = |op: &str| TaskSpec::blocking(TaskClass::new("m")).operation(op.to_string());
    let p1 = match g.admit(&spec("a")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        o => panic!("{o:?}"),
    };
    let ticket = match g.admit(&spec("b")) {
        AdmissionDecision::Queued { ticket } => ticket,
        o => panic!("expected queue (8+8>10), got {o:?}"),
    };
    assert_eq!(g.snapshot().classes[&TaskClass::new("m")].queued, 1);

    // Downward reconcile: measured 2 bytes -> 2 units; held 8 -> 2, frees 6.
    assert!(g.reconcile_memory(p1, 2));
    assert!(
        g.claim(ticket).is_some(),
        "downward reconcile must promote queued work"
    );
}

// ---- direct engine path has authoritative substrate inventory --------------

#[test]
fn direct_governor_new_exposes_builtin_inventory() {
    // A governor built straight from the engine (no host `Builder`) must still
    // expose the canonical built-in substrate inventory in its snapshot — the
    // inventory is intrinsic to the policy, not injected only by the host.
    let g = gov(vec![("c", ClassPolicy::new())]);
    let names: Vec<String> = g
        .snapshot()
        .substrates
        .iter()
        .map(|s| s.name.to_string())
        .collect();
    for builtin in BUILTIN_SUBSTRATES {
        assert!(
            names.contains(&(*builtin).to_string()),
            "direct Governor::new missing built-in substrate {builtin}"
        );
    }
}

// ---- parent_stage is authoritative for recursion identity ------------------

#[test]
fn same_parent_stage_different_substrate_is_recursive() {
    // Two children of the same root declaring the SAME parent_stage but running
    // on DIFFERENT substrates. Recursion identity is the declared lineage point
    // (root, parent_stage), not the substrate — so the second is a recursive
    // re-entry. (Before the fix this admitted: the guard keyed on the
    // substrate-derived bootstrap stage, so "blocking" != "cpu" dodged it.)
    let g = gov(vec![(
        "worker",
        ClassPolicy::new().max_inflight(8).cpu_units(1),
    )]);
    let c1 =
        TaskSpec::blocking(TaskClass::new("worker")).child_of("root", TaskStage::new("fanout"));
    let c2 = TaskSpec::cpu(TaskClass::new("worker")).child_of("root", TaskStage::new("fanout"));
    assert!(matches!(g.admit(&c1), AdmissionDecision::Admitted { .. }));
    assert!(
        matches!(
            g.admit(&c2),
            AdmissionDecision::Rejected(AdmissionVerdict::RecursiveAdmission)
        ),
        "same (root, parent_stage) is recursive regardless of substrate"
    );
}

#[test]
fn different_parent_stage_same_substrate_admits_both() {
    // Same substrate, DIFFERENT declared parent_stage -> distinct lineage points,
    // both admit. (Before the fix the second was rejected: both bootstrap stages
    // were "blocking", so the substrate-keyed guard saw a collision.)
    let g = gov(vec![(
        "worker",
        ClassPolicy::new().max_inflight(8).cpu_units(1),
    )]);
    let a =
        TaskSpec::blocking(TaskClass::new("worker")).child_of("root", TaskStage::new("stage-a"));
    let b =
        TaskSpec::blocking(TaskClass::new("worker")).child_of("root", TaskStage::new("stage-b"));
    assert!(matches!(g.admit(&a), AdmissionDecision::Admitted { .. }));
    assert!(
        matches!(g.admit(&b), AdmissionDecision::Admitted { .. }),
        "distinct parent_stages under one root are distinct lineage points"
    );
}
