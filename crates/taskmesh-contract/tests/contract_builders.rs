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
        .child_of("fetch:doc:42", TaskStage::new("fanout"))
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
        .child_of("fetch:doc:42", TaskStage::new("fanout"));
    assert_eq!(child2.root_operation_id, "fetch:doc:42");

    // A plain root is still keyed by its own operation name.
    let root = TaskSpec::io(TaskClass::new("c")).operation("op-1");
    assert_eq!(root.root_operation_id, "op-1");
}

#[test]
fn child_of_inherits_root_id_and_sets_scope() {
    let child = TaskSpec::cpu(TaskClass::new("c")).child_of("root-7", TaskStage::new("fanout"));
    assert_eq!(child.root_operation_id, "root-7");
    assert!(matches!(child.scope, TaskScope::Child { .. }));
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
