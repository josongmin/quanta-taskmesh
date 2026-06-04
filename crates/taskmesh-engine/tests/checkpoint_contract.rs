//! T06: checkpoint policy is enforceable metadata preserved through core intake.

use std::collections::BTreeMap;
use std::sync::Arc;

use taskmesh_contract::*;
use taskmesh_engine::*;

#[test]
fn checkpoint_metadata_survives_core_intake() {
    let checkpoint = CheckpointPolicy {
        every_n_work_items: Some(256),
        before_fan_out: true,
        before_large_allocation: false,
        before_stage_boundary: true,
        before_reduce: false,
    };
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("c"),
        ClassPolicy::new().checkpoint_policy(checkpoint),
    );
    let g = Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(0)),
    );

    let recovered = g
        .checkpoint_policy(&TaskClass::new("c"))
        .expect("class exists");
    assert_eq!(recovered, checkpoint);
    assert_eq!(recovered.every_n_work_items, Some(256));
    assert!(g.checkpoint_policy(&TaskClass::new("missing")).is_none());
}
