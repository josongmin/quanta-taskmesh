//! Named Governor queue-promotion cost (B01). Fixture construction and queue
//! population are outside the timed span. One timed release must promote exactly
//! one queued request; Criterion's denominator is that completed promotion.
//! The deep-backlog groups separately time N unique queue admissions and one
//! release that bypasses N-1 capability-blocked requests. Criterion reports
//! one N-admission fill batch or one completed promotion per sample; divide the
//! fill-batch time by N to describe cost per admission.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use taskmesh_contract::{
    ClassPolicy, ManualClock, OverflowPolicy, ResourceBudget, TaskClass, TaskSpec,
};
use taskmesh_engine::{
    AdmissionDecision, ClaimOutcome, Governor, PermitId, PolicySet, ReleaseOutcome, Ticket,
};

struct QueuedFixture {
    governor: Governor,
    holder: PermitId,
    tickets: Vec<Ticket>,
}

fn fixture(class_count: usize, depth: usize) -> QueuedFixture {
    let mut classes = BTreeMap::new();
    classes.insert(
        TaskClass::new("holder"),
        ClassPolicy::new().max_inflight(1).cpu_units(1),
    );
    for index in 0..class_count {
        classes.insert(
            TaskClass::new(format!("q{index}")),
            ClassPolicy::new()
                .max_inflight(1_000_000)
                .max_queue_depth(u32::try_from(depth).expect("bounded depth"))
                .cpu_units(1)
                .overflow_policy(OverflowPolicy::QueueWithinDepth),
        );
    }
    let governor = Governor::new(
        PolicySet::new(ResourceBudget::new().cpu_units(1), classes),
        Arc::new(ManualClock::new(0)),
    )
    .expect("valid queue-scaling policy");
    let holder = match governor
        .admit(&TaskSpec::blocking(TaskClass::new("holder")).operation("queue-scaling-holder"))
    {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("holder must own the CPU unit: {other:?}"),
    };
    let mut tickets = Vec::with_capacity(class_count * depth);
    for index in 0..class_count {
        let class = TaskClass::new(format!("q{index}"));
        for round in 0..depth {
            match governor.admit(
                &TaskSpec::blocking(class.clone()).operation(format!("queued-{index}-{round}")),
            ) {
                AdmissionDecision::Queued { ticket } => tickets.push(ticket),
                other => panic!("request must queue behind holder: {other:?}"),
            }
        }
    }
    let queued: usize = governor
        .snapshot()
        .classes
        .values()
        .map(|class| class.queued as usize)
        .sum();
    assert_eq!(queued, class_count * depth, "fixture queue depth differs");
    QueuedFixture {
        governor,
        holder,
        tickets,
    }
}

fn assert_one_promotion(fixture: QueuedFixture) {
    let mut promoted = 0;
    for ticket in fixture.tickets {
        match fixture.governor.claim(ticket) {
            ClaimOutcome::Ready(_) => promoted += 1,
            ClaimOutcome::Pending => {}
            other => panic!("queued request terminated unexpectedly: {other:?}"),
        }
    }
    assert_eq!(promoted, 1, "one release must promote one queued request");
    assert!(fixture
        .governor
        .snapshot()
        .conservation_violation()
        .is_none());
}

fn queue_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("governor_queue_promotion_one_release");
    for class_count in [1_usize, 8, 32] {
        for depth in [1_usize, 16] {
            group.bench_with_input(
                BenchmarkId::new("classes_depth", format!("{class_count}_{depth}")),
                &(class_count, depth),
                |b, &(classes, depth)| {
                    b.iter_custom(|iters| {
                        let mut measured = Duration::ZERO;
                        for _ in 0..iters {
                            let queued = fixture(classes, depth);
                            let start = Instant::now();
                            assert_eq!(
                                queued.governor.release(queued.holder),
                                ReleaseOutcome::Released
                            );
                            measured += start.elapsed();
                            assert_one_promotion(queued);
                        }
                        measured
                    });
                },
            );
        }
    }
    group.finish();
}

fn deep_policy(depth: usize) -> ClassPolicy {
    ClassPolicy::new()
        .max_inflight(1)
        .max_queue_depth(u32::try_from(depth).expect("queue depth fits u32"))
        .cpu_units(1)
        .overflow_policy(OverflowPolicy::QueueWithinDepth)
}

