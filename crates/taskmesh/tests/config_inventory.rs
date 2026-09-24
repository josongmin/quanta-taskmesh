//! Regression: `TokioRuntime::config()` preserves the portable declaration and
//! carries the resolved substrate inventory (built-ins plus deployment
//! additions). Machine-resolved limits remain owned by the governor snapshot.

use taskmesh::ext::*;
use taskmesh::*;

fn names(records: &[SubstrateRecord]) -> Vec<String> {
    records.iter().map(|s| s.name.to_string()).collect()
}

#[test]
fn builder_rejects_malformed_class_policy_before_admission() {
    let Err(GovernorError::PolicyViolation(message)) = Builder::new()
        .class_policy(TaskClass::new("bad class"), ClassPolicy::new())
        .build()
    else {
        panic!("policy key rejected at build");
    };
    assert!(message.contains("invalid policy class"), "{message}");
}

#[test]
fn builder_rejects_duplicate_class_policy_even_when_values_match() {
    let Err(GovernorError::PolicyViolation(message)) = Builder::new()
        .class_policy(TaskClass::new("c"), ClassPolicy::new().max_inflight(1))
        .class_policy(TaskClass::new("c"), ClassPolicy::new().max_inflight(1))
        .build()
    else {
        panic!("duplicate class registration rejected at build");
    };
    assert!(
        message.contains("duplicate class policy registration: c"),
        "{message}"
    );
}

#[test]
fn duplicate_custom_capability_cannot_change_finite_limit_to_unbounded() {
    let Err(GovernorError::PolicyViolation(message)) = Builder::new()
        .substrate(SubstrateRecord::new(
            "external-gpu",
            SubstrateKind::CompetingExecution,
            Some("external-gpu"),
        ))
        .capability_limit("external-gpu", 7)
        .capability_limit("external-gpu", 0)
        .build()
    else {
        panic!("duplicate capability registration rejected at build");
    };
    assert!(
        message.contains("duplicate capability limit registration: external-gpu"),
        "{message}"
    );
}

#[test]
fn builder_owned_capability_pools_cannot_be_overridden() {
    for pool in [
        "cpu",
        "blocking",
        "large_stack",
        "maintenance",
        "local_runtime",
        "physical.cpu",
        "physical.shared_blocking",
        "physical.dedicated",
    ] {
        let Err(GovernorError::PolicyViolation(message)) =
            Builder::new().capability_limit(pool, 1).build()
        else {
            panic!("topology-owned pool {pool} must reject caller capacity");
        };
        assert!(
            message.contains(&format!(
                "capability limit for built-in pool {pool} cannot be overridden"
            )),
            "{pool}: {message}"
        );
    }
}

#[test]
fn an_unbounded_sentinel_cannot_override_a_builder_owned_pool() {
    let Err(GovernorError::PolicyViolation(message)) =
        Builder::new().capability_limit("blocking", 0).build()
    else {
        panic!("the unbounded sentinel must not bypass builder ownership");
    };
    assert!(
        message.contains("capability limit for built-in pool blocking cannot be overridden"),
        "{message}"
    );
}

#[test]
fn config_carries_resolved_substrate_inventory() {
    let extra = SubstrateRecord::new(
        "external-gpu",
        SubstrateKind::CompetingExecution,
        Some("external-gpu"),
    );
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(TaskClass::new("c"), ClassPolicy::new().cpu_units(1))
        .substrate(extra)
        .capability_limit("external-gpu", 0)
        .build()
        .unwrap();

    let config_names = names(&rt.config().substrates);

    // Every built-in is present in the config inventory.
    for builtin in BUILTIN_SUBSTRATES {
        assert!(
            config_names.contains(&(*builtin).to_string()),
            "config inventory missing built-in {builtin}"
        );
    }
    // The deployment addition is present too.
    assert!(config_names.contains(&"external-gpu".to_string()));

    // And the config inventory matches the governor snapshot exactly (same
    // authority, two views): config() is not lossy.
    let snapshot_names = names(&rt.governor().snapshot().substrates);
    assert_eq!(
        config_names, snapshot_names,
        "config inventory must mirror the governor snapshot inventory"
    );
}

#[test]
fn the_auto_cpu_gate_is_the_detected_parallelism() {
    // Under `cpu_auto` with no reservation and an open clamp window, the `cpu`
    // capability gate is sized from the one parallelism reading the builder
    // takes — the same number the default executor is built from. A gate that
    // disagreed with the pool would admit work the pool cannot run at once.
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(TaskClass::new("c"), ClassPolicy::new().cpu_units(1))
        .topology(TopologyConfig::new().cpu_auto())
        .build()
        .expect("an auto topology builds");
    let detected = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let gate = rt.governor().snapshot().capabilities["cpu"].limit;
    assert_eq!(
        usize::try_from(gate).expect("a u32 gate fits usize"),
        detected,
        "the cpu gate under cpu_auto must equal the detected parallelism"
    );
    assert_eq!(rt.config().topology.cpu.mode, CpuMode::Auto);
}

#[test]
fn custom_limits_have_one_resolved_runtime_authority() {
    let rt = Builder::new()
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(TaskClass::new("c"), ClassPolicy::new().cpu_units(1))
        .substrate(SubstrateRecord::new(
            "external-gpu",
            SubstrateKind::CompetingExecution,
            Some("external-gpu"),
        ))
        .capability_limit("external-gpu", 7)
        .build()
        .expect("a governed external pool builds");

    assert_eq!(
        rt.governor().snapshot().capabilities["external-gpu"].limit,
        7,
        "resolved limits are runtime state owned by the governor snapshot"
    );
    assert_eq!(rt.config().topology.cpu.mode, CpuMode::Auto);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn a_fixed_cpu_pool_beyond_the_slot_domain_is_rejected_as_the_cpu_pool() {
    // The `cpu` pool is not among the topology's declared slots (it is resolved
    // from parallelism), so the contract-level validation cannot see it. The
    // builder still has to refuse a fixed count that does not fit the `u32`
    // gate — and name the pool it refused, so the error is not mistaken for a
    // blocking or large-stack misconfiguration.
    let too_many = usize::try_from(u32::MAX).expect("fits") + 1;
    let Err(error) = Builder::new()
        .resources(ResourceBudget::new().cpu_units(8).memory_units(8))
        .class_policy(TaskClass::new("c"), ClassPolicy::new().cpu_units(1))
        .topology(TopologyConfig::new().cpu_fixed(too_many))
        .build()
    else {
        panic!("a cpu pool of 2^32 workers cannot be gated and must be refused");
    };
    assert_eq!(
        error,
        GovernorError::InvalidTopology(TopologyError::SlotCountTooLarge {
            pool: "cpu",
            slots: too_many,
            max: taskmesh_contract::MAX_CAPABILITY_SLOTS,
        }),
        "the refusal names the cpu pool and the count"
    );
}
