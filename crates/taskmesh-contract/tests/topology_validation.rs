//! TM16-003 regression: topology is public, deserializable input, so every
//! impossible shape is a typed rejection — never a panic inside a resolver.
//!
//! The original defect was a `clamp(min, max)` on caller-supplied bounds:
//! `clamp` panics when `min > max`, and both the default executor and the Rayon
//! adapter called it. A fallible `Builder::build` that panics is not fallible.

use taskmesh_contract::{CpuMode, TopologyConfig, TopologyError, MAX_CAPABILITY_SLOTS};

#[test]
fn inverted_worker_bounds_are_reported_not_panicked() {
    let topology = TopologyConfig::new().min_workers(8).max_workers(2);
    assert_eq!(
        topology.validate(),
        Err(TopologyError::InvertedCpuWorkerBounds {
            min_workers: 8,
            max_workers: 2,
        })
    );
    assert_eq!(
        topology.try_resolved_cpu_workers(4),
        Err(TopologyError::InvertedCpuWorkerBounds {
            min_workers: 8,
            max_workers: 2,
        })
    );
}

#[test]
fn the_total_resolver_never_panics_on_an_inverted_window() {
    // `resolved_cpu_workers` is total by construction, so even a caller that
    // skips validation gets a number rather than an unwind. Validation is what
    // tells them the window was wrong; totality is what stops the panic.
    let topology = TopologyConfig::new().min_workers(8).max_workers(2);
    for available in [0, 1, 4, 64, usize::MAX] {
        let resolved = topology.resolved_cpu_workers(available);
        assert!(resolved >= 1, "resolved workers must stay positive");
    }
}

#[test]
fn zero_fixed_cpu_pool_is_rejected() {
    // `Fixed(0)` asks for a pool that can never run anything. Silently clamping
    // it to one worker would grant capacity the operator did not ask for.
    let topology = TopologyConfig::new().cpu_fixed(0);
    assert_eq!(topology.validate(), Err(TopologyError::ZeroFixedCpuWorkers));
}

#[test]
fn slot_counts_beyond_the_governed_maximum_are_rejected() {
    for (name, topology) in [
        (
            "blocking",
            TopologyConfig::new().blocking_threads(MAX_CAPABILITY_SLOTS + 1),
        ),
        (
            "large_stack",
            TopologyConfig::new().large_stack_slots(MAX_CAPABILITY_SLOTS + 1),
        ),
        (
            "maintenance",
            TopologyConfig::new().maintenance_workers(MAX_CAPABILITY_SLOTS + 1),
        ),
        (
            "local_runtime",
            TopologyConfig::new().local_runtime_slots(MAX_CAPABILITY_SLOTS + 1),
        ),
    ] {
        assert_eq!(
            topology.validate(),
            Err(TopologyError::SlotCountTooLarge {
                pool: name,
                slots: MAX_CAPABILITY_SLOTS + 1,
                max: MAX_CAPABILITY_SLOTS,
            }),
            "{name} must reject an unrepresentable slot count"
        );
    }
    // The maximum itself is accepted: the boundary is inclusive.
    assert_eq!(
        TopologyConfig::new()
            .blocking_threads(MAX_CAPABILITY_SLOTS)
            .validate(),
        Ok(())
    );
}

#[test]
fn valid_topologies_resolve_as_documented() {
    // Auto minus reserved cores, clamped into the window.
    let auto = TopologyConfig::new().cpu_auto().reserve_cores(2);
    assert_eq!(auto.try_resolved_cpu_workers(8), Ok(6));
    // Reserving more than exists floors at the minimum rather than underflowing.
    assert_eq!(auto.try_resolved_cpu_workers(1), Ok(1));

    let fixed = TopologyConfig::new().cpu_fixed(16).max_workers(4);
    assert_eq!(fixed.try_resolved_cpu_workers(64), Ok(4));

    let floored = TopologyConfig::new().cpu_fixed(1).min_workers(3);
    assert_eq!(floored.try_resolved_cpu_workers(64), Ok(3));

    // `0` slot counts keep their published "no limit" meaning and stay valid.
    assert_eq!(TopologyConfig::default().validate(), Ok(()));
    assert_eq!(TopologyConfig::default().cpu.mode, CpuMode::Auto);
}

#[test]
fn equal_worker_bounds_are_a_valid_window() {
    // The clamp window is inclusive on both ends: `min == max` is a window of
    // exactly one value, not an inverted one. It pins the pool to that size
    // whatever the machine offers.
    let pinned = TopologyConfig::new().min_workers(4).max_workers(4);
    assert_eq!(pinned.validate(), Ok(()));
    assert_eq!(pinned.try_resolved_cpu_workers(64), Ok(4));
    assert_eq!(pinned.try_resolved_cpu_workers(1), Ok(4));
}

#[test]
fn cpu_mode_setters_touch_only_the_cpu_mode() {
    // `cpu_auto` / `cpu_fixed` switch the mode and nothing else: a blocking
    // pool sized earlier in the builder chain is kept, and switching back to
    // `Auto` after `Fixed` really is `Auto`.
    let topology = TopologyConfig::new().blocking_threads(2).cpu_auto();
    assert_eq!(topology.blocking_threads, 2);
    assert_eq!(topology.cpu.mode, CpuMode::Auto);
    let switched = TopologyConfig::new()
        .blocking_threads(2)
        .cpu_fixed(3)
        .cpu_auto();
    assert_eq!(switched.cpu.mode, CpuMode::Auto);
    assert_eq!(switched.blocking_threads, 2);
    assert_eq!(
        TopologyConfig::new().cpu_auto().cpu_fixed(3).cpu.mode,
        CpuMode::Fixed(3)
    );
}
