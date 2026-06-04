//! Throughput–latency characterization of a single saturated retrieval class
//! (ADR 9000 / P1). Open-loop, coordinated-omission-corrected.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use taskmesh_bench::loadgen::simulate;
use taskmesh_bench::workload::{
    fixture, generate, mean_interval_ns, retrieval_policy, WorkloadConfig,
};

fn bench(c: &mut Criterion) {
    let cfg = WorkloadConfig {
        classes: vec!["retrieval".into()],
        lambda: 50_000.0,
        count: 5_000,
        ..Default::default()
    };
    let arrivals = generate(&cfg);
    let interval = mean_interval_ns(cfg.lambda);
    // Aggregate capacity = inflight / service_time. With `inflight` slots, a
    // per-task service of `inflight * interval` makes capacity ≈ the offered rate
    // (ρ ≈ 1) — i.e. actually drives the class to its knee, not 1/inflight idle.
    let inflight = 32u32;
    let service_ns = interval * inflight as u64;

    c.bench_function("retrieval_saturation_sim", |b| {
        b.iter(|| {
            let fx = fixture(vec![("retrieval", retrieval_policy(inflight, 128))], 0, 0);
            let (lat, res) = simulate(&fx, &arrivals, service_ns, interval);
            // Invariants: nothing vanishes, and the tail is fully recorded.
            assert!(res.is_conserved(), "conservation violated: {res:?}");
            assert_eq!(lat.dropped(), 0, "latency samples were dropped");
            black_box((lat.p99(), res.started(), res.max_queue_observed));
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