fn deep_governor(depth: usize, disjoint_capabilities: bool) -> Governor {
    let mut classes = BTreeMap::from([
        (
            TaskClass::new("cpu-holder"),
            ClassPolicy::new().max_inflight(1).cpu_units(1),
        ),
        (TaskClass::new("queued"), deep_policy(depth)),
    ]);
    if disjoint_capabilities {
        classes.insert(
            TaskClass::new("cap-holder"),
            ClassPolicy::new().max_inflight(1),
        );
    }
    let mut policy = PolicySet::new(ResourceBudget::new().cpu_units(1), classes);
    if disjoint_capabilities {
        policy = policy
            .with_capability_limits(BTreeMap::from([("blocking".to_owned(), 1)]))
            .expect("built-in blocking capability exists");
    }
    Governor::new(policy, Arc::new(ManualClock::new(0))).expect("production-valid policy")
}

fn hold_cpu(governor: &Governor) -> PermitId {
    match governor.admit(&TaskSpec::local(TaskClass::new("cpu-holder")).operation("cpu-holder")) {
        AdmissionDecision::Admitted { permit_id } => permit_id,
        other => panic!("CPU holder must admit: {other:?}"),
    }
}

fn queue_fill_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("governor_queue_fill_unique_admissions");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    for depth in [1_000_usize, 10_000] {
        group.bench_with_input(BenchmarkId::new("queued", depth), &depth, |b, &depth| {
            b.iter_custom(|iters| {
                let mut measured = Duration::ZERO;
                for _ in 0..iters {
                    let governor = deep_governor(depth, false);
                    let _holder = hold_cpu(&governor);
                    let specs: Vec<_> = (0..depth)
                        .map(|index| {
                            TaskSpec::blocking(TaskClass::new("queued"))
                                .operation(format!("fill-{index}"))
                        })
                        .collect();
                    let start = Instant::now();
                    for spec in &specs {
                        assert!(matches!(
                            governor.admit(spec),
                            AdmissionDecision::Queued { .. }
                        ));
                    }
                    measured += start.elapsed();
                    let snapshot = governor.snapshot();
                    assert_eq!(
                        snapshot.classes[&TaskClass::new("queued")].queued as usize,
                        depth
                    );
                    assert!(snapshot.conservation_violation().is_none());
                }
                measured
            });
        });
    }
    group.finish();
}

fn disjoint_capability_promotion_cost(c: &mut Criterion) {
    let mut group = c.benchmark_group("governor_queue_disjoint_capability_one_promotion");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    for depth in [1_000_usize, 10_000] {
        group.bench_with_input(BenchmarkId::new("queued", depth), &depth, |b, &depth| {
            b.iter_custom(|iters| {
                let mut measured = Duration::ZERO;
                for _ in 0..iters {
                    let governor = deep_governor(depth, true);
                    let cap_holder = match governor.admit(
                        &TaskSpec::blocking(TaskClass::new("cap-holder")).operation("cap-holder"),
                    ) {
                        AdmissionDecision::Admitted { permit_id } => permit_id,
                        other => panic!("capability holder must admit: {other:?}"),
                    };
                    let cpu_holder = hold_cpu(&governor);
                    for index in 0..(depth - 1) {
                        let spec = TaskSpec::blocking(TaskClass::new("queued"))
                            .operation(format!("blocked-{index}"));
                        assert!(matches!(
                            governor.admit(&spec),
                            AdmissionDecision::Queued { .. }
                        ));
                    }
                    let runnable =
                        TaskSpec::cpu(TaskClass::new("queued")).operation("runnable-cpu");
                    let ticket = match governor.admit(&runnable) {
                        AdmissionDecision::Queued { ticket } => ticket,
                        other => panic!("runnable follower must initially queue: {other:?}"),
                    };
                    let start = Instant::now();
                    assert_eq!(governor.release(cpu_holder), ReleaseOutcome::Released);
                    measured += start.elapsed();
                    assert!(matches!(governor.claim(ticket), ClaimOutcome::Ready(_)));
                    let snapshot = governor.snapshot();
                    assert_eq!(
                        snapshot.classes[&TaskClass::new("queued")].queued as usize,
                        depth - 1
                    );
                    assert!(snapshot.conservation_violation().is_none());
                    assert_eq!(governor.release(cap_holder), ReleaseOutcome::Released);
                }
                measured
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    queue_scaling,
    queue_fill_cost,
    disjoint_capability_promotion_cost
);
criterion_main!(benches);
