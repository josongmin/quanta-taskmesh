//! Phase-0 hot-path benchmark (ADR 9000 / P2 wall-clock track): the governance
//! tax of one admit→release cycle on the pure engine, with zero runtime noise.

use std::collections::BTreeMap;
use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use taskmesh_contract::{ClassPolicy, ManualClock, ResourceBudget, TaskClass, TaskSpec};
use taskmesh_engine::{AdmissionDecision, Governor, PolicySet, RequestKey};

fn build_governor() -> Governor {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("retrieval"),
        ClassPolicy::new()
            .max_inflight(1_000_000)
            .cpu_units(1)
            .memory_units(1),
    );
    let resources = ResourceBudget::new().cpu_units(0).memory_units(0);
    // Counter clock (not SystemClock): the doc promises "zero runtime noise", so
    // admit/release must not pay a per-call SystemTime syscall.
    Governor::new_unchecked(
        PolicySet::new(resources, classes),
        Arc::new(ManualClock::new(0)),
    )
}

fn bench_admit_release(c: &mut Criterion) {
    let governor = build_governor();
    let spec = TaskSpec::blocking(TaskClass::new("retrieval")).operation("search:repo:1");

    c.bench_function("admit_release_success", |b| {
        b.iter(|| {
            let permit = match governor.admit(&spec, RequestKey::new("search:repo:1")) {
                AdmissionDecision::Admitted { permit_id } => permit_id,
                other => panic!("expected admit, got {other:?}"),
            };
            governor.release(permit);
        });
    });

    c.bench_function("admit_unknown_class_reject", |b| {
        let ghost = TaskSpec::blocking(TaskClass::new("ghost")).operation("x");
        // One-time correctness guard that runs even in --release (criterion's
        // profile), so a fixture regression making `ghost` resolve to an admit
        // can't silently turn this into an admit benchmark. The reject path
        // mutates no state, so this does not perturb the timed loop below.
        assert!(
            matches!(
                governor.admit(&ghost, RequestKey::new("x")),
                AdmissionDecision::Rejected(_)
            ),
            "unknown class must reject"
        );
        b.iter(|| {
            // black_box the verdict so the dead return cannot be elided.
            black_box(governor.admit(&ghost, RequestKey::new("x")));
        });
    });
}

criterion_group!(benches, bench_admit_release);
criterion_main!(benches);
