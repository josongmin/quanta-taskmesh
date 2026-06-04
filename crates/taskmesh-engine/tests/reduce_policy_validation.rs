//! T06: a fan-out stage is not shippable without a complete reduce policy.

use taskmesh_contract::*;
use taskmesh_engine::Governor;

#[test]
fn fan_out_without_reduce_policy_rejects() {
    let spec = TaskSpec::cpu(TaskClass::new("c"))
        .fan_out_stage(TaskStage::new("merge"), SubstrateHint::SharedCpuExecutor);
    assert!(Governor::validate_reduce(&spec).is_err());
}

#[test]
fn fan_out_with_complete_reduce_policy_passes() {
    let spec = TaskSpec::cpu(TaskClass::new("c")).reduce_stage(
        TaskStage::new("merge"),
        SubstrateHint::SharedCpuExecutor,
        DeterministicReducePolicy::keyed("doc_id"),
    );
    assert!(Governor::validate_reduce(&spec).is_ok());
}

#[test]
fn fan_out_with_empty_sort_key_rejects() {
    let spec = TaskSpec::cpu(TaskClass::new("c")).reduce_stage(
        TaskStage::new("merge"),
        SubstrateHint::SharedCpuExecutor,
        DeterministicReducePolicy::keyed("   "),
    );
    assert!(Governor::validate_reduce(&spec).is_err());
}

#[test]
fn sequential_stages_need_no_reduce_policy() {
    let spec = TaskSpec::blocking(TaskClass::new("c"))
        .stage(TaskStage::new("rank"), SubstrateHint::SharedCpuExecutor);
    assert!(Governor::validate_reduce(&spec).is_ok());
}

// --- enforcement at the admission choke-point (not just the static helper) ---

use std::collections::BTreeMap;
use std::sync::Arc;
use taskmesh_engine::{AdmissionDecision, ManualClock, PolicySet, RequestKey};

fn gov() -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new().max_inflight(8).cpu_units(1),
    );
    Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(0)),
    )
}

#[test]
fn admit_rejects_fan_out_without_reduce_policy() {
    let g = gov();
    let bad = TaskSpec::cpu(TaskClass::new("c"))
        .fan_out_stage(TaskStage::new("merge"), SubstrateHint::SharedCpuExecutor)
        .operation("bad-reduce");
    assert!(matches!(
        g.admit(&bad, RequestKey::new("bad-reduce")),
        AdmissionDecision::Rejected(AdmissionVerdict::MalformedTask)
    ));
}

#[test]
fn admit_accepts_fan_out_with_complete_reduce_policy() {
    let g = gov();
    let good = TaskSpec::cpu(TaskClass::new("c"))
        .reduce_stage(
            TaskStage::new("merge"),
            SubstrateHint::SharedCpuExecutor,
            DeterministicReducePolicy::keyed("doc_id"),
        )
        .operation("good-reduce");
    assert!(matches!(
        g.admit(&good, RequestKey::new("good-reduce")),
        AdmissionDecision::Admitted { .. }
    ));
}
