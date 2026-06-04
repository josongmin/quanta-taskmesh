//! Instruction-count gate for the governance hot paths (ADR 9000 / P2).
//!
//! Wall-clock benchmarks (`admit_release.rs`) measure reality but vary by
//! machine. This bench runs the same hot paths under Cachegrind/Callgrind via
//! `iai-callgrind`, yielding *deterministic, machine-independent* instruction,
//! L1/LL cache, and RAM-hit counts — the load-bearing CI regression gate.
//!
//! Requires valgrind and the matching `iai-callgrind-runner`; built only with
//! `--features iai` and run on the Linux CI runner. Setup is excluded from the
//! measured counts via the `setup =` hook, so each count is one governance op.

use std::collections::BTreeMap;
use std::sync::Arc;

use iai_callgrind::{black_box, library_benchmark, library_benchmark_group, main};
use taskmesh_contract::{ClassPolicy, ManualClock, ResourceBudget, TaskClass, TaskSpec};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet};

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
        let _ = governor.admit(&spec);
    }
    governor
}

// One admit→release cycle — the per-task governance cost.
#[library_benchmark]
#[bench::roundtrip(setup = setup_roundtrip)]
fn admit_release(input: (Governor, TaskSpec)) {
    let (governor, spec) = input;
    if let AdmissionDecision::Admitted { permit_id } = governor.admit(black_box(&spec)) {
        governor.release(black_box(permit_id));
    }
}

// The fail-closed reject path for an unknown class.
#[library_benchmark]
#[bench::reject(setup = setup_reject)]
fn admit_unknown_reject(input: (Governor, TaskSpec)) {
    let (governor, spec) = input;
    black_box(governor.admit(black_box(&spec)));
}

// Snapshot over an 8-class governor with outstanding permits.
#[library_benchmark]
#[bench::classes8(setup = setup_snapshot)]
fn snapshot(governor: Governor) {
    black_box(governor.snapshot());
}

library_benchmark_group!(
    name = governance;
    benchmarks = admit_release, admit_unknown_reject, snapshot
);

main!(library_benchmark_groups = governance);
