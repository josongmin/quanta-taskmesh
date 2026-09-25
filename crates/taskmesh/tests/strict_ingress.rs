//! BG25-002: strict bytes are a separate authority boundary from raw DTO Serde.

use serde_json::{json, Value};
use taskmesh::{
    parse_runtime_config, parse_task_spec, ClassPolicy, ResourceBudget, StrictIngressError,
    StrictIngressLimits, SubstrateHint, TaskClass, TaskSpec, TopologyConfig,
};

fn task() -> Value {
    json!({
        "class": "c",
        "operation": "root",
        "root_operation_id": "root",
        "source": "internal",
        "reason": "ExplicitMapping",
        "scope": "Root",
        "blocking_dispatch": "shared_blocking",
        "stages": [{
            "class": "c",
            "stage": "blocking",
            "substrate_hint": "BlockingPool",
            "fan_out": false,
            "reduce_policy": null
        }]
    })
}

fn parse(
    value: &Value,
    limits: StrictIngressLimits,
) -> Result<taskmesh::ValidatedTaskPlan, StrictIngressError> {
    parse_task_spec(&serde_json::to_vec(value).unwrap(), limits)
}

fn config() -> Value {
    json!({
        "topology": TopologyConfig::new().cpu_fixed(1).shared_blocking_domain(taskmesh::PhysicalDomainMode::Fixed(1)),
        "resources": ResourceBudget::new().cpu_units(4).memory_units(4),
        "classes": {"c": ClassPolicy::new().max_inflight(1)},
        "extra_substrates": [],
        "capability_limits": {}
    })
}

