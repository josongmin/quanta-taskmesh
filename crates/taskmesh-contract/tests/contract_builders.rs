//! T01: builder smoke and identifier semantics.

use std::collections::BTreeSet;

use taskmesh_contract::*;

#[test]
fn task_spec_constructors_set_bootstrap_stage() {
    assert_eq!(
        TaskSpec::io(TaskClass::new("c")).primary_substrate_hint(),
        SubstrateHint::AsyncIo
    );
    assert_eq!(
        TaskSpec::blocking(TaskClass::new("c")).primary_substrate_hint(),
        SubstrateHint::BlockingPool
    );
    assert_eq!(
        TaskSpec::cpu(TaskClass::new("c")).primary_substrate_hint(),
        SubstrateHint::SharedCpuExecutor
    );
    assert_eq!(
        TaskSpec::local(TaskClass::new("c")).primary_substrate_hint(),
        SubstrateHint::LocalRuntime
    );
}

#[test]
fn operation_sets_root_operation_id_for_root() {
    let spec = TaskSpec::io(TaskClass::new("c")).operation("op-1");
    assert_eq!(spec.operation, "op-1");
    assert_eq!(spec.root_operation_id, "op-1");
    assert!(matches!(spec.scope, TaskScope::Root));
}

#[test]
fn child_of_inherits_root_id_and_sets_scope() {
    let child = TaskSpec::cpu(TaskClass::new("c")).child_of("root-7", TaskStage::new("fanout"));
    assert_eq!(child.root_operation_id, "root-7");
    assert!(matches!(child.scope, TaskScope::Child { .. }));
}

#[test]
fn builder_defaults_smoke() {
    let _ = ClassPolicy::new();
    let _ = ResourceBudget::new();
    let _ = TopologyConfig::new();
    assert!(ClassPolicy::new().max_inflight(0).is_disabled());
    assert!(!ClassPolicy::new().is_disabled());
}

#[test]
fn identifier_ordering_hash_equality() {
    assert_eq!(TaskClass::new("a"), TaskClass::new("a"));
    assert!(TaskClass::new("a") < TaskClass::new("b"));

    let mut set = BTreeSet::new();
    set.insert(TaskStage::new("io"));
    set.insert(TaskStage::new("io"));
    set.insert(TaskStage::new("cpu"));
    assert_eq!(set.len(), 2);
    // BTreeSet yields deterministic sorted order.
    let ordered: Vec<_> = set.iter().map(|s| s.as_str().to_string()).collect();
    assert_eq!(ordered, vec!["cpu".to_string(), "io".to_string()]);
}

#[test]
fn memory_unit_scale_rounds_up() {
    let scale = MemoryUnitScale {
        bytes_per_unit: 4096,
    };
    assert_eq!(scale.units_for(0), 0);
    assert_eq!(scale.units_for(1), 1);
    assert_eq!(scale.units_for(4096), 1);
    assert_eq!(scale.units_for(4097), 2);
}
