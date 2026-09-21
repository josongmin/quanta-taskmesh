//! Composite admission + deterministic-reduce hot paths (ADR 9000, T06 slice).
//! Times child→root attribution and reduce validation; both must be allocation-
//! light and order-independent.

use std::collections::BTreeMap;
use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use taskmesh_contract::{
    ClassPolicy, DeterministicReducePolicy, ManualClock, ResourceBudget, SubstrateHint, TaskClass,
    TaskSpec, TaskStage,
};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet, ReleaseOutcome};

fn governor() -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("worker"),
        ClassPolicy::new()
            .max_inflight(1_000_000)
            .cpu_units(1)
            .memory_units(1),
    );
    Governor::new_unchecked(
        PolicySet::new(ResourceBudget::new(), classes),
        Arc::new(ManualClock::new(0)),
    )
}

fn bench(c: &mut Criterion) {
    // Child admit -> root attribution -> release, distinct stage per iteration to
    // avoid the recursion guard, measuring composite accounting cost.
    let g = governor();
    let mut n = 0u64;
    c.bench_function("composite_child_admit_release", |b| {
        b.iter(|| {
            n += 1;
            let stage = TaskStage::new(format!("s{}", n % 4096));
            let child =
                TaskSpec::blocking(TaskClass::new("worker")).child_of("root", "root", stage);
            // The op is admit→release. A queued or rejected child is a different
            // operation and must not be timed as a cheaper cycle.
            match g.admit(&child) {
                AdmissionDecision::Admitted { permit_id } => {
                    assert_eq!(g.release(permit_id), ReleaseOutcome::Released);
                }
                other => panic!("composite benchmark requires Admitted, got {other:?}"),
            }
        });
    });

    // Deterministic reduce validation of a fan-out spec.
    let spec = TaskSpec::cpu(TaskClass::new("worker")).reduce_stage(
        TaskStage::new("merge"),
        SubstrateHint::SharedCpuExecutor,
        DeterministicReducePolicy::keyed("doc_id"),
    );
    c.bench_function("reduce_policy_validation", |b| {
        // black_box the input each iteration so the loop-invariant validation of
        // an immutable spec cannot be hoisted out and measured as ~nothing. The
        // spec is valid by construction; a validation that starts failing is a
        // behavioral regression, not a faster benchmark.
        b.iter(|| {
            let verdict = Governor::validate_reduce(black_box(&spec));
            assert!(verdict.is_ok(), "fixture spec must validate: {verdict:?}");
            black_box(verdict.is_ok())
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
