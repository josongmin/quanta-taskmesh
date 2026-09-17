//! Instruction-count gate for the governance hot paths (ADR 9000 / P2).
//!
//! Wall-clock benchmarks (`admit_release.rs`) measure reality but vary by
//! machine. This bench runs the same hot paths under Cachegrind/Callgrind via
//! `iai-callgrind`, yielding *deterministic, machine-independent* instruction,
//! L1/LL cache, and RAM-hit counts — the load-bearing CI regression gate.
//!
//! Requires valgrind and the matching `iai-callgrind-runner`; built only with
//! `--features iai` and run on the Linux CI runner.
//!
//! # Measured region
//!
//! Setup runs in the `setup` hook and is excluded. The benchmark function
//! **returns its input**, so the input's destructor runs in the `teardown` hook
//! — also excluded. Without that, each function dropped the governor (state,
//! policy, registry) on the way out and the "one governance op" figure included
//! the teardown of the fixture that produced it. Snapshot *output* is returned
//! too; whether its drop is measured is therefore explicit, not incidental.
//!
//! # Verdicts are asserted
//!
//! An admission that does not produce the verdict the benchmark was written for
//! is a different operation. It is a failure here, not a cheaper sample.

use std::collections::BTreeMap;
use std::sync::Arc;

use iai_callgrind::{black_box, library_benchmark, library_benchmark_group, main};
use taskmesh_contract::{
    AdmissionVerdict, ClassPolicy, ManualClock, ResourceBudget, Snapshot, TaskClass, TaskSpec,
};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet, ReleaseOutcome};

/// Measurement definition version. Bumped whenever the measured region changes
/// (as it did when teardown moved out of it); baselines are keyed by it.
pub const MEASUREMENT_SCHEMA: u32 = 2;

fn one_class_governor() -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("retrieval"),
        ClassPolicy::new()
            .max_inflight(1_000_000)
            .cpu_units(1)
            .memory_units(1),
    );
    // All-zero budgets disable the cpu/memory ceilings → admit always succeeds.
    Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(0)),
    )
}

fn setup_roundtrip() -> (Governor, TaskSpec) {
    (
        one_class_governor(),
        TaskSpec::blocking(TaskClass::new("retrieval")).operation("op"),
    )
}

fn setup_reject() -> (Governor, TaskSpec) {
    (
        one_class_governor(),
        TaskSpec::blocking(TaskClass::new("ghost")).operation("x"),
    )
}

fn setup_snapshot() -> Governor {
    let mut classes = BTreeMap::new();
    for i in 0..8 {
        classes.insert(
            TaskClass::new(format!("c{i}")),
            ClassPolicy::new()
                .max_inflight(1_000_000)
                .cpu_units(1)
                .memory_units(1),
        );
    }
    let governor = Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(0)),
    );
    for i in 0..8 {
        let spec = TaskSpec::blocking(TaskClass::new(format!("c{i}"))).operation(format!("op{i}"));
        assert!(
            matches!(governor.admit(&spec), AdmissionDecision::Admitted { .. }),
            "snapshot fixture must hold eight live permits"
        );
    }
    governor
}

/// Destroy a benchmark's input outside the measured region.
fn teardown_input<T>(input: T) {
    drop(input);
}

/// Destroy the snapshot output *and* the input governor outside the measured
/// region. The op is "take a snapshot", not "take one and free it".
fn teardown_snapshot(output: (Governor, Snapshot)) {
    drop(output);
}

// One admit→release cycle — the per-task governance cost.
#[library_benchmark]
#[bench::roundtrip(setup = setup_roundtrip, teardown = teardown_input)]
fn admit_release(input: (Governor, TaskSpec)) -> (Governor, TaskSpec) {
    let (governor, spec) = input;
    match governor.admit(black_box(&spec)) {
        AdmissionDecision::Admitted { permit_id } => {
            assert_eq!(
                governor.release(black_box(permit_id)),
                ReleaseOutcome::Released
            );
        }
        other => panic!("roundtrip benchmark requires Admitted, got {other:?}"),
    }
    (governor, spec)
}

// The fail-closed reject path for an unknown class.
#[library_benchmark]
#[bench::reject(setup = setup_reject, teardown = teardown_input)]
fn admit_unknown_reject(input: (Governor, TaskSpec)) -> (Governor, TaskSpec) {
    let (governor, spec) = input;
    match governor.admit(black_box(&spec)) {
        AdmissionDecision::Rejected(AdmissionVerdict::UnknownClass { .. }) => {}
        other => panic!("reject benchmark requires UnknownClass, got {other:?}"),
    }
    (governor, spec)
}

// Snapshot over an 8-class governor with outstanding permits.
#[library_benchmark]
#[bench::classes8(setup = setup_snapshot, teardown = teardown_snapshot)]
fn snapshot(governor: Governor) -> (Governor, Snapshot) {
    let snapshot = black_box(governor.snapshot());
    (governor, snapshot)
}

library_benchmark_group!(
    name = governance;
    benchmarks = admit_release, admit_unknown_reject, snapshot
);

main!(library_benchmark_groups = governance);
