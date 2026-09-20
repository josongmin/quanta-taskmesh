//! Arbitrary configurations through the fail-closed front doors.
//!
//! `Governor::validate_policy`, `TopologyConfig::validate` and
//! `Builder::build` are the places a consumer's configuration is judged. Each
//! must answer with a typed refusal or a usable runtime — never a panic, never
//! an `Ok` whose runtime disagrees with the configuration it accepted:
//!
//! * a refused policy is refused with `PolicyViolation` / `InvalidTopology`
//!   whose message renders;
//! * an accepted policy builds a governor whose snapshot lists exactly the
//!   configured classes and every registered capability pool with the
//!   configured limit;
//! * `Builder::build` agrees with `validate_policy` + `TopologyConfig::validate`
//!   on what is acceptable (it must not admit a policy the validator refuses),
//!   and a built runtime's `executor_capabilities()` and `snapshot()` are
//!   readable at once (nothing is spawned by `build`).
#![no_main]

mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use taskmesh::{Builder, GovernorError, Runtime as _};
use taskmesh_contract::{
    CancellationPolicy, ClassPolicy, FairnessPolicy, ManualClock, MemoryOvercommitPolicy,
    MemoryPermitMode, MemoryReleasePolicy, OverflowPolicy, ResourceBudget, RetryAfterPolicy,
    TaskClass, TopologyConfig,
};
use taskmesh_engine::{Governor, PolicySet};

#[derive(Arbitrary, Debug)]
struct ClassShape {
    name: u8,
    max_inflight: u8,
    max_queue_depth: u8,
    cpu_units: u8,
    memory_units: u8,
    fairness: u8,
    fairness_arg: u32,
    overflow: u8,
    overcommit: u8,
    fallback: u8,
    release: u8,
    permit_mode: u8,
    cancellation: u8,
    retry_after: u8,
    retry_after_ms: u16,
    best_effort: bool,
}

#[derive(Arbitrary, Debug)]
struct Config {
    classes: Vec<ClassShape>,
    budget_cpu: u32,
    budget_memory: u32,
    per_request_cpu: u8,
    per_request_memory: u8,
    bytes_per_unit: u16,
    limits: Vec<(u8, u8)>,
    cpu_mode: u8,
    cpu_arg: u16,
    reserve_cores: u8,
    min_workers: u16,
    max_workers: u16,
    blocking_threads: u16,
    large_stack_slots: u16,
    maintenance_workers: u16,
    local_runtime_slots: u16,
}

const NAMES: [&str; 5] = ["a", "b", "c", "light", "heavy"];
const POOLS: [&str; 5] = ["blocking", "cpu", "large_stack", "local_runtime", "nowhere"];

fn class_name(selector: u8) -> TaskClass {
    TaskClass::new(NAMES[usize::from(selector) % NAMES.len()])
}

fn class_policy(shape: &ClassShape) -> ClassPolicy {
    let fairness = match shape.fairness % 5 {
        0 => FairnessPolicy::Fifo,
        1 => FairnessPolicy::DeficitRoundRobin {
            quantum: shape.fairness_arg,
        },
        2 => FairnessPolicy::WeightedFairQueue {
            weight: shape.fairness_arg,
            burst: shape.fairness_arg / 3,
        },
        3 => FairnessPolicy::DeadlineAware {
            slack_ms: u64::from(shape.fairness_arg),
        },
        _ => FairnessPolicy::BestEffortScavenger,
    };
    let overflow = match shape.overflow % 3 {
        0 => OverflowPolicy::Reject,
        1 => OverflowPolicy::QueueWithinDepth,
        _ => OverflowPolicy::DropBestEffort,
    };
    let overcommit = match shape.overcommit % 3 {
        0 => MemoryOvercommitPolicy::Reject,
        1 => MemoryOvercommitPolicy::Queue,
        _ => MemoryOvercommitPolicy::DegradeToLight {
            fallback_class: class_name(shape.fallback),
        },
    };
    let release = match shape.release % 3 {
        0 => MemoryReleasePolicy::OnTaskCompletion,
        1 => MemoryReleasePolicy::OnStageBoundary,
        _ => MemoryReleasePolicy::LeakDetecting,
    };
    let permit_mode = match shape.permit_mode % 3 {
        0 => MemoryPermitMode::Estimated,
        1 => MemoryPermitMode::Measured,
        _ => MemoryPermitMode::Hybrid,
    };
    let cancellation = match shape.cancellation % 3 {
        0 => CancellationPolicy::PreSubmitOnly,
        1 => CancellationPolicy::Cooperative,
        _ => CancellationPolicy::CooperativeWithDeadline,
    };
    let retry_after = match shape.retry_after % 3 {
        0 => RetryAfterPolicy::None,
        1 => RetryAfterPolicy::FixedMs(u64::from(shape.retry_after_ms)),
        _ => RetryAfterPolicy::Adaptive,
    };
    ClassPolicy::new()
        .max_inflight(u32::from(shape.max_inflight))
        .max_queue_depth(u32::from(shape.max_queue_depth))
        .cpu_units(u32::from(shape.cpu_units))
        .memory_units(u32::from(shape.memory_units))
        .fairness(fairness)
        .overflow_policy(overflow)
        .memory_overcommit_policy(overcommit)
        .memory_release_policy(release)
        .memory_permit_mode(permit_mode)
        .cancellation_policy(cancellation)
        .retry_after_policy(retry_after)
        .best_effort(shape.best_effort)
}

