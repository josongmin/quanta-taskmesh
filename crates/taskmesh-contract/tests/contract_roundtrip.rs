//! T01: serde roundtrip proofs for the frozen wire contract.

use std::collections::BTreeMap;

use taskmesh_contract::*;

fn sample_class_policy() -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(8)
        .max_queue_depth(16)
        .cpu_units(2)
        .memory_units(4)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .retry_after_policy(RetryAfterPolicy::Adaptive)
        .fairness(FairnessPolicy::WeightedFairQueue {
            weight: 4,
            burst: 2,
        })
}

#[test]
fn task_spec_roundtrips() {
    let spec = TaskSpec::blocking(TaskClass::new("retrieval"))
        .operation("search:repo:1")
        .stage(TaskStage::new("rank"), SubstrateHint::SharedCpuExecutor)
        .reduce_stage(
            TaskStage::new("merge"),
            SubstrateHint::SharedCpuExecutor,
            DeterministicReducePolicy::keyed("doc_id"),
        );
    let json = serde_json::to_string(&spec).unwrap();
    let back: TaskSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(spec, back);
}

#[test]
fn class_policy_roundtrips() {
    let policy = sample_class_policy();
    let json = serde_json::to_string_pretty(&policy).unwrap();
    let back: ClassPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(policy, back);
}

#[test]
fn runtime_config_roundtrips_pretty() {
    let mut classes = BTreeMap::new();
    classes.insert(TaskClass::new("retrieval"), sample_class_policy());
    let config = RuntimeConfig {
        topology: TopologyConfig::new()
            .cpu_auto()
            .reserve_cores(1)
            .blocking_threads(8),
        resources: ResourceBudget::new()
            .cpu_units(64)
            .memory_units(256)
            .memory_unit_scale(4096),
        classes,
        substrates: vec![SubstrateRecord::new(
            "external-gpu",
            SubstrateKind::CompetingExecution,
            Some("external-gpu"),
        )],
    };
    let json = serde_json::to_string_pretty(&config).unwrap();
    let back: RuntimeConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(config, back);
}

#[test]
fn snapshot_roundtrips() {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("retrieval"),
        ClassSnapshot {
            inflight: 3,
            queued: 5,
            cpu_units_held: 6,
            memory_units_held: 12,
        },
    );
    let snapshot = Snapshot {
        classes,
        substrates: vec![SubstrateRecord::new(
            "cpu",
            SubstrateKind::CompetingExecution,
            Some("cpu"),
        )],
    };
    let json = serde_json::to_string(&snapshot).unwrap();
    let back: Snapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(snapshot, back);
}
