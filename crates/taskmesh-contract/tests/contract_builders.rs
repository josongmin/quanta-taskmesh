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
fn operation_after_child_of_preserves_root_id() {
    // README order: child_of(...).operation(...). Naming the child's own
    // operation must NOT re-root it — root attribution & recursion key depend
    // on the inherited root id.
    let child = TaskSpec::cpu(TaskClass::new("rank"))
        .child_of("fetch:doc:42", "fetch:doc:42", TaskStage::new("fanout"))
        .operation("rank:shard:3");
    assert_eq!(
        child.root_operation_id, "fetch:doc:42",
        "child root must survive"
    );
    assert_eq!(child.operation, "rank:shard:3");
    assert!(matches!(child.scope, TaskScope::Child { .. }));

    // The reverse order (operation then child_of) still re-roots, as child_of
    // explicitly sets the parent root.
    let child2 = TaskSpec::cpu(TaskClass::new("rank"))
        .operation("rank:shard:3")
        .child_of("fetch:doc:42", "fetch:doc:42", TaskStage::new("fanout"));
    assert_eq!(child2.root_operation_id, "fetch:doc:42");

    // A plain root is still keyed by its own operation name.
    let root = TaskSpec::io(TaskClass::new("c")).operation("op-1");
    assert_eq!(root.root_operation_id, "op-1");
}

#[test]
fn child_of_inherits_root_id_and_sets_scope() {
    let child =
        TaskSpec::cpu(TaskClass::new("c")).child_of("root-7", "parent-6", TaskStage::new("fanout"));
    assert_eq!(child.root_operation_id, "root-7");
    assert!(matches!(child.scope, TaskScope::Child { .. }));
}