fn topology(config: &Config) -> TopologyConfig {
    let base = TopologyConfig::new();
    let base = match config.cpu_mode % 3 {
        0 => base.cpu_auto(),
        1 => base.cpu_fixed(usize::from(config.cpu_arg)),
        _ => base
            .reserve_cores(usize::from(config.reserve_cores))
            .min_workers(usize::from(config.min_workers))
            .max_workers(usize::from(config.max_workers)),
    };
    base.blocking_threads(usize::from(config.blocking_threads))
        .large_stack_slots(usize::from(config.large_stack_slots))
        .maintenance_workers(usize::from(config.maintenance_workers))
        .local_runtime_slots(usize::from(config.local_runtime_slots))
}

fuzz_target!(|config: Config| {
    support::checkpoint("policy_topology", "target_entry");
    let resources = ResourceBudget::new()
        .cpu_units(config.budget_cpu)
        .memory_units(config.budget_memory)
        .per_request_cpu_units(u32::from(config.per_request_cpu))
        .per_request_memory_units(u32::from(config.per_request_memory))
        .memory_unit_scale(u64::from(config.bytes_per_unit));
    let mut classes = BTreeMap::new();
    for shape in config.classes.iter().take(8) {
        classes.insert(class_name(shape.name), class_policy(shape));
    }
    let mut limits = BTreeMap::new();
    for (pool, limit) in config.limits.iter().take(6) {
        limits.insert(
            POOLS[usize::from(*pool) % POOLS.len()].to_string(),
            u32::from(*limit),
        );
    }

    // The engine's policy front door: a limit for a pool no substrate provides
    // is refused by name; everything else is judged by `validate_policy`.
    let policy = match PolicySet::new(resources.clone(), classes.clone())
        .with_capability_limits(limits.clone())
    {
        Ok(policy) => Some(policy),
        Err(error) => {
            assert!(
                limits.contains_key("nowhere"),
                "capability limits refused without an unregistered pool: {error}"
            );
            assert!(error.to_string().contains("nowhere"));
            None
        }
    };
    if let Some(policy) = policy {
        match Governor::validate_policy(&policy) {
            Ok(()) => {
                let governor = Governor::new(policy, Arc::new(ManualClock::new(0)))
                    .expect("validated policy builds");
                support::checkpoint("policy_topology", "policy_valid");
                let snapshot = governor.snapshot();
                assert_eq!(
                    snapshot.classes.keys().cloned().collect::<Vec<_>>(),
                    classes.keys().cloned().collect::<Vec<_>>(),
                    "the snapshot must list exactly the configured classes"
                );
                for (pool, limit) in &limits {
                    assert_eq!(
                        snapshot.capabilities[pool].limit, *limit,
                        "pool {pool}: the configured limit is the published limit"
                    );
                }
                assert_eq!(snapshot.conservation_violation(), None);
            }
            Err(error) => {
                assert!(
                    matches!(error, GovernorError::PolicyViolation(_)),
                    "validate_policy must refuse with PolicyViolation, got {error:?}"
                );
                let rendered = error.to_string();
                assert!(!rendered.is_empty());
                // A refused policy must not build either.
                assert!(
                    Governor::new(policy, Arc::new(ManualClock::new(0))).is_err(),
                    "Governor::new accepted a policy validate_policy refused"
                );
            }
        }
    }

    // The host front door: topology validation, then the builder, which must
    // agree with the validators and spawn nothing.
    let topology = topology(&config);
    let topology_verdict = topology.validate();
    let mut builder = Builder::new()
        .topology(topology.clone())
        .resources(resources);
    for (class, policy) in &classes {
        builder = builder.class_policy(class.clone(), policy.clone());
    }
    match builder.build() {
        Ok(runtime) => {
            support::checkpoint("policy_topology", "topology_valid");
            assert!(
                topology_verdict.is_ok(),
                "build accepted a topology validate() refuses: {topology_verdict:?}"
            );
            let snapshot = runtime.snapshot();
            assert_eq!(
                snapshot.classes.keys().cloned().collect::<Vec<_>>(),
                classes.keys().cloned().collect::<Vec<_>>()
            );
            assert_eq!(snapshot.conservation_violation(), None);
            let _ = runtime.executor_capabilities();
            assert!(!runtime.is_draining());
        }
        Err(error) => {
            assert!(
                matches!(
                    error,
                    GovernorError::PolicyViolation(_) | GovernorError::InvalidTopology(_)
                ),
                "build must refuse with a configuration error, got {error:?}"
            );
            let _ = error.to_string();
        }
    }
});
