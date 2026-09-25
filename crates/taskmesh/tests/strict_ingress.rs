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
