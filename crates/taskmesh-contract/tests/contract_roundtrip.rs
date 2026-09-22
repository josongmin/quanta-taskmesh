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
fn validated_task_plan_roundtrips_without_reopening_the_boundary() {
    let plan = TaskSpec::blocking(TaskClass::new("retrieval"))
        .operation("search:repo:1")
        .validate()
        .expect("valid plan");
    let json = serde_json::to_string(&plan).unwrap();
    let back: ValidatedTaskPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(plan, back);
}

#[test]
#[allow(deprecated)]
fn legacy_plan_source_strings_decode_and_reencode_exactly() {
    for (wire, constant) in [
        ("PublicSdk", PlanSource::PublicSdk),
        ("FluentSdk", PlanSource::FluentSdk),
        ("SearchAdapter", PlanSource::SearchAdapter),
        ("Warmup", PlanSource::Warmup),
        ("Indexing", PlanSource::Indexing),
        ("Internal", PlanSource::Internal),
    ] {
        let decoded: PlanSource = serde_json::from_str(&format!(r#""{wire}""#)).unwrap();
        assert_eq!(decoded, constant);
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            format!(r#""{wire}""#)
        );
    }
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
            // Deliberately past `u32::MAX`: an unlimited budget (`0`) lets a
            // class hold more than a single request's `u32` cost, and the wire
            // form has to survive that without narrowing.
            cpu_units_held: u128::from(u32::MAX) + 6,
            memory_units_held: u128::from(u64::MAX) + 12,
            dispatch_reserved: 1,
            accepted: 1,
            running: 1,
            cleanup_pending: 0,
            admitted_total: 9,
            started_total: 4,
            terminated_total: 6,
        },
    );
    let snapshot = Snapshot {
        schema_version: taskmesh_contract::SNAPSHOT_SCHEMA_VERSION,
        classes,
        substrates: vec![SubstrateRecord::new(
            "cpu",
            SubstrateKind::CompetingExecution,
            Some("cpu"),
        )],
        capabilities: BTreeMap::from([(
            "cpu".to_string(),
            taskmesh_contract::CapabilityUsage {
                in_use: 3,
                limit: 8,
            },
        )]),
    };
    let json = serde_json::to_string(&snapshot).unwrap();
    let back: Snapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(snapshot, back);
}

#[test]
fn exact_resource_aggregates_cross_the_wire_as_decimal_strings() {
    // A JSON number would round-trip a large `u128` through an `f64` and come
    // back wrong, which is exactly the kind of quiet narrowing that makes an
    // over-budget total look healthy. The wire form is a string for that reason.
    let class = ClassSnapshot {
        cpu_units_held: u128::MAX,
        memory_units_held: u128::MAX - 1,
        ..ClassSnapshot::default()
    };
    let json = serde_json::to_string(&class).unwrap();
    assert!(
        json.contains(&format!("\"{}\"", u128::MAX)),
        "exact units must serialize as a decimal string: {json}"
    );
    let back: ClassSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(back.cpu_units_held, u128::MAX);
    assert_eq!(back.memory_units_held, u128::MAX - 1);
}