#[test]
fn task_limits_reject_before_typed_decode_or_stage_allocation() {
    let value = task();
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(parse_task_spec(&bytes, StrictIngressLimits::default()).is_ok());
    assert_eq!(
        parse_task_spec(
            &bytes,
            StrictIngressLimits {
                max_bytes: bytes.len() - 1,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::ByteLimit {
            actual: bytes.len(),
            max: bytes.len() - 1
        })
    );
    assert!(parse_task_spec(
        &bytes,
        StrictIngressLimits {
            max_bytes: bytes.len(),
            ..StrictIngressLimits::default()
        }
    )
    .is_ok());
    assert_eq!(
        parse_task_spec(
            &bytes,
            StrictIngressLimits {
                max_stages: 0,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::InvalidLimit("max_stages"))
    );
    assert_eq!(
        parse_task_spec(
            &bytes,
            StrictIngressLimits {
                max_depth: 2,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::DepthLimit { max: 2 })
    );
    assert!(parse_task_spec(
        &bytes,
        StrictIngressLimits {
            max_depth: 3,
            ..StrictIngressLimits::default()
        }
    )
    .is_ok());

    let mut two = value;
    two["stages"].as_array_mut().unwrap().push(json!({
        "class": "c", "stage": "second", "substrate_hint": "BlockingPool",
        "fan_out": false, "reduce_policy": null
    }));
    assert_eq!(
        parse(
            &two,
            StrictIngressLimits {
                max_stages: 1,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::StageLimit { max: 1 })
    );
    assert!(parse(
        &two,
        StrictIngressLimits {
            max_stages: 2,
            ..StrictIngressLimits::default()
        }
    )
    .is_ok());
}

#[test]
fn duplicate_and_unknown_keys_reject_at_every_nested_boundary() {
    let bytes = serde_json::to_vec(&task()).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let duplicate = text.replacen(
        "\"operation\":\"root\"",
        "\"operation\":\"root\",\"operation\":\"other\"",
        1,
    );
    assert_eq!(
        parse_task_spec(duplicate.as_bytes(), StrictIngressLimits::default()),
        Err(StrictIngressError::DuplicateKey {
            key: "operation".into()
        })
    );

    let mut unknown_stage = task();
    unknown_stage["stages"][0]["fan_otu"] = json!(true);
    assert_eq!(
        parse(&unknown_stage, StrictIngressLimits::default()),
        Err(StrictIngressError::UnknownKey {
            object: "stage",
            key: "fan_otu".into()
        })
    );
    let mut unknown_root = task();
    unknown_root["stack_size_byte"] = json!(123456);
    assert_eq!(
        parse(&unknown_root, StrictIngressLimits::default()),
        Err(StrictIngressError::UnknownKey {
            object: "task",
            key: "stack_size_byte".into()
        })
    );
}

#[test]
fn child_wait_and_blocking_dispatch_are_explicit() {
    let mut child = task();
    child["operation"] = json!("child");
    child["scope"] = json!({
        "Child": {"parent_operation_id": "root", "parent_stage": "blocking", "parent_awaits": false}
    });
    assert!(parse(&child, StrictIngressLimits::default()).is_ok());
    child["scope"]["Child"]
        .as_object_mut()
        .unwrap()
        .remove("parent_awaits");
    assert!(matches!(
        parse(&child, StrictIngressLimits::default()),
        Err(StrictIngressError::Decode(_))
    ));
    child["scope"]["Child"]["parent_await"] = json!(true);
    assert_eq!(
        parse(&child, StrictIngressLimits::default()),
        Err(StrictIngressError::UnknownKey {
            object: "child",
            key: "parent_await".into()
        })
    );

    let mut missing_dispatch = task();
    missing_dispatch
        .as_object_mut()
        .unwrap()
        .remove("blocking_dispatch");
    assert_eq!(
        parse(&missing_dispatch, StrictIngressLimits::default()),
        Err(StrictIngressError::BlockingDispatch(
            "blocking-family task requires an explicit dispatch tag"
        ))
    );
    let mut stack = task();
    stack["blocking_dispatch"] = json!({"requested_stack": {"stack_size_bytes": 1_048_576}});
    assert_eq!(
        parse(&stack, StrictIngressLimits::default())
            .unwrap()
            .as_spec()
            .stack_size_bytes,
        Some(1_048_576)
    );
    stack["blocking_dispatch"] = json!({"requested_stack": {"stack_size_byte": 1_048_576}});
    assert_eq!(
        parse(&stack, StrictIngressLimits::default()),
        Err(StrictIngressError::UnknownKey {
            object: "requested_stack",
            key: "stack_size_byte".into()
        })
    );

    let mut io = task();
    io["stages"][0]["substrate_hint"] = json!("AsyncIo");
    assert!(matches!(
        parse(&io, StrictIngressLimits::default()),
        Err(StrictIngressError::BlockingDispatch(_))
    ));
}

#[test]
fn config_rejects_unknown_and_duplicate_before_builder_promotion() {
    let valid = config();
    let bytes = serde_json::to_vec(&valid).unwrap();
    let parsed = parse_runtime_config(&bytes, StrictIngressLimits::default()).unwrap();
    let runtime = parsed
        .into_builder()
        .build()
        .expect("ordinary Builder validation remains authoritative");
    assert_eq!(runtime.config().classes.len(), 1);

    let mut unknown = valid.clone();
    unknown["topology"]["physical_domans"] = json!({"dedicated": "Auto"});
    assert!(matches!(
        parse_runtime_config(&serde_json::to_vec(&unknown).unwrap(), StrictIngressLimits::default()),
        Err(StrictIngressError::UnknownKey { object: "topology", key }) if key == "physical_domans"
    ));
    let mut nested = valid;
    nested["classes"]["c"]["checkpoint_policy"]["before_reduse"] = json!(true);
    assert!(matches!(
        parse_runtime_config(&serde_json::to_vec(&nested).unwrap(), StrictIngressLimits::default()),
        Err(StrictIngressError::UnknownKey { object: "checkpoint_policy", key }) if key == "before_reduse"
    ));
    let duplicate = r#"{"topology":{},"topology":{},"resources":{},"classes":{}}"#;
    assert_eq!(
        parse_runtime_config(duplicate.as_bytes(), StrictIngressLimits::default()).err(),
        Some(StrictIngressError::DuplicateKey {
            key: "topology".into()
        })
    );
    let nested_duplicate =
        r#"{"topology":{},"resources":{"max_cpu_units":1,"max_cpu_units":2},"classes":{}}"#;
    assert_eq!(
        parse_runtime_config(nested_duplicate.as_bytes(), StrictIngressLimits::default()).err(),
        Some(StrictIngressError::DuplicateKey {
            key: "max_cpu_units".into()
        })
    );
}

#[test]
fn strict_config_accepts_supported_nested_variant_payloads() {
    use taskmesh::{
        CheckpointPolicy, FairnessPolicy, MemoryOvercommitPolicy, RetryAfterPolicy, SubstrateKind,
        SubstrateRecord,
    };

    let policy = ClassPolicy::new()
        .fairness(FairnessPolicy::WeightedFairQueue {
            weight: 2,
            burst: 1,
        })
        .retry_after_policy(RetryAfterPolicy::FixedMs(7))
        .memory_overcommit_policy(MemoryOvercommitPolicy::DegradeToLight {
            fallback_class: TaskClass::new("light"),
        })
        .checkpoint_policy(CheckpointPolicy {
            every_n_work_items: Some(10),
            before_fan_out: true,
            before_large_allocation: false,
            before_stage_boundary: true,
            before_reduce: true,
        });
    let mut value = config();
    value["classes"]["c"] = json!(policy);
    value["classes"]["light"] = json!(ClassPolicy::new());
    value["extra_substrates"] = json!([SubstrateRecord::new(
        "external",
        SubstrateKind::CompetingExecution,
        Some("external-pool"),
    )]);
    value["capability_limits"]["external-pool"] = json!(1);
    parse_runtime_config(
        &serde_json::to_vec(&value).unwrap(),
        StrictIngressLimits::default(),
    )
    .expect("all currently supported nested config shapes must decode");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_dispatch_tags_charge_the_actual_worker_domain() {
    use std::time::Duration;
    use taskmesh::{Builder, PhysicalDomainMode, RunError, Runtime};
    use tokio::sync::oneshot;

    let runtime = Builder::new()
        .topology(
            TopologyConfig::new()
                .cpu_fixed(1)
                .blocking_threads(1)
                .large_stack_slots(1)
                .shared_blocking_domain(PhysicalDomainMode::Fixed(1))
                .dedicated_domain(PhysicalDomainMode::Fixed(1)),
        )
        .class_policy(TaskClass::new("c"), ClassPolicy::new().max_inflight(1))
        .build()
        .unwrap();

    for (dispatch, role, domain, other_role, other_domain) in [
        (
            json!("shared_blocking"),
            "blocking",
            "physical.shared_blocking",
            "large_stack",
            "physical.dedicated",
        ),
        (
            json!({"requested_stack": {"stack_size_bytes": 1_048_576}}),
            "large_stack",
            "physical.dedicated",
            "blocking",
            "physical.shared_blocking",
        ),
    ] {
        let mut input = task();
        input["blocking_dispatch"] = dispatch;
        let plan = parse(&input, StrictIngressLimits::default()).unwrap();
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let worker_runtime = runtime.clone();
        let worker = tokio::spawn(async move {
            worker_runtime
                .run_blocking(plan.into_spec(), move || {
                    let _ = entered_tx.send(());
                    let _ = release_rx.blocking_recv();
                    Ok::<_, ()>(())
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), entered_rx)
            .await
            .expect("worker must enter")
            .expect("entry signal");
        let held = runtime.snapshot();
        assert_eq!(held.capabilities[role].in_use, 1, "semantic role");
        assert_eq!(held.capabilities[domain].in_use, 1, "physical domain");
        assert_eq!(held.capabilities[other_role].in_use, 0);
        assert_eq!(held.capabilities[other_domain].in_use, 0);
        let _ = release_tx.send(());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), worker)
                .await
                .expect("worker must complete")
                .expect("join"),
            Ok::<_, RunError<()>>(())
        );
        assert_eq!(runtime.snapshot().classes[&TaskClass::new("c")].inflight, 0);
    }
}

#[test]
fn raw_dto_compatibility_is_not_redefined_by_strict_ingress() {
    let raw = TaskSpec::blocking(TaskClass::new("c")).operation("root");
    let wire = serde_json::to_vec(&raw).unwrap();
    let decoded: TaskSpec = serde_json::from_slice(&wire).unwrap();
    assert_eq!(
        decoded.primary_substrate_hint(),
        SubstrateHint::BlockingPool
    );
    assert_eq!(decoded.stack_size_bytes, None);
    assert_eq!(raw, decoded);
}

#[test]
fn missing_raw_await_flag_queues_but_strict_declared_wait_rejects_the_cycle() {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use taskmesh::ext::{AdmissionDecision, Governor, ManualClock, PolicySet, ReleaseOutcome};
    use taskmesh::{AdmissionVerdict, OverflowPolicy, TaskStage};

    let class = TaskClass::new("c");
    let classes = BTreeMap::from([(
        class.clone(),
        ClassPolicy::new()
            .max_inflight(1)
            .max_queue_depth(1)
            .overflow_policy(OverflowPolicy::QueueWithinDepth),
    )]);
    let governor = Governor::new(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(0)),
    )
    .unwrap();
    let parent = TaskSpec::io(class.clone()).operation("parent");
    let AdmissionDecision::Admitted { permit_id } = governor.admit(&parent) else {
        panic!("parent must take the only class slot");
    };
    let child = TaskSpec::io(class)
        .awaited_child_of("parent", "parent", TaskStage::new("child"))
        .operation("child");
    let mut input = serde_json::to_value(child).unwrap();
    input.as_object_mut().unwrap().remove("stack_size_bytes");
    input["scope"]["Child"]
        .as_object_mut()
        .unwrap()
        .remove("parent_awaits");

    let raw: TaskSpec = serde_json::from_value(input.clone()).unwrap();
    assert!(matches!(
        raw.scope,
        taskmesh::TaskScope::Child {
            parent_awaits: false,
            ..
        }
    ));
    let AdmissionDecision::Queued { ticket } = governor.admit(&raw) else {
        panic!("raw missing flag silently becomes an unawaited queued child");
    };
    assert!(matches!(
        parse(&input, StrictIngressLimits::default()),
        Err(StrictIngressError::Decode(_))
    ));
    governor.abandon(ticket);

    input["scope"]["Child"]["parent_awaits"] = json!(true);
    let declared = parse(&input, StrictIngressLimits::default()).unwrap();
    let before = governor.snapshot();
    assert!(matches!(
        governor.admit_validated(&declared),
        AdmissionDecision::Rejected(AdmissionVerdict::NestedWaitCycle { .. })
    ));
    assert_eq!(
        governor.snapshot(),
        before,
        "cycle rejection is side-effect free"
    );
    assert_eq!(governor.release(permit_id), ReleaseOutcome::Released);
}
