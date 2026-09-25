//! BG25-002: strict bytes are a separate authority boundary from raw DTO Serde.

use serde_json::{json, Value};
use taskmesh::{
    parse_runtime_config, parse_task_spec, ClassPolicy, DeterministicReducePolicy,
    DuplicateMergePolicy, ErrorAggregationPolicy, PartialResultOrdering, ResourceBudget,
    StrictIngressError, StrictIngressLimits, SubstrateHint, TaskClass, TaskSpec, TaskStage,
    TieBreakPolicy, TopologyConfig,
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

#[test]
fn strict_ingress_error_display_is_stable() {
    use taskmesh_contract::TaskPlanError;

    let cases = [
        (
            StrictIngressError::InvalidLimit("max_bytes"),
            "strict ingress limit max_bytes must be nonzero",
        ),
        (
            StrictIngressError::ByteLimit { actual: 5, max: 4 },
            "strict ingress input is 5 bytes, above 4",
        ),
        (
            StrictIngressError::DepthLimit { max: 3 },
            "strict ingress depth exceeds 3",
        ),
        (
            StrictIngressError::StageLimit { max: 2 },
            "strict ingress stages exceed 2",
        ),
        (
            StrictIngressError::DuplicateKey { key: "x".into() },
            "duplicate JSON key \"x\"",
        ),
        (
            StrictIngressError::UnknownKey {
                object: "task",
                key: "x".into(),
            },
            "unknown key \"x\" in task",
        ),
        (
            StrictIngressError::InvalidShape("task"),
            "invalid JSON shape for task",
        ),
        (
            StrictIngressError::Decode("bad".into()),
            "invalid strict JSON: bad",
        ),
        (
            StrictIngressError::TaskPlan(TaskPlanError::NoStages),
            "invalid task plan: task plan must declare at least one stage",
        ),
        (
            StrictIngressError::BlockingDispatch("bad"),
            "invalid blocking dispatch: bad",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
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
    let expected = TaskSpec::blocking(TaskClass::new("c"))
        .operation("root")
        .validate()
        .expect("independently constructed valid task");
    assert_eq!(
        parse_task_spec(&bytes, StrictIngressLimits::default()),
        Ok(expected.clone())
    );
    assert_eq!(
        parse_task_spec(
            &bytes,
            StrictIngressLimits {
                max_bytes: 0,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::InvalidLimit("max_bytes"))
    );
    assert_eq!(
        parse_task_spec(
            &bytes,
            StrictIngressLimits {
                max_bytes: 1,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::ByteLimit {
            actual: bytes.len(),
            max: 1
        })
    );
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
    assert_eq!(
        parse_task_spec(
            &bytes,
            StrictIngressLimits {
                max_bytes: bytes.len(),
                ..StrictIngressLimits::default()
            }
        ),
        Ok(expected.clone())
    );
    let mut over_bytes = bytes.clone();
    over_bytes.push(b' ');
    assert_eq!(
        parse_task_spec(
            &over_bytes,
            StrictIngressLimits {
                max_bytes: bytes.len(),
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::ByteLimit {
            actual: bytes.len() + 1,
            max: bytes.len()
        })
    );
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
                max_depth: 0,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::InvalidLimit("max_depth"))
    );
    assert_eq!(
        parse_task_spec(
            &bytes,
            StrictIngressLimits {
                max_depth: 1,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::DepthLimit { max: 1 })
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
    assert_eq!(
        parse_task_spec(
            &bytes,
            StrictIngressLimits {
                max_depth: 3,
                ..StrictIngressLimits::default()
            }
        ),
        Ok(expected)
    );

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
    two["stages"][1] = json!({"unknown": {"deep": [1, 2, 3]}});
    assert_eq!(
        parse(
            &two,
            StrictIngressLimits {
                max_stages: 1,
                ..StrictIngressLimits::default()
            }
        ),
        Err(StrictIngressError::StageLimit { max: 1 }),
        "the over-limit body must not be decoded"
    );
    two["stages"][1] = json!({
        "class": "c", "stage": "second", "substrate_hint": "BlockingPool",
        "fan_out": false, "reduce_policy": null
    });
    let expected_two = TaskSpec::blocking(TaskClass::new("c"))
        .operation("root")
        .stage(TaskStage::new("second"), SubstrateHint::BlockingPool)
        .validate()
        .expect("two distinct declared stages");
    assert_eq!(
        parse(
            &two,
            StrictIngressLimits {
                max_stages: 2,
                ..StrictIngressLimits::default()
            }
        ),
        Ok(expected_two)
    );
}

#[test]
fn zero_duplicate_and_many_stages_have_distinct_strict_ingress_verdicts() {
    use taskmesh_contract::TaskPlanError;

    let mut zero = task();
    zero["stages"] = json!([]);
    zero.as_object_mut().unwrap().remove("blocking_dispatch");
    assert_eq!(
        parse(&zero, StrictIngressLimits::default()),
        Err(StrictIngressError::TaskPlan(TaskPlanError::NoStages))
    );

    let mut duplicate = task();
    let first = duplicate["stages"][0].clone();
    duplicate["stages"].as_array_mut().unwrap().push(first);
    assert_eq!(
        parse(&duplicate, StrictIngressLimits::default()),
        Err(StrictIngressError::TaskPlan(
            TaskPlanError::DuplicateStage {
                stage: TaskStage::new("blocking"),
            }
        ))
    );

    let mut many = task();
    let stages = many["stages"].as_array_mut().unwrap();
    for index in 1..=StrictIngressLimits::default().max_stages {
        let mut stage = stages[0].clone();
        stage["stage"] = json!(format!("stage-{index}"));
        stages.push(stage);
    }
    assert_eq!(
        parse(&many, StrictIngressLimits::default()),
        Err(StrictIngressError::StageLimit {
            max: StrictIngressLimits::default().max_stages,
        })
    );
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
    unknown_root["stack_size_byte"] = json!(123_456);
    assert_eq!(
        parse(&unknown_root, StrictIngressLimits::default()),
        Err(StrictIngressError::UnknownKey {
            object: "task",
            key: "stack_size_byte".into()
        })
    );
}

#[test]
fn strict_ingress_accepts_complete_reduce_policy_and_rejects_field_errors() {
    let policy = json!({
        "stable_sort_key": "document_id",
        "duplicate_merge": "KeepLastStable",
        "tie_break": "Lexicographic",
        "error_aggregation": "AllStable",
        "partial_result_ordering": "StableStageOrder"
    });
    let mut value = task();
    value["stages"][0]["fan_out"] = json!(true);
    value["stages"][0]["reduce_policy"] = policy.clone();

    let mut expected = TaskSpec::blocking(TaskClass::new("c")).operation("root");
    expected.stages[0].fan_out = true;
    expected.stages[0].reduce_policy = Some(DeterministicReducePolicy {
        stable_sort_key: "document_id".into(),
        duplicate_merge: DuplicateMergePolicy::KeepLastStable,
        tie_break: TieBreakPolicy::Lexicographic,
        error_aggregation: ErrorAggregationPolicy::AllStable,
        partial_result_ordering: PartialResultOrdering::StableStageOrder,
    });
    let parsed = parse(&value, StrictIngressLimits::default()).expect("complete reduce policy");
    assert_eq!(
        parsed,
        expected.validate().expect("independent builder policy")
    );
    assert_eq!(
        serde_json::to_value(&parsed.as_spec().stages[0].reduce_policy).unwrap(),
        policy
    );

    for field in [
        "stable_sort_key",
        "duplicate_merge",
        "tie_break",
        "error_aggregation",
        "partial_result_ordering",
    ] {
        let mut typo = value.clone();
        let reduce = typo["stages"][0]["reduce_policy"].as_object_mut().unwrap();
        let original = reduce.remove(field).unwrap();
        let misspelled = format!("{field}_typo");
        reduce.insert(misspelled.clone(), original);
        assert_eq!(
            parse(&typo, StrictIngressLimits::default()),
            Err(StrictIngressError::UnknownKey {
                object: "reduce_policy",
                key: misspelled,
            }),
            "unknown {field} variant must be rejected before typed decode"
        );

        let mut missing = value.clone();
        missing["stages"][0]["reduce_policy"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            matches!(
                parse(&missing, StrictIngressLimits::default()),
                Err(StrictIngressError::Decode(_))
            ),
            "missing {field} must not silently take a default"
        );
    }
}

#[test]
fn child_wait_and_blocking_dispatch_are_explicit() {
    let mut child = task();
    child["operation"] = json!("child");
    child["scope"] = json!({
        "Child": {"parent_operation_id": "root", "parent_stage": "blocking", "parent_awaits": false}
    });
    let expected_child = TaskSpec::blocking(TaskClass::new("c"))
        .child_of("root", "root", TaskStage::new("blocking"))
        .operation("child")
        .validate()
        .expect("explicit unawaited child");
    assert_eq!(
        parse(&child, StrictIngressLimits::default()),
        Ok(expected_child)
    );
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

#[test]
fn strict_config_fairness_variants_require_exact_names_and_payload_keys() {
    use taskmesh::FairnessPolicy;

    for (wire, expected, variant, fields) in [
        (
            json!({"WeightedFairQueue": {"weight": 3, "burst": 2}}),
            FairnessPolicy::WeightedFairQueue {
                weight: 3,
                burst: 2,
            },
            "WeightedFairQueue",
            &[("weight", "weigth"), ("burst", "brust")][..],
        ),
        (
            json!({"DeficitRoundRobin": {"quantum": 7}}),
            FairnessPolicy::DeficitRoundRobin { quantum: 7 },
            "DeficitRoundRobin",
            &[("quantum", "quantm")][..],
        ),
        (
            json!({"DeadlineAware": {"slack_ms": 29}}),
            FairnessPolicy::DeadlineAware { slack_ms: 29 },
            "DeadlineAware",
            &[("slack_ms", "slack_mss")][..],
        ),
    ] {
        let mut valid = config();
        valid["classes"]["c"]["fairness"] = wire;
        let runtime = parse_runtime_config(
            &serde_json::to_vec(&valid).unwrap(),
            StrictIngressLimits::default(),
        )
        .expect("explicit fairness variant must pass strict scanning")
        .into_builder()
        .build()
        .expect("valid fairness policy must build");
        assert_eq!(
            runtime.config().classes[&TaskClass::new("c")].fairness,
            expected
        );

        for &(field, typo) in fields {
            let mut misspelled = valid.clone();
            let payload = misspelled["classes"]["c"]["fairness"][variant]
                .as_object_mut()
                .unwrap();
            let original = payload.remove(field).unwrap();
            payload.insert(typo.into(), original);
            assert_eq!(
                parse_runtime_config(
                    &serde_json::to_vec(&misspelled).unwrap(),
                    StrictIngressLimits::default(),
                )
                .err(),
                Some(StrictIngressError::UnknownKey {
                    object: variant,
                    key: typo.into(),
                })
            );

            let mut missing = valid.clone();
            missing["classes"]["c"]["fairness"][variant]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(matches!(
                parse_runtime_config(
                    &serde_json::to_vec(&missing).unwrap(),
                    StrictIngressLimits::default(),
                ),
                Err(StrictIngressError::Decode(_))
            ));
        }
    }
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
                    entered_tx.send(()).expect("caller waits for worker entry");
                    release_rx
                        .blocking_recv()
                        .expect("caller releases the held worker");
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
        release_tx
            .send(())
            .expect("worker still holds the receiver");
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

    let mut misspelled_stack = serde_json::to_value(&raw).unwrap();
    misspelled_stack
        .as_object_mut()
        .unwrap()
        .remove("stack_size_bytes");
    misspelled_stack["stack_size_byte"] = json!(1_048_576);
    let legacy: TaskSpec = serde_json::from_value(misspelled_stack.clone()).unwrap();
    assert_eq!(
        legacy.stack_size_bytes, None,
        "raw Serde still defaults to shared"
    );
    assert_eq!(
        legacy
            .clone()
            .validate()
            .expect("raw DTO compatibility is unchanged")
            .as_spec(),
        &legacy
    );
    assert_eq!(
        parse(&misspelled_stack, StrictIngressLimits::default()),
        Err(StrictIngressError::UnknownKey {
            object: "task",
            key: "stack_size_byte".into(),
        })
    );
}

#[test]
fn missing_raw_await_flag_queues_but_strict_declared_wait_rejects_the_cycle() {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use taskmesh::ext::{
        AbandonOutcome, AdmissionDecision, Governor, ManualClock, PolicySet, ReleaseOutcome,
    };
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
    assert_eq!(governor.abandon(ticket), AbandonOutcome::Abandoned);

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