#[test]
fn only_awaited_child_of_declares_that_the_parent_waits() {
    // D12: lineage (`child_of`) says nothing about who waits for whom; the
    // wait is a separate, explicit declaration. Both re-root the child.
    let lineage =
        TaskSpec::cpu(TaskClass::new("c")).child_of("root-7", "parent-6", TaskStage::new("fanout"));
    assert_eq!(
        lineage.scope,
        TaskScope::Child {
            parent_operation_id: "parent-6".to_string(),
            parent_stage: TaskStage::new("fanout"),
            parent_awaits: false,
        },
        "child_of must not declare a wait the caller did not"
    );
    let awaited = TaskSpec::cpu(TaskClass::new("c")).awaited_child_of(
        "root-7",
        "parent-6",
        TaskStage::new("fanout"),
    );
    assert_eq!(awaited.root_operation_id, "root-7");
    assert_eq!(
        awaited.scope,
        TaskScope::Child {
            parent_operation_id: "parent-6".to_string(),
            parent_stage: TaskStage::new("fanout"),
            parent_awaits: true,
        }
    );

    // The declaration is on the wire. A legacy child payload without an exact
    // immediate-parent identity is ambiguous and rejected.
    let json = serde_json::to_string(&awaited.scope).expect("serializes");
    assert!(json.contains("\"parent_awaits\":true"), "{json}");
    let error = serde_json::from_str::<TaskScope>(r#"{"Child":{"parent_stage":"fanout"}}"#)
        .expect_err("legacy child scope without immediate-parent identity must reject");
    assert!(
        error.to_string().contains("parent_operation_id"),
        "the wire error must identify the missing immediate parent: {error}"
    );
}

#[test]
fn product_neutral_plan_source_is_bounded() {
    let source = PlanSource::new("adapter.v2").expect("valid opaque key");
    assert_eq!(source.as_str(), "adapter.v2");
    assert_eq!(serde_json::to_string(&source).unwrap(), r#""adapter.v2""#);

    assert_eq!(
        PlanSource::new("").unwrap_err(),
        TaskPlanError::InvalidIdentifier {
            field: TaskIdentifierField::PlanSource,
            violation: IdentifierViolation::Empty,
        }
    );
    assert_eq!(
        PlanSource::new("x".repeat(MAX_TASK_IDENTIFIER_LEN + 1)).unwrap_err(),
        TaskPlanError::InvalidIdentifier {
            field: TaskIdentifierField::PlanSource,
            violation: IdentifierViolation::TooLong {
                actual: MAX_TASK_IDENTIFIER_LEN + 1,
                max: MAX_TASK_IDENTIFIER_LEN,
            },
        }
    );
}

#[test]
fn builder_defaults_smoke() {
    assert_eq!(ClassPolicy::new(), ClassPolicy::default());
    assert_eq!(ResourceBudget::new(), ResourceBudget::default());
    assert_eq!(TopologyConfig::new(), TopologyConfig::default());
    assert!(ClassPolicy::new().max_inflight(0).is_disabled());
    assert!(!ClassPolicy::new().is_disabled());
}

#[test]
fn class_policy_builder_preserves_every_explicit_field() {
    let checkpoint = CheckpointPolicy {
        every_n_work_items: Some(7),
        before_fan_out: false,
        before_large_allocation: false,
        before_stage_boundary: false,
        before_reduce: false,
    };
    let policy = ClassPolicy::new()
        .max_inflight(11)
        .max_queue_depth(12)
        .cpu_units(13)
        .memory_units(14)
        .fairness(FairnessPolicy::DeficitRoundRobin { quantum: 15 })
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
        .retry_after_policy(RetryAfterPolicy::FixedMs(16))
        .memory_permit_mode(MemoryPermitMode::Hybrid)
        .memory_release_policy(MemoryReleasePolicy::OnStageBoundary)
        .memory_overcommit_policy(MemoryOvercommitPolicy::DegradeToLight {
            fallback_class: TaskClass::new("fallback"),
        })
        .cancellation_policy(CancellationPolicy::CooperativeWithDeadline)
        .checkpoint_policy(checkpoint)
        .best_effort(true);

    assert_eq!(policy.max_inflight, 11);
    assert_eq!(policy.max_queue_depth, 12);
    assert_eq!(policy.permit_cost.cpu_units, 13);
    assert_eq!(policy.permit_cost.memory_units, 14);
    assert_eq!(
        policy.fairness,
        FairnessPolicy::DeficitRoundRobin { quantum: 15 }
    );
    assert_eq!(policy.overflow_policy, OverflowPolicy::QueueWithinDepth);
    assert_eq!(policy.retry_after_policy, RetryAfterPolicy::FixedMs(16));
    assert_eq!(policy.memory_permit_mode, MemoryPermitMode::Hybrid);
    assert_eq!(
        policy.memory_release_policy,
        MemoryReleasePolicy::OnStageBoundary
    );
    assert_eq!(
        policy.memory_overcommit_policy,
        MemoryOvercommitPolicy::DegradeToLight {
            fallback_class: TaskClass::new("fallback"),
        }
    );
    assert_eq!(
        policy.cancellation_policy,
        CancellationPolicy::CooperativeWithDeadline
    );
    assert_eq!(policy.checkpoint_policy, checkpoint);
    assert!(policy.best_effort);
}

#[test]
fn resource_budget_builder_preserves_every_explicit_field() {
    let budget = ResourceBudget::new()
        .cpu_units(21)
        .memory_units(22)
        .per_request_cpu_units(23)
        .per_request_memory_units(24)
        .memory_unit_scale(4096);

    assert_eq!(budget.max_cpu_units, 21);
    assert_eq!(budget.max_memory_units, 22);
    assert_eq!(budget.per_request_max_cpu_units, 23);
    assert_eq!(budget.per_request_max_memory_units, 24);
    assert_eq!(budget.memory_unit_scale.bytes_per_unit, 4096);
}

#[test]
fn queueability_is_exactly_queue_within_depth() {
    assert!(!OverflowPolicy::Reject.is_queueable());
    assert!(OverflowPolicy::QueueWithinDepth.is_queueable());
    assert!(!OverflowPolicy::DropBestEffort.is_queueable());
}

#[test]
fn manual_clock_set_advance_and_read_are_observable() {
    let clock = ManualClock::new(41);
    assert_eq!(clock.now_ms(), 41);
    clock.advance(1);
    assert_eq!(clock.now_ms(), 42);
    clock.set(99);
    assert_eq!(clock.now_ms(), 99);
}

#[test]
fn execution_phase_rank_and_started_boundary_are_exact() {
    let cases = [
        (ExecutionPhase::DispatchReserved, 0, false),
        (ExecutionPhase::Accepted, 1, false),
        (ExecutionPhase::Running, 2, true),
        (ExecutionPhase::CleanupPending, 3, true),
    ];
    for (phase, rank, started) in cases {
        assert_eq!(phase.rank(), rank);
        assert_eq!(phase.has_started(), started);
    }
}

#[test]
fn plan_source_display_and_stack_request_preserve_values() {
    let source = PlanSource::new("adapter.v2").expect("valid source");
    assert_eq!(source.to_string(), "adapter.v2");

    let ordinary = TaskSpec::io(TaskClass::new("c"));
    assert_eq!(ordinary.requested_stack_size_bytes(), None);
    let stacked = ordinary.stack_size_bytes(8 * 1024 * 1024);
    assert_eq!(stacked.requested_stack_size_bytes(), Some(8 * 1024 * 1024));
}

#[test]
fn substrate_hint_mapping_is_total_and_exact() {
    let cases = [
        (SubstrateHint::AsyncIo, None),
        (SubstrateHint::BlockingPool, Some("blocking")),
        (SubstrateHint::SharedCpuExecutor, Some("cpu")),
        (SubstrateHint::LargeStackCapability, Some("large_stack")),
        (SubstrateHint::LocalRuntime, Some("local_runtime")),
        (SubstrateHint::BackgroundOnly, Some("maintenance")),
    ];
    for (hint, pool) in cases {
        assert_eq!(hint.capability_pool(), pool);
    }
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
fn the_system_clock_reads_the_wall_clock_in_milliseconds() {
    // `SystemClock` is the one production adapter of the `Clock` port. Its
    // reading is unix-epoch milliseconds: bracketed by two wall-clock samples
    // taken around it, and within one second of either. A clock that reported
    // seconds, or an unrelated origin, would put every lease timestamp — and
    // therefore every leak-sweep decision — off by orders of magnitude.
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let unix_ms = |at: SystemTime| {
        u64::try_from(
            at.duration_since(UNIX_EPOCH)
                .expect("the test runs after 1970")
                .as_millis(),
        )
        .expect("fits u64 for a very long time")
    };
    // nosemgrep: taskmesh-test-uninjected-system-time -- reason: the adapter under test *is* the wall clock; the assertion brackets its reading between two direct samples.
    let before = unix_ms(SystemTime::now());
    let observed = SystemClock.now_ms();
    // nosemgrep: taskmesh-test-uninjected-system-time -- reason: same bracket, closing sample.
    let after = unix_ms(SystemTime::now());
    assert!(
        before <= observed && observed <= after,
        "SystemClock::now_ms ({observed}) must lie between the wall-clock samples around it ({before}..={after})"
    );
    let second = u64::try_from(Duration::from_secs(1).as_millis()).expect("small");
    assert!(
        after - before < second,
        "the bracket itself is far narrower than a second, so the reading is within one second of SystemTime::now()"
    );
}

#[test]
fn memory_unit_scale_rounds_up() {
    let scale = MemoryUnitScale {
        bytes_per_unit: 4096,
    };
    assert_eq!(scale.units_for(0), Ok(0));
    assert_eq!(scale.units_for(1), Ok(1));
    assert_eq!(scale.units_for(4096), Ok(1));
    assert_eq!(scale.units_for(4097), Ok(2));
}

#[test]
fn memory_unit_scale_reports_unconvertible_readings() {
    use taskmesh_contract::ResourceConversionError;

    // Saturating either of these would report *less* memory than is really
    // held, which is how an over-budget reservation passes a capacity check.
    let unscaled = MemoryUnitScale { bytes_per_unit: 0 };
    assert_eq!(
        unscaled.units_for(4096),
        Err(ResourceConversionError::UnscaledMemoryUnits)
    );

    let fine_grained = MemoryUnitScale { bytes_per_unit: 1 };
    assert_eq!(
        fine_grained.units_for(u64::from(u32::MAX) + 1),
        Err(ResourceConversionError::MemoryUnitsOverflow {
            bytes: u64::from(u32::MAX) + 1,
            bytes_per_unit: 1,
        })
    );
    // The boundary itself still converts.
    assert_eq!(fine_grained.units_for(u64::from(u32::MAX)), Ok(u32::MAX));
}
