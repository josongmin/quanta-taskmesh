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
