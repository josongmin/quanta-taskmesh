//! Named Governor queue-promotion cost (B01). Fixture construction and queue
//! population are outside the timed span. One timed release must promote exactly
//! one queued request; Criterion's denominator is that completed promotion.

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

criterion_group!(benches, queue_scaling);
criterion_main!(benches);
